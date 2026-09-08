//! Aggregate methods: counting rows, grouping them, and computing aggregates
//! over the whole filtered set.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const QUERY_COUNT: &str = "query.count";
/// Group records and compute aggregates (COUNT, AVG, SUM, MIN, MAX).
pub const QUERY_GROUP_BY: &str = "query.groupBy";
/// Compute aggregates over the whole filtered set, without grouping.
pub const QUERY_AGGREGATE: &str = "query.aggregate";

/// Count request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountParams {
    pub protocol_version: u32,
    pub model: String,
    /// Optional query arguments (where, take, skip).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

/// Group-by request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupByParams {
    pub protocol_version: u32,
    pub model: String,
    /// Query arguments: by, where, having, take, skip, orderBy, count, avg, sum, min, max.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

/// Aggregate request parameters.
///
/// Same aggregate arguments as [`GroupByParams`] minus `by` and `having`:
/// without a grouping key there is one result row covering the whole filtered
/// set, so there is nothing to group or filter groups by.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AggregateParams {
    pub protocol_version: u32,
    pub model: String,
    /// Query arguments: where, count, avg, sum, min, max.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}
