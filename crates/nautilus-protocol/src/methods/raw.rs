//! Raw SQL methods: the statement the client writes itself, with or without
//! bound parameters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Method name for executing a raw SQL query (no parameter binding).
pub const QUERY_RAW: &str = "query.rawQuery";
/// Method name for executing a raw prepared-statement query (with bound params).
pub const QUERY_RAW_STMT: &str = "query.rawStmtQuery";

/// Raw SQL query request parameters.
///
/// Execute the SQL string as-is against the database and return the result rows
/// as generic JSON objects.  No parameter binding is performed — embed literal
/// values directly in the SQL string or use [`RawStmtQueryParams`] instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawQueryParams {
    /// Protocol version (required in all requests).
    pub protocol_version: u32,
    /// Raw SQL string to execute.
    pub sql: String,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

/// Raw prepared-statement query request parameters.
///
/// Execute the SQL string with bound parameters and return the result rows as
/// generic JSON objects.  Use `$1`, `$2`, … (PostgreSQL) or `?` (MySQL /
/// SQLite) as placeholders; parameters are bound in the order they appear in
/// the `params` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawStmtQueryParams {
    /// Protocol version (required in all requests).
    pub protocol_version: u32,
    /// Raw SQL string containing parameter placeholders.
    pub sql: String,
    /// Ordered list of parameter values to bind.
    #[serde(default)]
    pub params: Vec<Value>,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}
