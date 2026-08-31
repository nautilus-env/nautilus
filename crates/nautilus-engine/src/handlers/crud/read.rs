use std::collections::HashMap;

use futures::stream::StreamExt;

use super::common::{
    parse_and_qualify_model_filter, qualify_model_filter, wrap_count_result, wrap_data_result,
    wrap_result,
};
use super::include::hydrate_rows_with_includes;
use super::*;
use crate::conversion::normalize_row_with_hints;
use crate::filter::IncludeNode;
use crate::state::connector_to_protocol;
use nautilus_core::JsonPathCast;
use nautilus_migrate::DatabaseProvider;
use nautilus_schema::ir::{CompositeFieldIr, ResolvedFieldType, ScalarType};

/// Pre-execution artefact produced by [`build_find_many_plan`].
///
/// Splitting plan-building from row consumption lets the buffered fast path
/// ([`execute_find_many_rows`]) and the chunked streaming path
/// ([`stream_find_many_chunked`]) share the same SQL/hint computation while
/// each owning its consumption strategy. Streaming requires `backward = false`
/// and `include.is_empty()` because both transformations need the full row
/// set in memory before any output can leave the engine.
struct FindManyPlan {
    sql: Sql,
    row_hints: Vec<Option<ValueHint>>,
    backward: bool,
    include: HashMap<String, IncludeNode>,
}

enum ResolvedOrderTarget {
    Column(String),
    Expr(Expr),
}

fn strip_order_qualifier(name: &str) -> &str {
    name.split_once("__")
        .map(|(_, column)| column)
        .unwrap_or(name)
}

fn is_orderable_nested_field(field: &CompositeFieldIr) -> bool {
    if field.is_array {
        return false;
    }

    match &field.field_type {
        ResolvedFieldType::Enum { .. } => true,
        ResolvedFieldType::Scalar(
            ScalarType::Boolean
            | ScalarType::Json
            | ScalarType::Jsonb
            | ScalarType::Hstore
            | ScalarType::Geometry
            | ScalarType::Geography
            | ScalarType::Vector { .. }
            | ScalarType::Bytes,
        ) => false,
        ResolvedFieldType::Scalar(_) => true,
        ResolvedFieldType::CompositeType { .. } | ResolvedFieldType::Relation(_) => false,
    }
}

fn json_cast_for_nested_field(field: &CompositeFieldIr) -> JsonPathCast {
    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Int | ScalarType::BigInt) => JsonPathCast::Signed,
        ResolvedFieldType::Scalar(ScalarType::Float) => JsonPathCast::Double,
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => JsonPathCast::Decimal,
        _ => JsonPathCast::None,
    }
}

fn resolve_order_target(
    state: &EngineState,
    model: &ModelIr,
    logical_to_db: &HashMap<String, String>,
    field_path: &str,
) -> Result<ResolvedOrderTarget, ProtocolError> {
    let field_path = strip_order_qualifier(field_path);
    let Some((parent_name, nested_name)) = field_path.split_once('.') else {
        if model
            .scalar_fields()
            .find(|field| field.logical_name == field_path || field.db_name == field_path)
            .is_some_and(|field| {
                matches!(field.field_type, ResolvedFieldType::CompositeType { .. })
            })
        {
            return Err(ProtocolError::InvalidFilter(format!(
                "Composite field '{}' cannot be ordered directly; use '{}.<field>'",
                field_path, field_path
            )));
        }

        let db_col = logical_to_db
            .get(field_path)
            .cloned()
            .unwrap_or_else(|| field_path.to_string());
        return Ok(ResolvedOrderTarget::Column(db_col));
    };

    let parent_field = model
        .scalar_fields()
        .find(|field| field.logical_name == parent_name || field.db_name == parent_name)
        .ok_or_else(|| {
            ProtocolError::InvalidFilter(format!("Unknown orderBy field '{}'", parent_name))
        })?;

    if parent_field.is_array {
        return Err(ProtocolError::InvalidFilter(format!(
            "Composite array field '{}' cannot be used with orderBy path '{}'",
            parent_name, field_path
        )));
    }

    let ResolvedFieldType::CompositeType { type_name, .. } = &parent_field.field_type else {
        return Err(ProtocolError::InvalidFilter(format!(
            "orderBy path '{}' starts with non-composite field '{}'",
            field_path, parent_name
        )));
    };

    let composite = state.schema.get_composite_type(type_name).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Composite type '{}' not found while resolving orderBy '{}'",
            type_name, field_path
        ))
    })?;

    let nested_field = composite
        .fields
        .iter()
        .find(|field| field.logical_name == nested_name || field.db_name == nested_name)
        .ok_or_else(|| {
            ProtocolError::InvalidFilter(format!(
                "Unknown orderBy composite field '{}.{}'",
                parent_name, nested_name
            ))
        })?;

    if !is_orderable_nested_field(nested_field) {
        return Err(ProtocolError::InvalidFilter(format!(
            "Field '{}' cannot be used with classic orderBy because it is not orderable",
            field_path
        )));
    }

    Ok(ResolvedOrderTarget::Expr(Expr::composite_field(
        model.db_name.clone(),
        parent_field.db_name.clone(),
        nested_field.db_name.clone(),
        nested_field.logical_name.clone(),
        json_cast_for_nested_field(nested_field),
    )))
}

fn build_find_many_plan(
    state: &EngineState,
    model: &ModelIr,
    query_args: QueryArgs,
) -> Result<FindManyPlan, ProtocolError> {
    let QueryArgs {
        filter,
        order_by,
        take,
        skip,
        include,
        select,
        cursor,
        backward,
        distinct,
        nearest,
        partition,
        join,
    } = query_args;

    let metadata = state.model_metadata(model);
    let logical_to_db = metadata.logical_to_db();
    let qualified_filter =
        filter.map(|expr| qualify_filter_columns(expr, &model.db_name, logical_to_db));
    let pk_fields = metadata.primary_key_fields();

    let mut builder = Select::from_table(&model.db_name).with_capacity(SelectCapacity {
        items: metadata.scalar_fields().len(),
        joins: usize::from(join.is_some()),
        order_by_columns: order_by.len() + distinct.len() + pk_fields.len(),
        order_by_exprs: usize::from(nearest.is_some()),
        distinct: distinct.len(),
        ..SelectCapacity::default()
    });
    let mut row_hints = Vec::new();

    for field in metadata.scalar_fields() {
        if !select.is_empty()
            && !select.contains(field.logical_name())
            && !pk_fields
                .iter()
                .any(|pk_field| pk_field.logical_name() == field.logical_name())
        {
            continue;
        }
        builder = builder.item(SelectItem::from(field.marker().clone()));
        row_hints.push(field.hint());
    }

    let combined_filter = if let Some(ref cursor_map) = cursor {
        let pk_refs: Vec<(&str, &str)> = pk_fields
            .iter()
            .map(|field| (field.logical_name(), field.qualified_column()))
            .collect();

        let cursor_pred = build_cursor_predicate(&pk_refs, cursor_map, backward)
            .map_err(|e| ProtocolError::InvalidParams(format!("Invalid cursor: {}", e)))?;

        let existing_order_cols: std::collections::HashSet<&str> =
            order_by.iter().map(|order| order.column.as_str()).collect();
        for pk_field in pk_fields {
            if !existing_order_cols.contains(pk_field.db_name()) {
                let dir = if backward {
                    OrderDir::Desc
                } else {
                    OrderDir::Asc
                };
                builder = builder.order_by(pk_field.db_name().to_string(), dir);
            }
        }

        Some(match qualified_filter {
            Some(existing) => existing.and(cursor_pred),
            None => cursor_pred,
        })
    } else {
        qualified_filter
    };

    if let Some(filter_expr) = combined_filter {
        builder = builder.filter(filter_expr);
    }

    if let Some(nearest) = nearest {
        let db_col = logical_to_db
            .get(nearest.field.as_str())
            .cloned()
            .unwrap_or(nearest.field);
        let distance_expr = Expr::vector_distance(
            nearest.metric,
            Expr::column(format!("{}__{}", model.db_name, db_col)),
            Expr::param(Value::Vector(nearest.query)),
        );
        builder = builder.order_by_expr(distance_expr, OrderDir::Asc);
    }

    if !distinct.is_empty() {
        let existing_order_cols: std::collections::HashSet<&str> =
            order_by.iter().map(|order| order.column.as_str()).collect();
        for column in &distinct {
            let db_col = logical_to_db
                .get(column.as_str())
                .cloned()
                .unwrap_or_else(|| column.clone());
            if !existing_order_cols.contains(db_col.as_str()) {
                let dir = if backward {
                    OrderDir::Desc
                } else {
                    OrderDir::Asc
                };
                builder = builder.order_by(db_col, dir);
            }
        }
    }

    for order in order_by {
        let dir = if backward {
            match order.direction {
                OrderDir::Asc => OrderDir::Desc,
                OrderDir::Desc => OrderDir::Asc,
            }
        } else {
            order.direction
        };
        match resolve_order_target(state, model, logical_to_db, &order.column)? {
            ResolvedOrderTarget::Column(db_col) => {
                builder = builder.order_by(db_col, dir);
            }
            ResolvedOrderTarget::Expr(expr) => {
                builder = builder.order_by_expr(expr, dir);
            }
        }
    }

    if let Some(take) = take {
        builder = builder.take(take);
    }
    if let Some(skip) = skip {
        builder = builder.skip(skip);
    }
    if !distinct.is_empty() {
        let distinct_db: Vec<String> = distinct
            .iter()
            .map(|column| {
                logical_to_db
                    .get(column.as_str())
                    .cloned()
                    .unwrap_or_else(|| column.clone())
            })
            .collect();
        builder = builder.distinct(distinct_db);
    }

    if let Some(window) = partition {
        builder = builder.partition_window(window);
    }

    // The joined columns are appended to the select list after the model's own,
    // so their hints have to be appended in the same order for
    // `normalize_row_with_hints` to line up with the row it decodes.
    if let Some(join) = join {
        row_hints.extend(join.hints);
        builder = builder.join(join.clause);
    }

    let select = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build query: {}", e)))?;

    let sql = state
        .dialect
        .render_select_owned(select)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    Ok(FindManyPlan {
        sql,
        row_hints,
        backward,
        include,
    })
}

/// Canonicalise a projection for use in a plan-cache key: selected fields
/// plus implicit PK fields, sorted, or empty when all columns are selected.
fn resolved_projection(
    metadata: &crate::metadata::ModelMetadata,
    selected_fields: &std::collections::HashSet<&str>,
) -> Vec<String> {
    if selected_fields.is_empty() {
        return Vec::new();
    }
    let mut combined: Vec<String> = selected_fields.iter().map(|s| s.to_string()).collect();
    for pk in metadata.primary_key_fields() {
        let logical = pk.logical_name();
        if !selected_fields.contains(logical) {
            combined.push(logical.to_string());
        }
    }
    combined.sort();
    combined.dedup();
    combined
}

/// Build the plan-cache key (and the owned parameter values to bind on a hit)
/// for a `findMany`/`findFirst` request, or `None` when the request is not
/// cacheable: cursor, backward pagination, distinct, vector ordering, a joined
/// relation table and includes change the SQL or the post-processing in ways
/// the cached replay does not cover, and the filter must be a flat parametric
/// AND chain.
fn find_many_cache_request(
    state: &EngineState,
    model: &ModelIr,
    query_args: &QueryArgs,
) -> Option<(crate::plan_cache::FindManyPlanKey, Vec<Value>)> {
    if query_args.cursor.is_some()
        || query_args.backward
        || query_args.nearest.is_some()
        || query_args.partition.is_some()
        || query_args.join.is_some()
        || !query_args.distinct.is_empty()
        || !query_args.include.is_empty()
    {
        return None;
    }

    let (filter_shape, params) = match &query_args.filter {
        None => (Vec::new(), Vec::new()),
        Some(filter) => {
            let shape = crate::plan_cache::extract_param_filter(filter)?;
            (
                shape
                    .predicates
                    .iter()
                    .map(|(column, op, variant)| ((*column).to_string(), op.clone(), *variant))
                    .collect(),
                shape.values.iter().map(|value| (*value).clone()).collect(),
            )
        }
    };

    let metadata = state.model_metadata(model);
    let selected_refs: std::collections::HashSet<&str> =
        query_args.select.iter().map(String::as_str).collect();

    Some((
        crate::plan_cache::FindManyPlanKey {
            model_db_name: model.db_name.clone(),
            selected_logical_fields: resolved_projection(metadata, &selected_refs),
            filter_shape,
            order_by: query_args
                .order_by
                .iter()
                .map(|order| (order.column.clone(), order.direction))
                .collect(),
            take: query_args.take,
            skip: query_args.skip,
        },
        params,
    ))
}

pub(super) async fn execute_find_many_rows(
    state: &EngineState,
    model: &ModelIr,
    query_args: QueryArgs,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    if let Some((cache_key, params)) = find_many_cache_request(state, model, &query_args) {
        if let Some(plan) = state.plan_cache().get_find_many(&cache_key) {
            let sql = Sql {
                text: plan.sql_text.clone(),
                params,
            };
            return normalize_rows_with_hints(
                state.execute_query_on(&sql, "Query", tx_id).await?,
                &plan.row_hints,
            );
        }

        let plan = build_find_many_plan(state, model, query_args)?;
        state.plan_cache().insert_find_many(
            cache_key,
            std::sync::Arc::new(crate::plan_cache::CachedReadPlan {
                sql_text: plan.sql.text.clone(),
                row_hints: plan.row_hints.clone(),
            }),
        );
        return normalize_rows_with_hints(
            state.execute_query_on(&plan.sql, "Query", tx_id).await?,
            &plan.row_hints,
        );
    }

    let plan = build_find_many_plan(state, model, query_args)?;
    let mut rows = normalize_rows_with_hints(
        state.execute_query_on(&plan.sql, "Query", tx_id).await?,
        &plan.row_hints,
    )?;

    if plan.backward {
        rows.reverse();
    }

    hydrate_rows_with_includes(state, model, rows, &plan.include, tx_id).await
}

pub(super) async fn execute_find_many_params(
    state: &EngineState,
    params: FindManyParams,
) -> Result<Vec<Row>, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        crate::filter::SchemaContext::with_state(state),
    )?;

    execute_find_many_rows(state, model, query_args, tx_id.as_deref()).await
}

pub(in crate::handlers) async fn execute_find_many_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindManyArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let model = get_model_or_error(state, model_name)?;
    let metadata = state.model_metadata(model);
    let query_args = QueryArgs::from_find_many_args(args, metadata.field_types())?;

    execute_find_many_rows(state, model, query_args, transaction_id).await
}

pub(super) fn build_find_unique_sql(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Expr,
    selected_fields: &std::collections::HashSet<&str>,
) -> Result<(Sql, Vec<Option<ValueHint>>), ProtocolError> {
    let metadata = state.model_metadata(model);
    let pk_fields = metadata.primary_key_fields();

    let mut builder = Select::from_table(&model.db_name).with_capacity(SelectCapacity {
        items: metadata.scalar_fields().len(),
        ..SelectCapacity::default()
    });
    let mut row_hints = Vec::new();

    for field in metadata.scalar_fields() {
        if !selected_fields.is_empty()
            && !selected_fields.contains(field.logical_name())
            && !pk_fields
                .iter()
                .any(|pk_field| pk_field.logical_name() == field.logical_name())
        {
            continue;
        }

        builder = builder.item(SelectItem::from(field.marker().clone()));
        row_hints.push(field.hint());
    }

    let select = builder
        .filter(qualified_filter)
        .take(1)
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build query: {}", e)))?;

    let sql = state
        .dialect
        .render_select_owned(select)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    Ok((sql, row_hints))
}

/// Load up to `limit` rows of `model` matching `filter`, projecting every
/// scalar column.
///
/// The nested-write paths use this to resolve `connect` targets and to find the
/// row a nested operation hangs off, so the projection has to cover whatever
/// key the relation references, not just the primary key.
pub(super) async fn find_rows_by_filter(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    limit: i32,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let metadata = state.model_metadata(model);
    let qualified_filter = parse_and_qualify_model_filter(
        model,
        filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    find_rows_by_expr(state, model, Some(qualified_filter), Some(limit), tx_id).await
}

/// Load the rows of `model` matching an already-qualified predicate.
///
/// The mutation paths reuse the predicate they built for the statement itself,
/// so the read-back a backend without `RETURNING` needs sees exactly the rows
/// the statement did.
pub(super) async fn find_rows_by_expr(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Option<Expr>,
    limit: Option<i32>,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let metadata = state.model_metadata(model);

    let mut builder = Select::from_table(&model.db_name).with_capacity(SelectCapacity {
        items: metadata.scalar_fields().len(),
        ..SelectCapacity::default()
    });
    let mut row_hints = Vec::with_capacity(metadata.scalar_fields().len());
    for field in metadata.scalar_fields() {
        builder = builder.item(SelectItem::from(field.marker().clone()));
        row_hints.push(field.hint());
    }

    if let Some(filter) = qualified_filter {
        builder = builder.filter(filter);
    }
    if let Some(limit) = limit {
        builder = builder.take(limit);
    }

    let select = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build query: {}", e)))?;

    let sql = state
        .dialect
        .render_select_owned(select)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    normalize_rows_with_hints(
        state.execute_query_on(&sql, "Query", tx_id).await?,
        &row_hints,
    )
}

/// Load every row of `model` matching `filter`.
///
/// The nested-write path uses this to resolve the members of a relation before
/// it narrows an operation to them, where a limit would silently drop rows the
/// caller asked to reach.
pub(super) async fn find_all_rows_by_filter(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let metadata = state.model_metadata(model);
    let qualified_filter = parse_and_qualify_model_filter(
        model,
        filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    find_rows_by_expr(state, model, Some(qualified_filter), None, tx_id).await
}

/// Load the single row of `model` matching `filter`, or `None`.
pub(super) async fn find_one_row(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    tx_id: Option<&str>,
) -> Result<Option<Row>, ProtocolError> {
    Ok(find_rows_by_filter(state, model, filter, 1, tx_id)
        .await?
        .into_iter()
        .next())
}

async fn execute_find_unique_rows(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Expr,
    selected_fields: &std::collections::HashSet<&str>,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let (sql, row_hints) = build_find_unique_sql(state, model, qualified_filter, selected_fields)?;
    normalize_rows_with_hints(
        state.execute_query_on(&sql, "Query", tx_id).await?,
        &row_hints,
    )
}

/// Build the [`FindUniquePlanKey`] for a request matched by [`extract_simple_eq_filter`].
///
/// The resolved projection is canonicalised (selected fields plus implicit PK
/// fields, sorted) so semantically equivalent inputs share a cache entry.
fn find_unique_plan_key(
    model: &ModelIr,
    metadata: &crate::metadata::ModelMetadata,
    selected_fields: &std::collections::HashSet<&str>,
    shape: &crate::plan_cache::EqFilterShape<'_>,
) -> crate::plan_cache::FindUniquePlanKey {
    crate::plan_cache::FindUniquePlanKey {
        model_db_name: model.db_name.clone(),
        selected_logical_fields: resolved_projection(metadata, selected_fields),
        filter_columns: shape.columns.iter().map(|s| s.to_string()).collect(),
    }
}

pub(in crate::handlers) async fn execute_find_unique_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindUniqueArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    if !args.include.is_empty() {
        return execute_find_many_typed(
            state,
            model_name,
            &nautilus_core::FindManyArgs {
                where_: Some(args.where_.clone()),
                take: Some(1),
                select: args.select.clone(),
                include: args.include.clone(),
                ..Default::default()
            },
            transaction_id,
        )
        .await;
    }

    let model = get_model_or_error(state, model_name)?;
    let metadata = state.model_metadata(model);
    let selected_fields: std::collections::HashSet<&str> = args
        .select
        .iter()
        .filter_map(|(field, enabled)| enabled.then_some(field.as_str()))
        .collect();

    // Plan-cache fast path: only available when the filter is a flat AND chain
    // of `Column = Param` predicates so we can replay the rendered SQL by
    // re-binding parameter values without rebuilding the AST.
    if let Some(shape) = crate::plan_cache::extract_simple_eq_filter(&args.where_) {
        let cache_key = find_unique_plan_key(model, metadata, &selected_fields, &shape);
        if let Some(plan) = state.plan_cache().get_find_unique(&cache_key) {
            let sql = Sql {
                text: plan.sql_text.clone(),
                params: shape.values.iter().map(|v| (*v).clone()).collect(),
            };
            return normalize_rows_with_hints(
                state
                    .execute_query_on(&sql, "Query", transaction_id)
                    .await?,
                &plan.row_hints,
            );
        }

        let qualified_filter = qualify_filter_columns(
            args.where_.clone(),
            &model.db_name,
            metadata.logical_to_db(),
        );
        let (sql, row_hints) =
            build_find_unique_sql(state, model, qualified_filter, &selected_fields)?;
        state.plan_cache().insert_find_unique(
            cache_key,
            std::sync::Arc::new(crate::plan_cache::CachedReadPlan {
                sql_text: sql.text.clone(),
                row_hints: row_hints.clone(),
            }),
        );
        return normalize_rows_with_hints(
            state
                .execute_query_on(&sql, "Query", transaction_id)
                .await?,
            &row_hints,
        );
    }

    let qualified_filter = qualify_filter_columns(
        args.where_.clone(),
        &model.db_name,
        metadata.logical_to_db(),
    );
    execute_find_unique_rows(
        state,
        model,
        qualified_filter,
        &selected_fields,
        transaction_id,
    )
    .await
}

/// Drive `findMany` row-by-row through the connector's owned-stream path,
/// emitting `partial: true` chunks as they fill up.
///
/// This is the chunked-response code path: when the client sets `chunkSize`
/// and a response channel is available, the engine forwards each batch of at
/// most `chunk_size` rows to the transport as soon as they arrive, instead of
/// buffering the full result set first. The final batch is returned to the
/// caller (so the outer dispatcher emits a non-partial reply at the end).
///
/// Streaming is only safe when the result set does not need a global
/// transformation before output:
///
/// - `backward = true` would require reversing the entire `Vec<Row>` after the
///   fetch completes, defeating streaming semantics.
/// - `include` is not empty: relation hydration runs a follow-up batch query
///   keyed on the parent PKs (`WHERE child.fk IN (parent_ids…)`), which needs
///   every parent row in memory.
///
/// Both cases fall back to the buffered path in [`handle_find_many`].
async fn stream_find_many_chunked(
    state: &EngineState,
    plan: FindManyPlan,
    tx_id: Option<&str>,
    chunk_size: usize,
    request_id: Option<nautilus_protocol::RpcId>,
    sender: mpsc::Sender<RpcResponse>,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let mut row_stream = state
        .execute_query_stream_on(plan.sql, tx_id)
        .await
        .map_err(|e| match e {
            ProtocolError::ConnectionFailed(msg) => ProtocolError::ConnectionFailed(msg),
            other => other,
        })?;

    // `pending` holds the most recently *filled* chunk: it is held back until
    // we know whether another full chunk follows. If yes, `pending` becomes a
    // partial response on the wire; if not (i.e. it is the last chunk), it is
    // returned as the final result so the caller emits a non-partial reply.
    // `accum` collects rows until it reaches `chunk_size`.
    let mut pending: Vec<Row> = Vec::with_capacity(chunk_size);
    let mut accum: Vec<Row> = Vec::with_capacity(chunk_size);

    while let Some(item) = row_stream.next().await {
        let raw_row = item.map_err(|e| connector_to_protocol(e, "Query"))?;
        let row = normalize_row_with_hints(raw_row, &plan.row_hints)?;
        accum.push(row);
        if accum.len() >= chunk_size {
            if !pending.is_empty() {
                let raw = wrap_data_result(&pending, "findMany chunk")?;
                pending.clear();
                sender
                    .send(ok_partial(request_id.clone(), raw))
                    .await
                    .map_err(|_| {
                        ProtocolError::Internal(
                            "Channel closed during chunked response".to_string(),
                        )
                    })?;
            }
            std::mem::swap(&mut pending, &mut accum);
        }
    }

    // End-of-stream: at most one of `pending` (a fully-filled chunk) and
    // `accum` (partial leftover) is non-empty. Whichever holds rows last is
    // returned as the final non-partial reply; if both are non-empty, flush
    // `pending` as a partial frame first so the leftover can be the final.
    let final_chunk = if accum.is_empty() {
        pending
    } else {
        if !pending.is_empty() {
            let raw = wrap_data_result(&pending, "findMany chunk")?;
            sender
                .send(ok_partial(request_id.clone(), raw))
                .await
                .map_err(|_| {
                    ProtocolError::Internal("Channel closed during chunked response".to_string())
                })?;
        }
        accum
    };

    wrap_data_result(&final_chunk, "findMany result")
}

/// Handle `query.findMany`.
///
/// Builds a SELECT for the requested model, applying optional `where`, `orderBy`,
/// `take`, `skip`, `cursor`, `distinct`, `select`, and `include` arguments.
/// Relation includes are hydrated after the parent rows load so child ordering
/// and pagination execute on the related query before JSON serialization.
/// Returns `QueryResult { data: [...] }`. Supports transactional execution via `transactionId`.
///
/// When the client sets `chunkSize` and the dispatcher provided a response
/// channel, eligible queries (no `include`, not backward-paginated) take the
/// streaming path and emit partial replies as rows arrive from the database.
/// Other queries fall back to the buffered path so reverse / hydrate logic can
/// run against the full row set.
pub(in crate::handlers) async fn handle_find_many(
    state: &EngineState,
    request: RpcRequest,
    sender: Option<mpsc::Sender<RpcResponse>>,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: FindManyParams = parse_params(&request, "findMany")?;

    find_many_with_params(state, params, request.id, sender).await
}

/// Typed `findMany` entry point shared by [`handle_find_many`] and
/// [`handle_find_first`], so callers that already hold a [`FindManyParams`]
/// skip the JSON round-trip through a synthetic [`RpcRequest`].
async fn find_many_with_params(
    state: &EngineState,
    params: FindManyParams,
    request_id: Option<nautilus_protocol::RpcId>,
    sender: Option<mpsc::Sender<RpcResponse>>,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let chunk_size = params.chunk_size.map(|n| n.max(1));
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        crate::filter::SchemaContext::with_state(state),
    )?;

    // Streaming path is reserved for plans whose output does not need a global
    // post-fetch transformation: backward pagination flips order in memory,
    // and include hydration needs every parent row before issuing the batch
    // child query. Both cases fall back to the buffered path below.
    let streamable = chunk_size.is_some()
        && sender.is_some()
        && !query_args.backward
        && query_args.include.is_empty();

    if streamable {
        let plan = build_find_many_plan(state, model, query_args)?;
        return stream_find_many_chunked(
            state,
            plan,
            tx_id.as_deref(),
            chunk_size.expect("checked above"),
            request_id,
            sender.expect("checked above"),
        )
        .await;
    }

    let rows = execute_find_many_rows(state, model, query_args, tx_id.as_deref()).await?;

    // Buffered chunking fallback: backward / include paths still benefit from
    // wire-level chunking even though the engine had to materialise the full
    // `Vec<Row>` first.
    if let (Some(size), Some(channel)) = (chunk_size, sender) {
        let mut chunks = rows.chunks(size).peekable();

        if chunks.peek().is_some() {
            while let Some(chunk) = chunks.next() {
                let is_last = chunks.peek().is_none();
                let raw = wrap_data_result(chunk, "findMany chunk")?;
                if is_last {
                    return Ok(raw);
                }
                channel
                    .send(ok_partial(request_id.clone(), raw))
                    .await
                    .map_err(|_| {
                        ProtocolError::Internal(
                            "Channel closed during chunked response".to_string(),
                        )
                    })?;
            }
        }
    }

    wrap_data_result(&rows, "findMany result")
}

pub(in crate::handlers) async fn handle_find_many_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: FindManyParams = parse_params(&request, "findMany")?;
    execute_find_many_params(state, params).await
}

/// Handle `query.findFirst` and delegate to [`find_many_with_params`] with `take=1`.
pub(in crate::handlers) async fn handle_find_first(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: FindFirstParams = parse_params(&request, "findFirst")?;

    let find_many_params = FindManyParams {
        protocol_version: params.protocol_version,
        model: params.model,
        args: params
            .args
            .map(|mut value| {
                if let serde_json::Value::Object(ref mut map) = value {
                    map.insert("take".into(), serde_json::json!(1));
                }
                value
            })
            .or_else(|| Some(serde_json::json!({ "take": 1 }))),
        transaction_id: params.transaction_id,
        chunk_size: None,
    };

    find_many_with_params(state, find_many_params, request.id, None).await
}

/// Handle `query.findUnique`.
///
/// Builds a SELECT with the provided unique filter and `LIMIT 1`. Does not support
/// relation includes or cursor pagination.
pub(in crate::handlers) async fn handle_find_unique(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: FindUniqueParams = parse_params(&request, "findUnique")?;

    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let qualified_filter = parse_and_qualify_model_filter(
        model,
        &params.filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;
    let rows = execute_find_unique_rows(
        state,
        model,
        qualified_filter,
        &std::collections::HashSet::new(),
        tx_id.as_deref(),
    )
    .await?;
    wrap_data_result(&rows, "findUnique result")
}

/// Handle `query.findUniqueOrThrow`.
pub(in crate::handlers) async fn handle_find_unique_or_throw(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let raw = handle_find_unique(state, request).await?;
    let parsed: serde_json::Value = serde_json::from_str(raw.get())
        .map_err(|e| ProtocolError::Internal(format!("Failed to parse result: {}", e)))?;
    let is_empty = parsed
        .get("data")
        .and_then(|value| value.as_array())
        .is_none_or(|array| array.is_empty());
    if is_empty {
        return Err(ProtocolError::RecordNotFound(
            "findUniqueOrThrow: no record found matching the given filter".to_string(),
        ));
    }
    Ok(raw)
}

/// Handle `query.findFirstOrThrow`.
pub(in crate::handlers) async fn handle_find_first_or_throw(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let raw = handle_find_first(state, request).await?;
    let parsed: serde_json::Value = serde_json::from_str(raw.get())
        .map_err(|e| ProtocolError::Internal(format!("Failed to parse result: {}", e)))?;
    let is_empty = parsed
        .get("data")
        .and_then(|value| value.as_array())
        .is_none_or(|array| array.is_empty());
    if is_empty {
        return Err(ProtocolError::RecordNotFound(
            "findFirstOrThrow: no record found matching the given filter".to_string(),
        ));
    }
    Ok(raw)
}

/// Handle `query.count`.
///
/// When `take` and/or `skip` are provided, the count is performed over the paginated window.
/// Render the `EXPLAIN` form of a statement for the active backend.
///
/// The three supported backends spell the request differently, and only
/// PostgreSQL and MySQL can time a real execution: SQLite's `EXPLAIN QUERY
/// PLAN` is static, so `analyze` is accepted and has no effect there rather
/// than failing a request the client cannot make succeed.
fn explain_statement(provider: DatabaseProvider, sql: &str, analyze: bool) -> String {
    match (provider, analyze) {
        (DatabaseProvider::Postgres, false) => format!("EXPLAIN (FORMAT JSON) {sql}"),
        (DatabaseProvider::Postgres, true) => format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}"),
        (DatabaseProvider::Mysql, false) => format!("EXPLAIN FORMAT=JSON {sql}"),
        (DatabaseProvider::Mysql, true) => format!("EXPLAIN ANALYZE {sql}"),
        (DatabaseProvider::Sqlite, _) => format!("EXPLAIN QUERY PLAN {sql}"),
    }
}

fn row_to_json_object(row: Row) -> JsonValue {
    JsonValue::Object(
        row.into_columns_iter()
            .map(|(name, value)| (name.to_string(), value.to_json_plain()))
            .collect(),
    )
}

/// Handle `query.explain`.
///
/// Builds the same plan a `findMany` with these arguments would run — including
/// the rendered placeholders and their bound values — and hands the statement to
/// the database's own `EXPLAIN`. Include hydration is not part of the plan: each
/// relation is a separate statement, so explaining the parent query alone keeps
/// the result a single plan the caller can read.
pub(in crate::handlers) async fn handle_explain(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: ExplainParams = parse_params(&request, "explain")?;
    check_protocol_version(params.protocol_version)?;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        crate::filter::SchemaContext::with_state(state),
    )?;

    let result = execute_explain(
        state,
        model,
        query_args,
        params.analyze,
        params.transaction_id.as_deref(),
    )
    .await?;

    let body = sonic_rs::to_string(&result).map_err(|e| {
        ProtocolError::Internal(format!("Failed to serialize explain result: {}", e))
    })?;
    wrap_result(body, "explain result")
}

/// Typed `explain` entry point for embedded callers holding
/// [`nautilus_core::FindManyArgs`], mirroring [`execute_find_many_typed`].
pub(in crate::handlers) async fn execute_explain_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindManyArgs,
    analyze: bool,
    transaction_id: Option<&str>,
) -> Result<nautilus_protocol::ExplainResult, ProtocolError> {
    let model = get_model_or_error(state, model_name)?;
    let metadata = state.model_metadata(model);
    let query_args = QueryArgs::from_find_many_args(args, metadata.field_types())?;

    execute_explain(state, model, query_args, analyze, transaction_id).await
}

async fn execute_explain(
    state: &EngineState,
    model: &ModelIr,
    query_args: QueryArgs,
    analyze: bool,
    tx_id: Option<&str>,
) -> Result<nautilus_protocol::ExplainResult, ProtocolError> {
    let plan = build_find_many_plan(state, model, query_args)?;
    let explain_sql = Sql {
        text: explain_statement(state.provider(), &plan.sql.text, analyze),
        params: plan.sql.params.clone(),
    };

    let rows = state
        .execute_query_on(&explain_sql, "Explain", tx_id)
        .await?;

    Ok(nautilus_protocol::ExplainResult {
        sql: plan.sql.text,
        params: plan
            .sql
            .params
            .iter()
            .map(nautilus_core::Value::to_json_plain)
            .collect(),
        plan: rows.into_iter().map(row_to_json_object).collect(),
    })
}

pub(in crate::handlers) async fn handle_count(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CountParams = parse_params(&request, "count")?;

    let count = execute_count_params(state, params).await?;
    wrap_count_result(count, "count result")
}

async fn execute_count_params(
    state: &EngineState,
    params: CountParams,
) -> Result<i64, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let query_args = QueryArgs::parse_typed(params.args, metadata.field_types())?;
    let qualified_filter = qualify_model_filter(model, metadata.logical_to_db(), query_args.filter);

    let has_pagination = query_args.take.is_some() || query_args.skip.is_some();

    let sql: Sql = if has_pagination {
        let mut inner = Select::from_table(&model.db_name)
            .with_capacity(SelectCapacity {
                items: 1,
                ..SelectCapacity::default()
            })
            .item(SelectItem::computed(Expr::param(Value::I32(1)), "_1"));
        if let Some(filter) = qualified_filter {
            inner = inner.filter(filter);
        }
        if let Some(take) = query_args.take {
            inner = inner.take(take);
        }
        if let Some(skip) = query_args.skip {
            inner = inner.skip(skip);
        }
        let inner_built = inner.build().map_err(|e| {
            ProtocolError::QueryPlanning(format!("Failed to build inner count query: {}", e))
        })?;
        let inner_rendered = state
            .dialect
            .render_select_owned(inner_built)
            .map_err(|e| {
                ProtocolError::QueryPlanning(format!("Failed to render inner count query: {}", e))
            })?;
        Sql {
            text: format!("SELECT COUNT(*) FROM ({}) AS _cntq", inner_rendered.text),
            params: inner_rendered.params,
        }
    } else {
        let mut builder = Select::from_table(&model.db_name)
            .with_capacity(SelectCapacity {
                items: 1,
                ..SelectCapacity::default()
            })
            .item(SelectItem::computed(
                Expr::function_call("COUNT", vec![Expr::star()]),
                "count",
            ));
        if let Some(filter) = qualified_filter {
            builder = builder.filter(filter);
        }
        let select = builder.build().map_err(|e| {
            ProtocolError::QueryPlanning(format!("Failed to build count query: {}", e))
        })?;
        state.dialect.render_select_owned(select).map_err(|e| {
            ProtocolError::QueryPlanning(format!("Failed to render count query: {}", e))
        })?
    };

    let rows = state
        .execute_query_on(&sql, "Count", tx_id.as_deref())
        .await?;
    let count: i64 = rows
        .first()
        .and_then(|row| row.get_by_pos(0))
        .map(|value| match value {
            Value::I64(n) => *n,
            Value::I32(n) => *n as i64,
            _ => 0,
        })
        .unwrap_or(0);

    Ok(count)
}

pub(in crate::handlers) async fn handle_count_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<i64, ProtocolError> {
    let params: CountParams = parse_params(&request, "count")?;
    execute_count_params(state, params).await
}

pub(in crate::handlers) async fn handle_count_typed(
    state: &EngineState,
    params: CountParams,
) -> Result<i64, ProtocolError> {
    execute_count_params(state, params).await
}
