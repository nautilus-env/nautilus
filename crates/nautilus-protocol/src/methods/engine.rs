//! Engine-level methods: the handshake that opens a session, the runtime
//! counters it exposes, and the cancellation of an in-flight request.

use crate::wire::RpcId;
use serde::{Deserialize, Serialize};

pub const ENGINE_HANDSHAKE: &str = "engine.handshake";

/// Snapshot of the engine's runtime counters.
pub const ENGINE_METRICS: &str = "engine.metrics";
/// Cancel an in-flight request by id.
///
/// This aborts the engine-side task and stops the response from being sent; it
/// does **not** reach the database, so a statement already running there keeps
/// running and keeps holding its connection until it finishes. Bounding that
/// requires a server-side limit — `statement_timeout` on PostgreSQL,
/// `max_execution_time` on MySQL — which the engine's `--statement-timeout-ms`
/// flag installs on every pooled connection.
pub const REQUEST_CANCEL: &str = "request.cancel";

/// Handshake request parameters.
///
/// The handshake must be the first request sent by a client to validate
/// protocol compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeParams {
    /// Protocol version the client is using.
    pub protocol_version: u32,

    /// Optional client name (e.g., "nautilus-js").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,

    /// Optional client version (e.g., "0.1.0").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
}

/// Handshake response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResult {
    /// Engine version.
    pub engine_version: String,

    /// Protocol version the engine supports.
    pub protocol_version: u32,
}

/// Cancel-request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCancelParams {
    /// Protocol version (required in all requests).
    pub protocol_version: u32,

    /// Identifier of the in-flight request to cancel.
    pub request_id: RpcId,
}

/// Cancel-request result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCancelResult {
    /// True when a live request was found and its engine task aborted.
    ///
    /// Says nothing about the database: see [`REQUEST_CANCEL`].
    pub cancelled: bool,
}

/// Engine metrics request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineMetricsParams {
    pub protocol_version: u32,
    /// Reset the cumulative counters after reading them.
    #[serde(default)]
    pub reset: bool,
}

/// Plan-cache counters for one cache section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCacheSectionMetrics {
    /// Entries currently held.
    pub entries: usize,
    /// Lookups that found a cached plan.
    pub hits: u64,
    /// Lookups that had to render a plan.
    pub misses: u64,
    /// Entries dropped to keep the section under its cap.
    pub evictions: u64,
}

/// Plan-cache counters, per section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCacheMetrics {
    /// Maximum entries a section holds before evicting.
    pub capacity: usize,
    /// `findUnique` section.
    pub find_unique: PlanCacheSectionMetrics,
    /// `findMany` / `findFirst` section.
    pub find_many: PlanCacheSectionMetrics,
}

/// Connection-pool counters, as reported by the driver.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolMetrics {
    /// Connections currently held by the pool (idle plus in use).
    pub size: u32,
    /// Connections currently idle.
    pub idle: usize,
}

/// Per-method request counters and cumulative latency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodMetrics {
    /// JSON-RPC method name.
    pub method: String,
    /// Requests dispatched.
    pub calls: u64,
    /// Requests that returned an error.
    pub errors: u64,
    /// Total time spent in the handler.
    pub total_ms: u64,
    /// Slowest single call observed.
    pub max_ms: u64,
}

/// Engine metrics snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineMetricsResult {
    /// Seconds since the engine state was built.
    pub uptime_seconds: u64,
    /// Read-plan cache counters.
    pub plan_cache: PlanCacheMetrics,
    /// Connection-pool counters.
    pub pool: PoolMetrics,
    /// Interactive transactions currently open.
    pub active_transactions: usize,
    /// Per-method counters, sorted by method name.
    pub methods: Vec<MethodMetrics>,
}
