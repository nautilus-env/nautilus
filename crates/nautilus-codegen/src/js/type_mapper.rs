//! Type mapping from Nautilus schema types to TypeScript types.
//!
//! Language-independent logic (auto-generation detection, filter operator
//! construction) lives in [`crate::js::JsBackend`]. This module keeps
//! only the TypeScript-specific helpers: full-field type composition, base-type
//! extraction, and default-value formatting.

use nautilus_schema::ir::{FieldIr, ScalarType};
use std::collections::HashMap;

use crate::backend::{FilterOperator, LanguageBackend};
use crate::js::JsBackend;

/// Maps a Nautilus scalar type to its TypeScript primitive string.
pub fn scalar_to_ts_type(scalar: &ScalarType) -> &'static str {
    JsBackend.scalar_to_type(scalar)
}

/// Builds the full TypeScript type for a field, including `T | null` and `T[]` wrappers.
pub fn field_to_ts_type(
    field: &FieldIr,
    enums: &HashMap<String, nautilus_schema::ir::EnumIr>,
) -> String {
    let base = get_base_ts_type(field, enums);
    JsBackend.wrap_field_type(field, base)
}

/// Returns the bare base TypeScript type without wrappers (e.g. `string`, `Date`).
pub fn get_base_ts_type(
    field: &FieldIr,
    enums: &HashMap<String, nautilus_schema::ir::EnumIr>,
) -> String {
    JsBackend.get_base_type(field, enums)
}

/// Returns `true` for fields whose values are supplied automatically by the
/// database (`autoincrement()`, `uuid()`, `now()`).
pub fn is_auto_generated(field: &FieldIr) -> bool {
    JsBackend.is_auto_generated(field)
}

/// Returns the TypeScript default value expression for a field, or `None`
/// if the caller should omit a default entirely.
pub fn get_ts_default_value(field: &FieldIr) -> Option<String> {
    JsBackend.get_default_value(field)
}

/// Returns filter operators for a field, considering its resolved type.
pub fn get_filter_operators_for_field(
    field: &FieldIr,
    enums: &HashMap<String, nautilus_schema::ir::EnumIr>,
) -> Vec<FilterOperator> {
    JsBackend.get_filter_operators_for_field(field, enums)
}
