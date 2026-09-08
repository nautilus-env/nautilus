//! Point reads: `findUnique` and the row lookups the write paths depend on.
//!
//! Both shapes render the same statement — every projected scalar column of a
//! single table behind a qualified predicate — so they share one builder. The
//! lookups differ only in their limit: the mutation paths read back the rows a
//! statement touched, and the nested-write paths resolve the targets of
//! `connect` and friends before narrowing an operation to them.

use std::collections::HashSet;
use std::sync::Arc;

use nautilus_connector::Row;
use nautilus_core::{Expr, Select, SelectCapacity, SelectItem};
use nautilus_dialect::Sql;
use nautilus_protocol::{check_protocol_version, FindUniqueParams, ProtocolError, RpcRequest};
use nautilus_schema::ir::ModelIr;
use serde_json::value::RawValue;
use serde_json::Value as JsonValue;

use super::find_many::execute_find_many_typed;
use super::plan::find_unique_plan_key;
use crate::conversion::{normalize_rows_with_hints, ValueHint};
use crate::filter::qualify_filter_columns;
use crate::handlers::crud::common::{
    ensure_unique_filter, parse_and_qualify_model_filter, wrap_data_result,
};
use crate::handlers::{get_model_or_error, parse_params};
use crate::plan_cache::{extract_simple_eq_filter, CachedReadPlan};
use crate::state::EngineState;

/// Render a single-table SELECT over `model` and the hints to decode its rows.
///
/// An empty `selected_fields` projects every scalar column; otherwise only the
/// named logical fields, plus the primary key the callers rely on to identify
/// a row whether or not they asked for it.
fn build_scalar_select(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Option<Expr>,
    selected_fields: &HashSet<&str>,
    limit: Option<i32>,
) -> Result<(Sql, Vec<Option<ValueHint>>), ProtocolError> {
    let metadata = state.model_metadata(model);
    let pk_fields = metadata.primary_key_fields();

    let mut builder =
        Select::from_table(crate::metadata::model_table(model)).with_capacity(SelectCapacity {
            items: metadata.scalar_fields().len(),
            ..SelectCapacity::default()
        });
    let mut row_hints = Vec::with_capacity(metadata.scalar_fields().len());

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

    Ok((sql, row_hints))
}

pub(in crate::handlers::crud) fn build_find_unique_sql(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Expr,
    selected_fields: &HashSet<&str>,
) -> Result<(Sql, Vec<Option<ValueHint>>), ProtocolError> {
    build_scalar_select(
        state,
        model,
        Some(qualified_filter),
        selected_fields,
        Some(1),
    )
}

async fn execute_find_unique_rows(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Expr,
    selected_fields: &HashSet<&str>,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let (sql, row_hints) = build_find_unique_sql(state, model, qualified_filter, selected_fields)?;
    normalize_rows_with_hints(
        state.execute_query_on(&sql, "Query", tx_id).await?,
        &row_hints,
    )
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
    let selected_fields: HashSet<&str> = args
        .select
        .iter()
        .filter_map(|(field, enabled)| enabled.then_some(field.as_str()))
        .collect();

    // Plan-cache fast path: only available when the filter is a flat AND chain
    // of `Column = Param` predicates so we can replay the rendered SQL by
    // re-binding parameter values without rebuilding the AST.
    if let Some(shape) = extract_simple_eq_filter(&args.where_) {
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
            Arc::new(CachedReadPlan {
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

/// Handle `query.findUnique`.
///
/// Builds a SELECT with the provided unique filter and `LIMIT 1`. Does not support
/// relation includes or cursor pagination.
pub(in crate::handlers) async fn handle_find_unique(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<RawValue>, ProtocolError> {
    let params: FindUniqueParams = parse_params(&request, "findUnique")?;

    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    ensure_unique_filter(model, &params.filter)?;
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
        &HashSet::new(),
        tx_id.as_deref(),
    )
    .await?;
    wrap_data_result(&rows, "findUnique result")
}

/// Handle `query.findUniqueOrThrow`.
pub(in crate::handlers) async fn handle_find_unique_or_throw(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<RawValue>, ProtocolError> {
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

/// Load up to `limit` rows of `model` matching `filter`, projecting every
/// scalar column.
///
/// The nested-write paths use this to resolve `connect` targets and to find the
/// row a nested operation hangs off, so the projection has to cover whatever
/// key the relation references, not just the primary key.
pub(in crate::handlers::crud) async fn find_rows_by_filter(
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
pub(in crate::handlers::crud) async fn find_rows_by_expr(
    state: &EngineState,
    model: &ModelIr,
    qualified_filter: Option<Expr>,
    limit: Option<i32>,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let (sql, row_hints) =
        build_scalar_select(state, model, qualified_filter, &HashSet::new(), limit)?;

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
pub(in crate::handlers::crud) async fn find_all_rows_by_filter(
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
pub(in crate::handlers::crud) async fn find_one_row(
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
