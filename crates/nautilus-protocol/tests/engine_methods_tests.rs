//! Contracts of the engine-level methods: handshake, metrics and cancellation.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_engine_method_names() {
    assert_eq!(ENGINE_HANDSHAKE, "engine.handshake");
    assert_eq!(ENGINE_METRICS, "engine.metrics");
    assert_eq!(REQUEST_CANCEL, "request.cancel");
}

#[test]
fn test_handshake_params_serialization() {
    let params = HandshakeParams {
        protocol_version: 1,
        client_name: Some("nautilus-js".to_string()),
        client_version: Some("0.1.0".to_string()),
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["clientName"], "nautilus-js");
    assert_eq!(json["clientVersion"], "0.1.0");

    let parsed: HandshakeParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.protocol_version, 1);
    assert_eq!(parsed.client_name, Some("nautilus-js".to_string()));
}

#[test]
fn test_handshake_result_serialization() {
    let result = HandshakeResult {
        engine_version: "0.1.0".to_string(),
        protocol_version: 1,
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["engineVersion"], "0.1.0");
    assert_eq!(json["protocolVersion"], 1);
}

#[test]
fn test_request_cancel_carries_the_request_id_of_the_wire() {
    let params = RequestCancelParams {
        protocol_version: 1,
        request_id: RpcId::Number(7),
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["requestId"], 7);

    let parsed: RequestCancelParams = serde_json::from_value(json).unwrap();
    assert!(matches!(parsed.request_id, RpcId::Number(7)));

    let result: RequestCancelResult = serde_json::from_value(json!({"cancelled": true})).unwrap();
    assert!(result.cancelled);
}

#[test]
fn test_engine_metrics_params_default_to_reading_without_reset() {
    let params: EngineMetricsParams =
        serde_json::from_value(json!({"protocolVersion": 1})).unwrap();
    assert!(!params.reset);
}

#[test]
fn test_engine_metrics_result_serialization() {
    let result = EngineMetricsResult {
        uptime_seconds: 12,
        plan_cache: PlanCacheMetrics {
            capacity: 128,
            find_unique: PlanCacheSectionMetrics {
                entries: 1,
                hits: 2,
                misses: 3,
                evictions: 4,
            },
            find_many: PlanCacheSectionMetrics::default(),
        },
        pool: PoolMetrics { size: 5, idle: 4 },
        active_transactions: 1,
        methods: vec![MethodMetrics {
            method: QUERY_FIND_MANY.to_string(),
            calls: 9,
            errors: 1,
            total_ms: 30,
            max_ms: 12,
        }],
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["uptimeSeconds"], 12);
    assert_eq!(json["planCache"]["capacity"], 128);
    assert_eq!(json["planCache"]["findUnique"]["evictions"], 4);
    assert_eq!(json["planCache"]["findMany"]["hits"], 0);
    assert_eq!(json["pool"]["idle"], 4);
    assert_eq!(json["activeTransactions"], 1);
    assert_eq!(json["methods"][0]["method"], "query.findMany");
    assert_eq!(json["methods"][0]["maxMs"], 12);
}
