use super::common::{
    model_scalar_value_hints, parse_optional_model_filter, wrap_count_result, wrap_mutation_result,
};
use super::*;

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

fn create_field_input_value(
    data_obj: &JsonMap<String, JsonValue>,
    field: &FieldIr,
) -> Result<Option<Value>, ProtocolError> {
    if field.is_updated_at {
        return match row_field_json(data_obj, field) {
            Some(json_val) if !json_val.is_null() => {
                Ok(Some(json_to_value_field(json_val, &field.field_type)?))
            }
            _ => Ok(Some(updated_at_now_value())),
        };
    }

    let Some(json_val) = row_field_json(data_obj, field) else {
        return Ok(None);
    };

    let is_null = json_val.is_null();
    let has_fn_default = matches!(&field.default_value, Some(DefaultValue::Function(_)));
    if is_null && has_fn_default {
        return Ok(None);
    }

    Ok(Some(json_to_value_field(json_val, &field.field_type)?))
}

fn update_field_input_value(
    data_obj: &JsonMap<String, JsonValue>,
    field: &FieldIr,
) -> Result<Option<Value>, ProtocolError> {
    if field.is_updated_at {
        return match row_field_json(data_obj, field) {
            Some(json_val) if !json_val.is_null() => {
                Ok(Some(json_to_value_field(json_val, &field.field_type)?))
            }
            _ => Ok(Some(updated_at_now_value())),
        };
    }

    row_field_json(data_obj, field)
        .map(|json_val| json_to_value_field(json_val, &field.field_type))
        .transpose()
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

/// Handle `query.create`.
pub(super) async fn handle_create(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CreateParams = serde_json::from_value(request.params)
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid create params: {}", e)))?;

    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_model_or_error(state, &params.model)?;

    let data_obj = params
        .data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;

    let mut builder = Insert::into_table(&model.db_name);
    let mut columns = Vec::new();
    let mut values = Vec::new();

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(value) = create_field_input_value(data_obj, field)? {
            columns.push(field_marker(model, field));
            values.push(value);
        }
    }

    builder = builder.columns(columns).values(values);
    if params.return_data {
        builder = builder.returning(model_scalar_markers(model));
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build insert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert(&insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if params.return_data {
        let rows = normalize_rows_with_hints(
            state
                .execute_query_on(&sql, "Insert", tx_id.as_deref())
                .await?,
            &model_scalar_value_hints(model),
        )?;
        wrap_mutation_result(&rows, "create result")
    } else {
        let count = state
            .execute_affected_on(&sql, "Insert", tx_id.as_deref())
            .await?;
        wrap_count_result(count, "create result")
    }
}

/// Handle `query.createMany`.
pub(super) async fn handle_create_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CreateManyParams = serde_json::from_value(request.params)
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid createMany params: {}", e)))?;

    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_model_or_error(state, &params.model)?;

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

    let mut all_values = Vec::new();
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

        let mut row_values = Vec::new();
        for field in &relevant_fields {
            if let Some(value) = create_field_input_value(data_obj, field)? {
                row_values.push(value);
            } else {
                row_values.push(Value::Null);
            }
        }
        all_values.push(row_values);
    }

    let mut builder = Insert::into_table(&model.db_name).columns(columns);
    for row in all_values {
        builder = builder.values(row);
    }
    if params.return_data {
        builder = builder.returning(model_scalar_markers(model));
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build insert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert(&insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if params.return_data {
        let rows = normalize_rows_with_hints(
            state
                .execute_query_on(&sql, "Insert", tx_id.as_deref())
                .await?,
            &model_scalar_value_hints(model),
        )?;
        wrap_mutation_result(&rows, "createMany result")
    } else {
        let count = state
            .execute_affected_on(&sql, "Insert", tx_id.as_deref())
            .await?;
        wrap_count_result(count, "createMany result")
    }
}

/// Handle `query.update`.
pub(super) async fn handle_update(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpdateParams = serde_json::from_value(request.params)
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid update params: {}", e)))?;

    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_model_or_error(state, &params.model)?;

    let field_type_map = build_field_type_map(model);
    let qualified_filter = parse_optional_model_filter(model, &params.filter, &field_type_map)?;

    let data_obj = params
        .data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;

    let mut builder = Update::table(&model.db_name);

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(value) = update_field_input_value(data_obj, field)? {
            builder = builder.set(field_marker(model, field), value);
        }
    }

    if let Some(filter) = qualified_filter {
        builder = builder.filter(filter);
    }

    if params.return_data {
        builder = builder.returning(model_scalar_markers(model));
    }

    let update = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build update: {}", e)))?;

    let sql = state
        .dialect
        .render_update(&update)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if params.return_data {
        let rows = normalize_rows_with_hints(
            state
                .execute_query_on(&sql, "Update", tx_id.as_deref())
                .await?,
            &model_scalar_value_hints(model),
        )?;
        wrap_mutation_result(&rows, "update result")
    } else {
        let count = state
            .execute_affected_on(&sql, "Update", tx_id.as_deref())
            .await?;
        wrap_count_result(count, "update result")
    }
}

/// Handle `query.delete`.
pub(super) async fn handle_delete(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: DeleteParams = serde_json::from_value(request.params)
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid delete params: {}", e)))?;

    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_model_or_error(state, &params.model)?;

    let field_type_map = build_field_type_map(model);
    let qualified_filter = parse_optional_model_filter(model, &params.filter, &field_type_map)?;

    let mut builder = Delete::from_table(&model.db_name);
    if let Some(filter) = qualified_filter {
        builder = builder.filter(filter);
    }

    if params.return_data {
        builder = builder.returning(model_scalar_markers(model));
    }

    let delete = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build delete: {}", e)))?;

    let sql = state
        .dialect
        .render_delete(&delete)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if params.return_data {
        let rows = normalize_rows_with_hints(
            state
                .execute_query_on(&sql, "Delete", tx_id.as_deref())
                .await?,
            &model_scalar_value_hints(model),
        )?;
        wrap_mutation_result(&rows, "delete result")
    } else {
        let count = state
            .execute_affected_on(&sql, "Delete", tx_id.as_deref())
            .await?;
        wrap_count_result(count, "delete result")
    }
}
