//! The methods that answer about the engine itself rather than about a model:
//! the handshake, the runtime counters and schema validation.

use nautilus_protocol::{
    check_protocol_version, EngineMetricsParams, HandshakeParams, HandshakeResult, ProtocolError,
    RpcRequest, SchemaValidateParams, SchemaValidateResult, PROTOCOL_VERSION,
};
use nautilus_schema::{analyze, Severity};

use super::request::parse_params;
use crate::state::EngineState;

/// Handle engine.handshake
pub(super) async fn handle_handshake(
    _state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: HandshakeParams = parse_params(&request, "handshake")?;

    check_protocol_version(params.protocol_version)?;

    let result = HandshakeResult {
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
    };

    serialize_result(&result, "handshake result")
}

/// Handle `engine.metrics`.
pub(super) async fn handle_engine_metrics(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: EngineMetricsParams = parse_params(&request, "engine.metrics")?;

    check_protocol_version(params.protocol_version)?;

    serialize_result(
        &state.metrics_snapshot(params.reset).await,
        "engine.metrics result",
    )
}

pub(super) async fn handle_schema_validate(
    _state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: SchemaValidateParams = parse_params(&request, "schema.validate")?;

    check_protocol_version(params.protocol_version)?;

    let analysis = analyze(&params.schema);
    let errors: Vec<String> = analysis
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message)
        .collect();

    let result = SchemaValidateResult {
        valid: errors.is_empty(),
        errors: (!errors.is_empty()).then_some(errors),
    };

    serialize_result(&result, "schema.validate result")
}

fn serialize_result<T: serde::Serialize>(
    result: &T,
    context: &str,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let s = sonic_rs::to_string(result)
        .map_err(|e| ProtocolError::Internal(format!("Failed to serialize {context}: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap {context}: {}", e)))
}
