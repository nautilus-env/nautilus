//! `count`.
//!
//! Without pagination the count is a plain aggregate; with `take` or `skip` it
//! has to count the rows of the window, which the window itself can only limit
//! inside a subquery.

use nautilus_core::{Expr, Select, SelectCapacity, SelectItem, Value};
use nautilus_dialect::Sql;
use nautilus_protocol::{check_protocol_version, CountParams, ProtocolError, RpcRequest};
use serde_json::value::RawValue;

use crate::filter::{QueryArgs, SchemaContext};
use crate::handlers::crud::common::{qualify_model_filter, wrap_count_result};
use crate::handlers::{get_model_or_error, parse_params};
use crate::metadata::model_table;
use crate::state::EngineState;

/// Handle `query.count`.
///
/// When `take` and/or `skip` are provided, the count is performed over the paginated window.
pub(in crate::handlers) async fn handle_count(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<RawValue>, ProtocolError> {
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
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        SchemaContext::with_state(state),
    )?;
    let qualified_filter = qualify_model_filter(model, metadata.logical_to_db(), query_args.filter);

    let has_pagination = query_args.take.is_some() || query_args.skip.is_some();

    let sql: Sql = if has_pagination {
        let mut inner = Select::from_table(model_table(model))
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
        let mut builder = Select::from_table(model_table(model))
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
