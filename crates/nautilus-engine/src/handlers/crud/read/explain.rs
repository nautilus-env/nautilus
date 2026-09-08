//! `explain`.
//!
//! The statement handed to the database is the one a `findMany` with the same
//! arguments would run, placeholders and bound values included. Include
//! hydration stays out of it: each relation is a separate statement, so
//! explaining the parent query alone keeps the result a single readable plan.

use nautilus_connector::Row;
use nautilus_dialect::Sql;
use nautilus_migrate::DatabaseProvider;
use nautilus_protocol::{
    check_protocol_version, ExplainParams, ExplainResult, ProtocolError, RpcRequest,
};
use nautilus_schema::ir::ModelIr;
use serde_json::value::RawValue;
use serde_json::Value as JsonValue;

use super::plan::build_find_many_plan;
use crate::filter::{QueryArgs, SchemaContext};
use crate::handlers::crud::common::wrap_result;
use crate::handlers::{get_model_or_error, parse_params};
use crate::state::EngineState;

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
) -> Result<Box<RawValue>, ProtocolError> {
    let params: ExplainParams = parse_params(&request, "explain")?;
    check_protocol_version(params.protocol_version)?;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        SchemaContext::with_state(state),
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
/// [`nautilus_core::FindManyArgs`], mirroring
/// [`super::find_many::execute_find_many_typed`].
pub(in crate::handlers) async fn execute_explain_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindManyArgs,
    analyze: bool,
    transaction_id: Option<&str>,
) -> Result<ExplainResult, ProtocolError> {
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
) -> Result<ExplainResult, ProtocolError> {
    let plan = build_find_many_plan(state, model, query_args)?;
    let explain_sql = Sql {
        text: explain_statement(state.provider(), &plan.sql.text, analyze),
        params: plan.sql.params.clone(),
    };

    let rows = state
        .execute_query_on(&explain_sql, "Explain", tx_id)
        .await?;

    Ok(ExplainResult {
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
