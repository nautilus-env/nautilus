//! Rust ordering methods for paths inside composite columns.

use crate::extension_types::ExtensionRegistry;
use crate::type_helpers::{
    is_orderable_composite_field, json_path_cast_variant, scalar_to_rust_type,
};
use heck::ToSnakeCase;
use nautilus_schema::ir::{CompositeFieldIr, ModelIr, ResolvedFieldType, SchemaIr};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
pub(super) struct NestedOrderByFieldContext {
    method_name: String,
    path: String,
    parent_db_name: String,
    field_db_name: String,
    json_key: String,
    json_cast: String,
    rust_type: String,
}

fn composite_field_rust_type(
    field: &CompositeFieldIr,
    extensions: &ExtensionRegistry,
) -> Option<String> {
    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => Some(scalar_to_rust_type(scalar, extensions)),
        ResolvedFieldType::Enum { enum_name, .. } => Some(enum_name.clone()),
        _ => None,
    }
}

pub(super) fn build_nested_order_by_fields(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
    reserved_methods: &HashSet<String>,
) -> Vec<NestedOrderByFieldContext> {
    let mut fields = Vec::new();
    let mut used_methods = reserved_methods.clone();

    for parent in model.scalar_fields() {
        if parent.is_array {
            continue;
        }

        let ResolvedFieldType::CompositeType { type_name, .. } = &parent.field_type else {
            continue;
        };
        let Some(composite) = ir.composite_types.get(type_name) else {
            continue;
        };

        for nested in &composite.fields {
            if !is_orderable_composite_field(nested) {
                continue;
            }
            let Some(rust_type) = composite_field_rust_type(nested, extensions) else {
                continue;
            };

            let path = format!("{}.{}", parent.logical_name, nested.logical_name);
            let base_method_name =
                format!("{}_{}", parent.logical_name, nested.logical_name).to_snake_case();
            let mut method_name = base_method_name.clone();
            let mut suffix = 0usize;
            while used_methods.contains(&method_name) {
                suffix += 1;
                method_name = if suffix == 1 {
                    format!("{base_method_name}_order")
                } else {
                    format!("{base_method_name}_order_{suffix}")
                };
            }
            used_methods.insert(method_name.clone());

            fields.push(NestedOrderByFieldContext {
                method_name,
                path,
                parent_db_name: parent.db_name.clone(),
                field_db_name: nested.db_name.clone(),
                json_key: nested.logical_name.clone(),
                json_cast: json_path_cast_variant(&nested.field_type).to_string(),
                rust_type,
            });
        }
    }

    fields
}
