//! Python input and output type expressions, enums and composite declarations.

use crate::extension_types::{python_input_type_for_extension, ExtensionRegistry};
use anyhow::Result;
use heck::ToSnakeCase;
use nautilus_schema::ir::{CompositeTypeIr, EnumIr, ResolvedFieldType};
use serde::Serialize;
use std::collections::HashMap;
use tera::Context;

use super::templates::render;

pub(super) fn output_base_python_type(
    field: &nautilus_schema::ir::FieldIr,
    enums: &HashMap<String, EnumIr>,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return ty.type_name.to_string();
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => {
            crate::python::type_mapper::scalar_to_python_type(scalar).to_string()
        }
        ResolvedFieldType::Enum { enum_name, .. } => {
            if enums.contains_key(enum_name) {
                enum_name.clone()
            } else {
                "str".to_string()
            }
        }
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => rel.target_model.clone(),
    }
}

pub(super) fn exact_output_python_type(
    field: &nautilus_schema::ir::FieldIr,
    base_type: String,
) -> String {
    if field.is_array {
        format!("List[{}]", base_type)
    } else if !field.is_required {
        format!("Optional[{}]", base_type)
    } else {
        base_type
    }
}

pub(super) fn exact_input_python_type(
    field: &nautilus_schema::ir::FieldIr,
    base_type: String,
) -> String {
    if field.is_array {
        format!("List[{}]", base_type)
    } else if !field.is_required {
        format!("Optional[{}]", base_type)
    } else {
        base_type
    }
}

pub(super) fn add_none_to_python_union(type_expr: String) -> String {
    let trimmed = type_expr.trim();
    if let Some(inner) = trimmed
        .strip_prefix("Union[")
        .and_then(|value| value.strip_suffix(']'))
    {
        format!("Union[{inner}, None]")
    } else {
        format!("Optional[{trimmed}]")
    }
}

pub(super) fn input_base_python_type(
    field: &nautilus_schema::ir::FieldIr,
    enums: &HashMap<String, EnumIr>,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return python_input_type_for_extension(ty);
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => {
            crate::python::type_mapper::scalar_to_python_type(scalar).to_string()
        }
        ResolvedFieldType::Enum { enum_name, .. } => {
            if enums.contains_key(enum_name) {
                enum_name.clone()
            } else {
                "str".to_string()
            }
        }
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => rel.target_model.clone(),
    }
}

/// Generate `types/types.py` — TypedDict declarations for all composite types.
///
/// Returns `None` when there are no composite types.
pub fn generate_python_composite_types(
    composite_types: &HashMap<String, CompositeTypeIr>,
) -> Result<Option<String>> {
    if composite_types.is_empty() {
        return Ok(None);
    }

    #[derive(Serialize)]
    struct CompositeFieldCtx {
        name: String,
        python_type: String,
    }

    #[derive(Serialize)]
    struct CompositeTypeCtx {
        name: String,
        fields: Vec<CompositeFieldCtx>,
    }

    let mut type_list: Vec<CompositeTypeCtx> = composite_types
        .values()
        .map(|ct| {
            let fields = ct
                .fields
                .iter()
                .map(|f| {
                    let base = match &f.field_type {
                        ResolvedFieldType::Scalar(s) => {
                            crate::python::type_mapper::scalar_to_python_type(s).to_string()
                        }
                        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
                        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
                        ResolvedFieldType::Relation(_) => "Any".to_string(),
                    };
                    let python_type = if f.is_array {
                        format!("List[{}]", base)
                    } else if !f.is_required {
                        format!("Optional[{}]", base)
                    } else {
                        base
                    };
                    CompositeFieldCtx {
                        name: f.logical_name.to_snake_case(),
                        python_type,
                    }
                })
                .collect();
            CompositeTypeCtx {
                name: ct.logical_name.clone(),
                fields,
            }
        })
        .collect();
    type_list.sort_by(|a, b| a.name.cmp(&b.name));

    let mut context = Context::new();
    context.insert("composite_types", &type_list);

    Ok(Some(render("composite_types.py.tera", &context)?))
}

/// Generate Python enums file.
pub fn generate_python_enums(enums: &HashMap<String, EnumIr>) -> Result<String> {
    let mut context = Context::new();

    #[derive(Serialize)]
    struct EnumContext {
        name: String,
        variants: Vec<String>,
    }

    let enum_contexts: Vec<EnumContext> = enums
        .values()
        .map(|e| EnumContext {
            name: e.logical_name.clone(),
            variants: e.variants.clone(),
        })
        .collect();

    context.insert("enums", &enum_contexts);

    render("enums.py.tera", &context)
}
