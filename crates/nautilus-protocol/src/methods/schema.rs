//! Schema methods: validating a schema source without touching the database.

use serde::{Deserialize, Serialize};

/// Method name for schema validation.
pub const SCHEMA_VALIDATE: &str = "schema.validate";

/// Schema validation request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaValidateParams {
    pub protocol_version: u32,
    pub schema: String,
}

/// Schema validation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaValidateResult {
    /// Whether the schema is valid.
    pub valid: bool,

    /// Validation errors if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<String>>,
}
