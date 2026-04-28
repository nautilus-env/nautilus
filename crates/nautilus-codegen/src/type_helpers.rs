//! Type mapping helpers for code generation.

use nautilus_schema::ir::DefaultValue;
use nautilus_schema::ir::{FieldIr, ResolvedFieldType, ScalarType};

use crate::extension_types::ExtensionRegistry;

/// Convert a `ScalarType` to its Rust type string. When the backing extension
/// for the scalar is declared in the schema, the generated wrapper path is
/// returned; otherwise the native scalar mapping is used.
pub(crate) fn scalar_to_rust_type(scalar: &ScalarType, extensions: &ExtensionRegistry) -> String {
    extensions
        .type_for_scalar(scalar)
        .map(|ty| ty.rust_type_path())
        .unwrap_or_else(|| scalar.rust_type().to_string())
}

/// Get the base Rust type for a field without optional / nullability wrappers.
pub(crate) fn field_to_rust_base_type(field: &FieldIr, extensions: &ExtensionRegistry) -> String {
    let base_type = match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_rust_type(scalar, extensions),
        ResolvedFieldType::Enum { enum_name } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => rel.target_model.clone(),
    };

    if field.is_array && !matches!(field.field_type, ResolvedFieldType::Relation(_)) {
        format!("Vec<{}>", base_type)
    } else {
        base_type
    }
}

/// Get the Rust type for a field, including `Option` / `Box` wrappers.
pub fn field_to_rust_type(field: &FieldIr, extensions: &ExtensionRegistry) -> String {
    let base_type = field_to_rust_base_type(field, extensions);

    if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
        return if field.is_array {
            format!("Vec<{}>", base_type)
        } else {
            format!("Option<Box<{}>>", base_type)
        };
    }

    if !field.is_required && !field.is_array {
        format!("Option<{}>", base_type)
    } else {
        base_type
    }
}

/// Get the Rust type used by `SUM()` outputs for a numeric field.
pub(crate) fn field_to_rust_sum_type(field: &FieldIr, extensions: &ExtensionRegistry) -> String {
    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Int | ScalarType::BigInt) => "i64".to_string(),
        ResolvedFieldType::Scalar(ScalarType::Float) => "f64".to_string(),
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => {
            "rust_decimal::Decimal".to_string()
        }
        _ => field_to_rust_base_type(field, extensions),
    }
}

/// Get the Rust type used by `AVG()` outputs for a numeric field.
pub(crate) fn field_to_rust_avg_type(field: &FieldIr) -> String {
    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => {
            "rust_decimal::Decimal".to_string()
        }
        _ => "f64".to_string(),
    }
}

/// Check if a field should be auto-generated (excluded from create builders).
pub fn is_auto_generated(field: &FieldIr) -> bool {
    if field.computed.is_some() {
        return true;
    }
    if let Some(default) = &field.default_value {
        matches!(
            default,
            DefaultValue::Function(func) if func.name == "autoincrement" || func.name == "uuid"
        )
    } else {
        false
    }
}
