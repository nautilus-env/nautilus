//! Composite type code generator (Rust backend).

use heck::ToSnakeCase;
use nautilus_schema::ir::{ResolvedFieldType, SchemaIr};
use serde::Serialize;
use tera::Context;

use crate::generator::TEMPLATES;

#[derive(Debug, Clone, Serialize)]
struct CompositeFieldContext {
    name: String,
    rust_type: String,
}

fn composite_field_rust_type(
    field_type: &ResolvedFieldType,
    is_required: bool,
    is_array: bool,
) -> String {
    let base = match field_type {
        ResolvedFieldType::Scalar(scalar) => scalar.rust_type().to_string(),
        ResolvedFieldType::Enum { enum_name } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name } => type_name.clone(),
        ResolvedFieldType::Relation(_) => "serde_json::Value".to_string(),
    };

    if is_array {
        format!("Vec<{}>", base)
    } else if !is_required {
        format!("Option<{}>", base)
    } else {
        base
    }
}

/// Generate Rust code for all composite types in the schema.
///
/// Returns `None` when there are no composite types.
pub fn generate_all_composite_types(ir: &SchemaIr) -> Option<String> {
    if ir.composite_types.is_empty() {
        return None;
    }

    let mut output = String::new();
    output.push_str("//! Generated composite types.\n\n");

    let mut names: Vec<&String> = ir.composite_types.keys().collect();
    names.sort();

    for name in names {
        let ctype = &ir.composite_types[name];
        let mut context = Context::new();
        context.insert("type_name", &ctype.logical_name);

        let fields: Vec<CompositeFieldContext> = ctype
            .fields
            .iter()
            .map(|f| CompositeFieldContext {
                name: f.logical_name.to_snake_case(),
                rust_type: composite_field_rust_type(&f.field_type, f.is_required, f.is_array),
            })
            .collect();

        context.insert("fields", &fields);

        let code = TEMPLATES
            .render("composite_type.tera", &context)
            .unwrap_or_else(|e| {
                panic!(
                    "template rendering failed for composite type '{}': {:?}",
                    name, e
                )
            });

        output.push_str(&code);
    }

    Some(output)
}
