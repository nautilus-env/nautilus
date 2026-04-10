use super::common::wrap_data_result;
use super::*;

/// Handle `query.rawQuery`.
pub(super) async fn handle_raw_query(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    use nautilus_protocol::RawQueryParams;

    let params: RawQueryParams = serde_json::from_value(request.params)
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid rawQuery params: {}", e)))?;

    check_protocol_version(params.protocol_version)?;

    let sql = nautilus_dialect::Sql {
        text: params.sql,
        params: vec![],
    };

    let rows = state
        .execute_direct_query_on(&sql, "rawQuery", params.transaction_id.as_deref())
        .await?;
    wrap_data_result(&rows, "rawQuery result")
}

/// Handle `query.rawStmtQuery`.
pub(super) async fn handle_raw_stmt_query(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    use nautilus_protocol::RawStmtQueryParams;

    let params: RawStmtQueryParams = serde_json::from_value(request.params)
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid rawStmtQuery params: {}", e)))?;

    check_protocol_version(params.protocol_version)?;

    let values: Vec<nautilus_core::Value> = params
        .params
        .iter()
        .map(json_to_value)
        .collect::<Result<_, _>>()?;

    let sql = nautilus_dialect::Sql {
        text: params.sql,
        params: values,
    };

    let rows = state
        .execute_direct_query_on(&sql, "rawStmtQuery", params.transaction_id.as_deref())
        .await?;
    wrap_data_result(&rows, "rawStmtQuery result")
}
