//! Write methods: the row-returning mutations, the count-only many-variants
//! and the result they share.

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_true() -> bool {
    true
}

pub const QUERY_CREATE: &str = "query.create";
pub const QUERY_CREATE_MANY: &str = "query.createMany";
pub const QUERY_UPDATE: &str = "query.update";
/// Update every row matching a filter and return only the affected-row count.
pub const QUERY_UPDATE_MANY: &str = "query.updateMany";
pub const QUERY_DELETE: &str = "query.delete";
/// Delete every row matching a filter and return only the affected-row count.
pub const QUERY_DELETE_MANY: &str = "query.deleteMany";
/// Insert a row, or update the conflicting one, in a single atomic statement.
pub const QUERY_UPSERT: &str = "query.upsert";

/// Create request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateParams {
    pub protocol_version: u32,
    pub model: String,
    pub data: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Whether to return the created row(s). Defaults to `true`.
    #[serde(default = "default_true")]
    pub return_data: bool,
}

/// Create many request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateManyParams {
    pub protocol_version: u32,
    pub model: String,
    pub data: Vec<Value>,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Whether to return the created row(s). Defaults to `true`.
    #[serde(default = "default_true")]
    pub return_data: bool,
}

/// Update request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateParams {
    pub protocol_version: u32,
    pub model: String,
    pub filter: Value,
    pub data: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Whether to return the updated row(s). Defaults to `true`.
    #[serde(default = "default_true")]
    pub return_data: bool,
}

/// Delete request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteParams {
    pub protocol_version: u32,
    pub model: String,
    pub filter: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Whether to return the deleted row(s). Defaults to `true`.
    #[serde(default = "default_true")]
    pub return_data: bool,
}

/// Upsert request parameters.
///
/// `filter` must select exactly the columns of one unique constraint (or the
/// primary key) of the model: those columns become the conflict target of the
/// underlying `INSERT ... ON CONFLICT` / `ON DUPLICATE KEY UPDATE` statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertParams {
    pub protocol_version: u32,
    pub model: String,
    /// Unique filter identifying the row to update on conflict.
    pub filter: Value,
    /// Fields written when the row does not exist yet.
    pub create: Value,
    /// Fields written when the row already exists. Empty means "leave it as is".
    pub update: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Whether to return the inserted/updated row. Defaults to `true`.
    #[serde(default = "default_true")]
    pub return_data: bool,
}

/// Update-many request parameters.
///
/// Unlike [`UpdateParams`], no `RETURNING` clause is ever emitted: the result
/// carries the affected-row count only, so the statement stays a single
/// round-trip regardless of how many rows it touches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateManyParams {
    pub protocol_version: u32,
    pub model: String,
    /// Rows to update. An empty filter updates every row of the model.
    pub filter: Value,
    pub data: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Accepted and ignored: a client that offers one `deleteMany` /
    /// `updateMany` entry point builds a single payload and picks the RPC by
    /// this flag, so it reaches the count-only method too. The many-variants
    /// always answer with a count.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub return_data: bool,
}

/// Delete-many request parameters.
///
/// Like [`UpdateManyParams`], the result carries the affected-row count only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteManyParams {
    pub protocol_version: u32,
    pub model: String,
    /// Rows to delete. An empty filter deletes every row of the model.
    pub filter: Value,
    /// Optional transaction ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Accepted and ignored: a client that offers one `deleteMany` /
    /// `updateMany` entry point builds a single payload and picks the RPC by
    /// this flag, so it reaches the count-only method too. The many-variants
    /// always answer with a count.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub return_data: bool,
}

/// Mutation result with count of affected rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    /// Number of rows affected.
    pub count: usize,

    /// Optional returning data for mutations that support RETURNING.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<Value>>,
}
