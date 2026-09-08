//! Read methods: the four `find` shapes, their `EXPLAIN` counterpart and the
//! row payload they all answer with.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const QUERY_FIND_MANY: &str = "query.findMany";
pub const QUERY_FIND_FIRST: &str = "query.findFirst";
pub const QUERY_FIND_UNIQUE: &str = "query.findUnique";
pub const QUERY_FIND_UNIQUE_OR_THROW: &str = "query.findUniqueOrThrow";
pub const QUERY_FIND_FIRST_OR_THROW: &str = "query.findFirstOrThrow";

/// Render the SQL for a read operation and ask the database to explain it.
pub const QUERY_EXPLAIN: &str = "query.explain";

/// Find many request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindManyParams {
    /// Protocol version (required in all requests).
    pub protocol_version: u32,

    /// Model name (e.g., "User", "Post").
    pub model: String,

    /// Query arguments (filters, ordering, pagination, etc.).
    /// Structure is flexible and parsed by the engine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,

    /// Optional transaction ID — if present, this query runs inside the given transaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,

    /// Optional chunk size for streaming large result sets.
    /// When set, the engine emits multiple partial responses of at most `chunk_size` rows each.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<usize>,
}

/// Find first request parameters (same shape as FindMany — optional full args).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindFirstParams {
    pub protocol_version: u32,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

/// Find unique request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindUniqueParams {
    pub protocol_version: u32,
    pub model: String,
    pub filter: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

/// Find unique or throw request parameters (same shape as FindUnique).
pub type FindUniqueOrThrowParams = FindUniqueParams;

/// Find first or throw request parameters (same shape as FindFirst).
pub type FindFirstOrThrowParams = FindFirstParams;

/// Explain request parameters.
///
/// Renders the SQL the engine would run for a `findMany` with these arguments
/// and hands it to the database's `EXPLAIN`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainParams {
    pub protocol_version: u32,
    pub model: String,
    /// Same argument shape as [`FindManyParams::args`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Run the statement to collect real timings (`EXPLAIN ANALYZE`).
    ///
    /// This *executes* the query. On a mutation that would be a side effect;
    /// explain only covers reads, so the cost is the read itself.
    #[serde(default)]
    pub analyze: bool,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

/// Explain result: the rendered statement plus the database's own plan output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResult {
    /// SQL text with placeholders, exactly as the engine would execute it.
    pub sql: String,
    /// Bound parameter values, in placeholder order.
    pub params: Vec<Value>,
    /// Rows returned by `EXPLAIN`, one JSON object per plan line.
    pub plan: Vec<Value>,
}

/// Query result containing data rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Result data as JSON objects.
    pub data: Vec<Value>,
}
