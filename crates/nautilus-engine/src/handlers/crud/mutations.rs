use super::common::{
    execute_mutation_result, parse_and_qualify_model_filter, parse_optional_model_filter,
    protocol_filter_body, wrap_count_result, wrap_mutation_result, MutationResultData,
};
use super::read::{build_find_unique_sql, find_rows_by_expr, find_rows_by_filter};
use super::{nested, *};
use nautilus_migrate::DatabaseProvider;

fn row_field_json<'a>(
    data_obj: &'a JsonMap<String, JsonValue>,
    field: &FieldIr,
) -> Option<&'a JsonValue> {
    data_obj
        .get(&field.logical_name)
        .or_else(|| data_obj.get(&field.db_name))
}

fn updated_at_now_value() -> Value {
    Value::DateTime(chrono::Utc::now().naive_utc())
}

#[derive(Clone, Copy)]
enum FieldInputMode {
    Create,
    Update,
}

fn field_input_value(
    state: &EngineState,
    data_obj: &JsonMap<String, JsonValue>,
    field: &FieldIr,
    mode: FieldInputMode,
) -> Result<Option<Value>, ProtocolError> {
    if field.is_updated_at {
        return match row_field_json(data_obj, field) {
            Some(json_val) if !json_val.is_null() => {
                Ok(Some(convert_field_input(state, json_val, field)?))
            }
            _ => Ok(Some(updated_at_now_value())),
        };
    }

    let Some(json_val) = row_field_json(data_obj, field) else {
        return Ok(None);
    };

    if matches!(mode, FieldInputMode::Create)
        && json_val.is_null()
        && matches!(&field.default_value, Some(DefaultValue::Function(_)))
    {
        return Ok(None);
    }

    Ok(Some(convert_field_input(state, json_val, field)?))
}

/// Convert a single field's JSON input into a [`Value`], routing PostgreSQL
/// native composite-type fields through [`json_to_value_composite`] so they bind
/// as a record literal instead of an untyped text/JSON value. On backends that
/// store composites as JSON, the regular [`json_to_value_field`] path is used.
fn convert_field_input(
    state: &EngineState,
    json_val: &JsonValue,
    field: &FieldIr,
) -> Result<Value, ProtocolError> {
    if let ResolvedFieldType::CompositeType { type_name, .. } = &field.field_type {
        if state.uses_native_composite_types() && !json_val.is_null() {
            if let Some(composite) = state.schema.get_composite_type(type_name) {
                return crate::conversion::json_to_value_composite(json_val, composite);
            }
        }
    }
    json_to_value_field(json_val, &field.field_type)
}

fn should_omit_server_default(json_val: &JsonValue, field: &FieldIr) -> bool {
    json_val.is_null() && matches!(&field.default_value, Some(DefaultValue::Function(_)))
}

fn create_many_effective_fields<'a>(
    model: &'a ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Vec<&'a FieldIr> {
    model
        .fields
        .iter()
        .filter(|field| !matches!(field.field_type, ResolvedFieldType::Relation(_)))
        .filter(|field| {
            if field.is_updated_at {
                return true;
            }
            row_field_json(data_obj, field)
                .is_some_and(|json_val| !should_omit_server_default(json_val, field))
        })
        .collect()
}

/// Whether the database supplies this field's value on insert without being
/// told it, in a way the engine can recover afterwards.
fn is_autoincrement(field: &FieldIr) -> bool {
    matches!(
        &field.default_value,
        Some(DefaultValue::Function(function)) if function.name == "autoincrement"
    )
}

fn unresolvable_key(model: &ModelIr, field: &FieldIr) -> ProtocolError {
    ProtocolError::UnsupportedOperation(format!(
        "Returning the written '{}' row needs its '{}' value, and this backend cannot report a generated one; supply it in the data or set returnData to false",
        model.logical_name, field.logical_name
    ))
}

fn primary_key_field<'a>(model: &'a ModelIr, name: &str) -> Result<&'a FieldIr, ProtocolError> {
    model.find_field(name).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Model '{}' names unknown field '{}' in its primary key",
            model.logical_name, name
        ))
    })
}

fn qualified_column(model: &ModelIr, field: &FieldIr) -> Expr {
    Expr::column(format!("{}__{}", model.db_name, field.db_name))
}

/// Predicate identifying the row an `INSERT` just wrote, for a backend that
/// cannot return it inline.
///
/// A key the caller supplied is known outright. The only generated key that can
/// be recovered afterwards is MySQL's single `AUTO_INCREMENT` column, whose
/// value the server reports as `LAST_INSERT_ID()` — on the same connection,
/// which is why the read-back runs inside a transaction.
fn inserted_row_filter(
    state: &EngineState,
    model: &ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<Expr, ProtocolError> {
    let mut conditions: Vec<Expr> = Vec::new();
    let mut generated: Option<&FieldIr> = None;

    for name in model.primary_key.fields() {
        let field = primary_key_field(model, name)?;
        let column = qualified_column(model, field);

        match field_input_value(state, data_obj, field, FieldInputMode::Create)? {
            Some(value) => conditions.push(column.eq(Expr::param(value))),
            None if generated.is_none() => {
                if state.provider() != DatabaseProvider::Mysql || !is_autoincrement(field) {
                    return Err(unresolvable_key(model, field));
                }
                generated = Some(field);
                conditions.push(column.eq(Expr::FunctionCall {
                    name: "LAST_INSERT_ID".to_string(),
                    args: vec![],
                }));
            }
            None => return Err(unresolvable_key(model, field)),
        }
    }

    conditions.into_iter().reduce(Expr::and).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Model '{}' has no primary key to read a written row back by",
            model.logical_name
        ))
    })
}

/// Predicate matching the rows an `UPDATE` touched, keyed on the primary key
/// values captured before it ran.
///
/// An update that assigns a primary-key column moves the row, so the assigned
/// value is what the read-back looks for.
fn updated_rows_filter(
    state: &EngineState,
    model: &ModelIr,
    rows: &[Row],
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<Option<Expr>, ProtocolError> {
    let mut per_row = Vec::with_capacity(rows.len());

    for row in rows {
        let mut conditions: Vec<Expr> = Vec::new();
        for name in model.primary_key.fields() {
            let field = primary_key_field(model, name)?;
            let value = match field_input_value(state, data_obj, field, FieldInputMode::Update)? {
                Some(assigned) => assigned,
                None => row
                    .get(&format!("{}__{}", model.db_name, field.db_name))
                    .cloned()
                    .ok_or_else(|| unresolvable_key(model, field))?,
            };
            conditions.push(qualified_column(model, field).eq(Expr::param(value)));
        }
        if let Some(predicate) = conditions.into_iter().reduce(Expr::and) {
            per_row.push(predicate);
        }
    }

    Ok(per_row.into_iter().reduce(Expr::or))
}

/// Insert one row of `model`, reading it back afterwards on a backend without
/// `RETURNING`.
async fn insert_row(
    state: &EngineState,
    model: &ModelIr,
    data: &JsonValue,
    return_data: bool,
    tx_id: Option<&str>,
) -> Result<MutationResultData, ProtocolError> {
    let metadata = state.model_metadata(model);

    let data_obj = data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;

    let scalar_field_capacity = metadata.scalar_fields().len();
    let mut columns = Vec::with_capacity(scalar_field_capacity);
    let mut values = Vec::with_capacity(scalar_field_capacity);

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(value) = field_input_value(state, data_obj, field, FieldInputMode::Create)? {
            columns.push(field_marker(model, field));
            values.push(value);
        }
    }

    let returns_inline = return_data && state.dialect.supports_returning();

    let mut builder = Insert::into_table(&model.db_name)
        .with_capacity(InsertCapacity {
            columns: columns.len(),
            rows: 1,
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        })
        .columns(columns)
        .values(values);
    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build insert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert_owned(insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if returns_inline || !return_data {
        return execute_mutation_result(
            state,
            &sql,
            "Insert",
            tx_id,
            metadata.scalar_hints(),
            returns_inline,
        )
        .await;
    }

    let filter = inserted_row_filter(state, model, data_obj)?;

    state
        .in_transaction(tx_id, |tx| async move {
            state.execute_affected_on(&sql, "Insert", Some(&tx)).await?;
            let rows = find_rows_by_expr(state, model, Some(filter), Some(1), Some(&tx)).await?;
            Ok(MutationResultData::Rows(rows))
        })
        .await
}

pub(super) async fn execute_create(
    state: &EngineState,
    params: CreateParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let model = get_writable_model_or_error(state, &params.model)?;
    let plan = nested::split(state, model, &params.data, false)?;

    if plan.is_empty() {
        return insert_row(
            state,
            model,
            &params.data,
            params.return_data,
            params.transaction_id.as_deref(),
        )
        .await;
    }

    let return_data = params.return_data;
    let needs_row = return_data || plan.writes_children();

    state
        .in_transaction(params.transaction_id.as_deref(), |tx| async move {
            let (data, deferred) =
                nested::prepare_parent_data(state, model, &plan, None, &tx).await?;
            let result = insert_row(state, model, &data, needs_row, Some(&tx)).await?;

            if plan.writes_children() {
                let parent = result.first_row().ok_or_else(|| {
                    ProtocolError::Internal(
                        "create could not read back the row its nested writes hang from"
                            .to_string(),
                    )
                })?;
                nested::apply_children(state, model, &plan, parent, &tx).await?;
            }
            deferred.run(state, &tx).await?;

            Ok(if return_data {
                result
            } else {
                MutationResultData::Count(1)
            })
        })
        .await
}

async fn execute_create_many(
    state: &EngineState,
    params: CreateManyParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_writable_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);

    if params.data.is_empty() {
        return Err(ProtocolError::InvalidParams(
            "data array cannot be empty".to_string(),
        ));
    }

    let first_obj = params.data[0]
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data items must be objects".to_string()))?;

    let relevant_fields = create_many_effective_fields(model, first_obj);
    let expected_keys: Vec<&str> = relevant_fields
        .iter()
        .map(|field| field.logical_name.as_str())
        .collect();
    let expected_key_set: std::collections::HashSet<&str> = expected_keys.iter().copied().collect();

    let columns: Vec<_> = relevant_fields
        .iter()
        .map(|field| field_marker(model, field))
        .collect();

    let mut all_values = Vec::with_capacity(params.data.len());
    for (row_idx, json_value) in params.data.iter().enumerate() {
        let data_obj = json_value.as_object().ok_or_else(|| {
            ProtocolError::InvalidParams("data items must be objects".to_string())
        })?;

        let row_fields = create_many_effective_fields(model, data_obj);
        let row_keys: Vec<&str> = row_fields
            .iter()
            .map(|field| field.logical_name.as_str())
            .collect();

        if row_keys != expected_keys {
            let row_key_set: std::collections::HashSet<&str> = row_keys.iter().copied().collect();
            let missing: Vec<&str> = expected_keys
                .iter()
                .copied()
                .filter(|key| !row_key_set.contains(key))
                .collect();
            let extra: Vec<&str> = row_keys
                .iter()
                .copied()
                .filter(|key| !expected_key_set.contains(key))
                .collect();
            return Err(ProtocolError::InvalidParams(format!(
                "createMany rows must use the same key set after omitting server defaults; row {} differs from row 0 (missing: [{}], extra: [{}])",
                row_idx,
                missing.join(", "),
                extra.join(", "),
            )));
        }

        let mut row_values = Vec::with_capacity(relevant_fields.len());
        for field in &relevant_fields {
            if let Some(value) = field_input_value(state, data_obj, field, FieldInputMode::Create)?
            {
                row_values.push(value);
            } else {
                row_values.push(Value::Null);
            }
        }
        all_values.push(row_values);
    }

    let mut builder = Insert::into_table(&model.db_name)
        .with_capacity(InsertCapacity {
            columns: columns.len(),
            rows: all_values.len(),
            returning: usize::from(params.return_data) * metadata.scalar_markers().len(),
        })
        .columns(columns)
        .rows(all_values);
    if params.return_data {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build insert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert_owned(insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    execute_mutation_result(
        state,
        &sql,
        "Insert",
        tx_id.as_deref(),
        metadata.scalar_hints(),
        params.return_data,
    )
    .await
}

/// Apply one set of column assignments, reading the affected rows back
/// afterwards on a backend without `RETURNING`.
async fn update_rows(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    data: &JsonValue,
    return_data: bool,
    tx_id: Option<&str>,
) -> Result<MutationResultData, ProtocolError> {
    let metadata = state.model_metadata(model);

    let qualified_filter = parse_optional_model_filter(
        model,
        filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    let data_obj = data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;

    let mut assignments = Vec::with_capacity(metadata.scalar_fields().len());

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(value) = field_input_value(state, data_obj, field, FieldInputMode::Update)? {
            assignments.push((field_marker(model, field), value));
        }
    }

    let returns_inline = return_data && state.dialect.supports_returning();

    let mut builder = Update::table(&model.db_name)
        .with_capacity(UpdateCapacity {
            assignments: assignments.len(),
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        })
        .assignments(assignments);

    if let Some(filter) = qualified_filter.clone() {
        builder = builder.filter(filter);
    }

    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let update = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build update: {}", e)))?;

    let sql = state
        .dialect
        .render_update_owned(update)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if returns_inline || !return_data {
        return execute_mutation_result(
            state,
            &sql,
            "Update",
            tx_id,
            metadata.scalar_hints(),
            returns_inline,
        )
        .await;
    }

    state
        .in_transaction(tx_id, |tx| async move {
            let before = find_rows_by_expr(state, model, qualified_filter, None, Some(&tx)).await?;
            state.execute_affected_on(&sql, "Update", Some(&tx)).await?;

            let Some(key_filter) = updated_rows_filter(state, model, &before, data_obj)? else {
                return Ok(MutationResultData::Rows(Vec::new()));
            };
            let rows = find_rows_by_expr(state, model, Some(key_filter), None, Some(&tx)).await?;
            Ok(MutationResultData::Rows(rows))
        })
        .await
}

/// Load the one row a nested write hangs off.
///
/// Nested operations link related rows to a specific parent, so the filter has
/// to name exactly one: a filter matching several offers no single key to point
/// children at.
async fn single_nested_target(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    tx: &str,
) -> Result<Row, ProtocolError> {
    let mut rows = find_rows_by_filter(state, model, filter, 2, Some(tx)).await?;

    if rows.len() > 1 {
        return Err(ProtocolError::InvalidFilter(format!(
            "An update on '{}' that writes relations needs a filter matching exactly one row, and this one matches several",
            model.logical_name
        )));
    }

    rows.pop().ok_or_else(|| {
        ProtocolError::RecordNotFound(format!(
            "update on '{}' matched no record to write relations against",
            model.logical_name
        ))
    })
}

pub(super) async fn execute_update(
    state: &EngineState,
    params: UpdateParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let model = get_writable_model_or_error(state, &params.model)?;
    let plan = nested::split(state, model, &params.data, true)?;

    if plan.is_empty() {
        return update_rows(
            state,
            model,
            &params.filter,
            &params.data,
            params.return_data,
            params.transaction_id.as_deref(),
        )
        .await;
    }

    let return_data = params.return_data;

    state
        .in_transaction(params.transaction_id.as_deref(), |tx| async move {
            let current = single_nested_target(state, model, &params.filter, &tx).await?;
            let (data, deferred) =
                nested::prepare_parent_data(state, model, &plan, Some(&current), &tx).await?;

            let writes_columns = data.as_object().is_some_and(|obj| !obj.is_empty());
            let updated = if writes_columns {
                Some(update_rows(state, model, &params.filter, &data, true, Some(&tx)).await?)
            } else {
                None
            };

            let parent = updated
                .as_ref()
                .and_then(MutationResultData::first_row)
                .unwrap_or(&current);
            nested::apply_children(state, model, &plan, parent, &tx).await?;
            deferred.run(state, &tx).await?;

            if !return_data {
                return Ok(MutationResultData::Count(1));
            }
            Ok(updated.unwrap_or_else(|| MutationResultData::Rows(vec![current])))
        })
        .await
}

/// Resolve the conflict target of an upsert from its unique `where` filter.
///
/// `INSERT ... ON CONFLICT` needs the exact column list of one unique index, so
/// the filter has to name a whole constraint and nothing else — a partial or
/// mixed key would either fail in the database or silently match a different
/// index than the caller meant.
fn unique_conflict_target<'a>(
    model: &'a ModelIr,
    filter: &JsonValue,
) -> Result<Vec<&'a FieldIr>, ProtocolError> {
    let JsonValue::Object(filter_obj) = protocol_filter_body(filter) else {
        return Err(ProtocolError::InvalidFilter(
            "upsert where must be an object".to_string(),
        ));
    };

    if filter_obj.is_empty() {
        return Err(ProtocolError::InvalidFilter(
            "upsert where cannot be empty".to_string(),
        ));
    }

    let mut filter_fields = Vec::with_capacity(filter_obj.len());
    for key in filter_obj.keys() {
        let field = model
            .scalar_fields()
            .find(|field| field.logical_name == *key || field.db_name == *key)
            .ok_or_else(|| {
                ProtocolError::InvalidFilter(format!(
                    "Unknown field '{}' in upsert where on model '{}'",
                    key, model.logical_name
                ))
            })?;
        filter_fields.push(field);
    }

    let candidates = std::iter::once(model.primary_key.fields())
        .chain(
            model
                .unique_constraints
                .iter()
                .map(|constraint| constraint.fields.iter().map(String::as_str).collect()),
        )
        .find(|candidate: &Vec<&str>| {
            candidate.len() == filter_fields.len()
                && candidate.iter().all(|name| {
                    filter_fields
                        .iter()
                        .any(|field| field.logical_name == *name || field.db_name == *name)
                })
        });

    let Some(candidate) = candidates else {
        let mut names: Vec<&str> = filter_obj.keys().map(String::as_str).collect();
        names.sort_unstable();
        return Err(ProtocolError::InvalidFilter(format!(
            "upsert where [{}] does not match the primary key or any unique constraint of model '{}'",
            names.join(", "),
            model.logical_name
        )));
    };

    Ok(candidate
        .iter()
        .filter_map(|name| {
            filter_fields
                .iter()
                .copied()
                .find(|field| field.logical_name == *name || field.db_name == *name)
        })
        .collect())
}

async fn execute_upsert(
    state: &EngineState,
    params: UpsertParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_writable_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);

    let create_obj = params
        .create
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("create must be an object".to_string()))?;
    let update_obj = params
        .update
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("update must be an object".to_string()))?;

    let target_fields = unique_conflict_target(model, &params.filter)?;

    let scalar_field_capacity = metadata.scalar_fields().len();
    let mut columns = Vec::with_capacity(scalar_field_capacity);
    let mut values = Vec::with_capacity(scalar_field_capacity);

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(value) = field_input_value(state, create_obj, field, FieldInputMode::Create)? {
            columns.push(field_marker(model, field));
            values.push(value);
        }
    }

    for target in &target_fields {
        if !columns.iter().any(|column| column.name == target.db_name) {
            return Err(ProtocolError::InvalidParams(format!(
                "upsert create data must set '{}' because it is part of the conflict target",
                target.logical_name
            )));
        }
    }

    let mut assignments = Vec::with_capacity(update_obj.len());
    if !update_obj.is_empty() {
        for field in &model.fields {
            if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
                continue;
            }
            if let Some(value) =
                field_input_value(state, update_obj, field, FieldInputMode::Update)?
            {
                assignments.push((field_marker(model, field), value));
            }
        }
    }

    let returns_inline = params.return_data && state.dialect.supports_returning();

    let mut builder = Insert::into_table(&model.db_name)
        .with_capacity(InsertCapacity {
            columns: columns.len(),
            rows: 1,
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        })
        .columns(columns)
        .values(values)
        .on_conflict(nautilus_core::OnConflict::do_update(
            target_fields
                .iter()
                .map(|field| field_marker(model, field))
                .collect(),
            assignments,
        ));
    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build upsert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert_owned(insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if params.return_data && !returns_inline {
        state
            .execute_affected_on(&sql, "Insert", tx_id.as_deref())
            .await?;
        return read_back_upserted_row(state, model, &params.filter, tx_id.as_deref()).await;
    }

    execute_mutation_result(
        state,
        &sql,
        "Insert",
        tx_id.as_deref(),
        metadata.scalar_hints(),
        returns_inline,
    )
    .await
}

/// Fetch the upserted row on dialects without `RETURNING` (MySQL).
///
/// The write itself stays atomic; only the read is a second round-trip, so a
/// concurrent writer can still change the row between the two statements unless
/// the caller wraps the upsert in a transaction.
async fn read_back_upserted_row(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    tx_id: Option<&str>,
) -> Result<MutationResultData, ProtocolError> {
    let metadata = state.model_metadata(model);
    let qualified_filter = parse_and_qualify_model_filter(
        model,
        filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    let (sql, row_hints) = build_find_unique_sql(
        state,
        model,
        qualified_filter,
        &std::collections::HashSet::new(),
    )?;

    let rows = normalize_rows_with_hints(
        state.execute_query_on(&sql, "Query", tx_id).await?,
        &row_hints,
    )?;
    Ok(MutationResultData::Rows(rows))
}

/// Handle `query.upsert`.
pub(in crate::handlers) async fn handle_upsert(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpsertParams = parse_params(&request, "upsert")?;

    match execute_upsert(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "upsert result"),
        MutationResultData::Count(count) => wrap_count_result(count, "upsert result"),
    }
}

pub(in crate::handlers) async fn handle_upsert_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: UpsertParams = parse_params(&request, "upsert")?;
    execute_upsert(state, params).await?.into_rows("upsert")
}

pub(in crate::handlers) async fn handle_upsert_typed(
    state: &EngineState,
    params: UpsertParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_upsert(state, params).await?.into_rows("upsert")
}

/// Handle `query.create`.
pub(in crate::handlers) async fn handle_create(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CreateParams = parse_params(&request, "create")?;

    match execute_create(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "create result"),
        MutationResultData::Count(count) => wrap_count_result(count, "create result"),
    }
}

pub(in crate::handlers) async fn handle_create_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: CreateParams = parse_params(&request, "create")?;
    execute_create(state, params).await?.into_rows("create")
}

pub(in crate::handlers) async fn handle_create_typed(
    state: &EngineState,
    params: CreateParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_create(state, params).await?.into_rows("create")
}

/// Handle `query.createMany`.
pub(in crate::handlers) async fn handle_create_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CreateManyParams = parse_params(&request, "createMany")?;

    match execute_create_many(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "createMany result"),
        MutationResultData::Count(count) => wrap_count_result(count, "createMany result"),
    }
}

pub(in crate::handlers) async fn handle_create_many_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: CreateManyParams = parse_params(&request, "createMany")?;
    execute_create_many(state, params)
        .await?
        .into_rows("createMany")
}

pub(in crate::handlers) async fn handle_create_many_typed(
    state: &EngineState,
    params: CreateManyParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_create_many(state, params)
        .await?
        .into_rows("createMany")
}

/// Handle `query.update`.
pub(in crate::handlers) async fn handle_update(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpdateParams = parse_params(&request, "update")?;

    match execute_update(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "update result"),
        MutationResultData::Count(count) => wrap_count_result(count, "update result"),
    }
}

pub(in crate::handlers) async fn handle_update_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: UpdateParams = parse_params(&request, "update")?;
    execute_update(state, params).await?.into_rows("update")
}

pub(in crate::handlers) async fn handle_update_typed(
    state: &EngineState,
    params: UpdateParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_update(state, params).await?.into_rows("update")
}

pub(super) async fn execute_delete(
    state: &EngineState,
    params: DeleteParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let model = get_writable_model_or_error(state, &params.model)?;
    let tx_id = params.transaction_id;
    let metadata = state.model_metadata(model);

    let qualified_filter = parse_optional_model_filter(
        model,
        &params.filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    let returns_inline = params.return_data && state.dialect.supports_returning();

    let mut builder = Delete::from_table(&model.db_name).with_capacity(DeleteCapacity {
        returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
    });
    if let Some(filter) = qualified_filter.clone() {
        builder = builder.filter(filter);
    }

    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let delete = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build delete: {}", e)))?;

    let sql = state
        .dialect
        .render_delete_owned(delete)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if returns_inline || !params.return_data {
        return execute_mutation_result(
            state,
            &sql,
            "Delete",
            tx_id.as_deref(),
            metadata.scalar_hints(),
            returns_inline,
        )
        .await;
    }

    state
        .in_transaction(tx_id.as_deref(), |tx| async move {
            let rows = find_rows_by_expr(state, model, qualified_filter, None, Some(&tx)).await?;
            state.execute_affected_on(&sql, "Delete", Some(&tx)).await?;
            Ok(MutationResultData::Rows(rows))
        })
        .await
}

/// Handle `query.delete`.
pub(in crate::handlers) async fn handle_delete(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: DeleteParams = parse_params(&request, "delete")?;

    match execute_delete(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "delete result"),
        MutationResultData::Count(count) => wrap_count_result(count, "delete result"),
    }
}

/// Handle `query.updateMany`.
///
/// Reuses the `query.update` statement builder with `return_data` pinned off,
/// so the model's `RETURNING` projection is never emitted and the result is the
/// affected-row count alone.
pub(in crate::handlers) async fn handle_update_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpdateManyParams = parse_params(&request, "updateMany")?;

    wrap_count_result(
        execute_update_many(state, params).await?,
        "updateMany result",
    )
}

pub(in crate::handlers) async fn handle_update_many_typed(
    state: &EngineState,
    params: UpdateManyParams,
) -> Result<usize, ProtocolError> {
    execute_update_many(state, params).await
}

async fn execute_update_many(
    state: &EngineState,
    params: UpdateManyParams,
) -> Result<usize, ProtocolError> {
    Ok(execute_update(
        state,
        UpdateParams {
            protocol_version: params.protocol_version,
            model: params.model,
            filter: params.filter,
            data: params.data,
            transaction_id: params.transaction_id,
            return_data: false,
        },
    )
    .await?
    .into_count())
}

/// Handle `query.deleteMany`. See [`handle_update_many`].
pub(in crate::handlers) async fn handle_delete_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: DeleteManyParams = parse_params(&request, "deleteMany")?;

    wrap_count_result(
        execute_delete_many(state, params).await?,
        "deleteMany result",
    )
}

pub(in crate::handlers) async fn handle_delete_many_typed(
    state: &EngineState,
    params: DeleteManyParams,
) -> Result<usize, ProtocolError> {
    execute_delete_many(state, params).await
}

async fn execute_delete_many(
    state: &EngineState,
    params: DeleteManyParams,
) -> Result<usize, ProtocolError> {
    Ok(execute_delete(
        state,
        DeleteParams {
            protocol_version: params.protocol_version,
            model: params.model,
            filter: params.filter,
            transaction_id: params.transaction_id,
            return_data: false,
        },
    )
    .await?
    .into_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_schema::validate_schema_source;

    fn model_of(source: &str, name: &str) -> ModelIr {
        validate_schema_source(source)
            .expect("schema should validate")
            .ir
            .models
            .into_values()
            .find(|model| model.logical_name == name)
            .expect("model missing")
    }

    fn user_model() -> ModelIr {
        model_of(
            r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  team  String
  slot  Int
  name  String

  @@unique([team, slot])
}
"#,
            "User",
        )
    }

    #[test]
    fn conflict_target_accepts_a_single_column_unique_constraint() {
        let model = user_model();
        let filter = serde_json::json!({ "where": { "email": "alice@example.com" } });

        let target = unique_conflict_target(&model, &filter).expect("email is unique");

        assert_eq!(
            target
                .iter()
                .map(|field| field.logical_name.as_str())
                .collect::<Vec<_>>(),
            vec!["email"]
        );
    }

    #[test]
    fn conflict_target_accepts_the_primary_key() {
        let model = user_model();
        let filter = serde_json::json!({ "id": 7 });

        let target = unique_conflict_target(&model, &filter).expect("id is the primary key");

        assert_eq!(target.len(), 1);
        assert_eq!(target[0].logical_name, "id");
    }

    #[test]
    fn conflict_target_orders_columns_as_the_constraint_declares_them() {
        let model = user_model();
        let filter = serde_json::json!({ "slot": 3, "team": "blue" });

        let target = unique_conflict_target(&model, &filter).expect("(team, slot) is unique");

        assert_eq!(
            target
                .iter()
                .map(|field| field.logical_name.as_str())
                .collect::<Vec<_>>(),
            vec!["team", "slot"]
        );
    }

    #[test]
    fn conflict_target_rejects_a_partial_composite_key() {
        let model = user_model();
        let filter = serde_json::json!({ "team": "blue" });

        let error = unique_conflict_target(&model, &filter)
            .expect_err("half of a composite unique key is not a conflict target");

        assert!(
            matches!(&error, ProtocolError::InvalidFilter(message) if message.contains("unique constraint")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn conflict_target_rejects_a_non_unique_column() {
        let model = user_model();
        let filter = serde_json::json!({ "name": "Alice" });

        let error =
            unique_conflict_target(&model, &filter).expect_err("name carries no unique constraint");

        assert!(matches!(error, ProtocolError::InvalidFilter(_)));
    }

    #[test]
    fn conflict_target_rejects_an_unknown_field() {
        let model = user_model();
        let filter = serde_json::json!({ "nickname": "Ali" });

        let error =
            unique_conflict_target(&model, &filter).expect_err("nickname is not a model field");

        assert!(
            matches!(&error, ProtocolError::InvalidFilter(message) if message.contains("Unknown field")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn conflict_target_rejects_an_empty_filter() {
        let model = user_model();
        let filter = serde_json::json!({});

        assert!(unique_conflict_target(&model, &filter).is_err());
    }
}
