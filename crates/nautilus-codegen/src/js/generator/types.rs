//! TypeScript input and output expressions, enum and composite declarations.

use crate::extension_types::{ts_input_type_for_extension, ExtensionRegistry};
use crate::js::type_mapper::scalar_to_ts_type;
use anyhow::Result;
use nautilus_schema::ir::{CompositeTypeIr, EnumIr, ResolvedFieldType};
use serde::Serialize;
use std::collections::HashMap;
use tera::Context;

use super::templates::render;

pub(super) fn output_base_ts_type(
    field: &nautilus_schema::ir::FieldIr,
    enums: &HashMap<String, EnumIr>,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return ty.type_name.to_string();
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_ts_type(scalar).to_string(),
        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => {
            if enums.contains_key(&rel.target_model) {
                rel.target_model.clone()
            } else {
                format!("{}Model", rel.target_model)
            }
        }
    }
}

pub(super) fn input_base_ts_type(
    field: &nautilus_schema::ir::FieldIr,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return ts_input_type_for_extension(ty);
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_ts_type(scalar).to_string(),
        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => format!("{}Model", rel.target_model),
    }
}

pub(super) fn exact_output_ts_type(
    field: &nautilus_schema::ir::FieldIr,
    base_type: String,
) -> String {
    if field.is_array {
        format!("{}[]", base_type)
    } else if !field.is_required {
        format!("{} | null", base_type)
    } else {
        base_type
    }
}

pub(super) fn exact_input_ts_type(
    field: &nautilus_schema::ir::FieldIr,
    base_type: String,
) -> String {
    if field.is_array {
        format!("{}[]", base_type)
    } else if !field.is_required {
        format!("{} | null", base_type)
    } else {
        base_type
    }
}

/// Generate `types.d.ts` — TypeScript interfaces for all composite types.
///
/// Returns `None` when there are no composite types.
pub fn generate_js_composite_types(
    composite_types: &HashMap<String, CompositeTypeIr>,
) -> Result<Option<String>> {
    if composite_types.is_empty() {
        return Ok(None);
    }

    #[derive(Serialize)]
    struct CompositeFieldCtx {
        name: String,
        ts_type: String,
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
                        ResolvedFieldType::Scalar(s) => scalar_to_ts_type(s).to_string(),
                        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
                        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
                        ResolvedFieldType::Relation(_) => "unknown".to_string(),
                    };
                    let ts_type = if f.is_array {
                        format!("{}[]", base)
                    } else if !f.is_required {
                        format!("{} | null", base)
                    } else {
                        base
                    };
                    CompositeFieldCtx {
                        name: f.logical_name.clone(),
                        ts_type,
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

    Ok(Some(render("composite_types.d.ts.tera", &context)?))
}

/// Generate `enums.js` + `enums.d.ts` for all enum definitions.
///
/// Returns `(js_code, dts_code)`.
pub fn generate_js_enums(enums: &HashMap<String, EnumIr>) -> Result<(String, String)> {
    #[derive(Serialize)]
    struct EnumCtx {
        name: String,
        variants: Vec<String>,
    }

    let mut enum_list: Vec<EnumCtx> = enums
        .values()
        .map(|e| EnumCtx {
            name: e.logical_name.clone(),
            variants: e.variants.clone(),
        })
        .collect();
    enum_list.sort_by(|a, b| a.name.cmp(&b.name));

    let mut context = Context::new();
    context.insert("enums", &enum_list);
    let js_code = render("enums.js.tera", &context)?;
    let dts_code = render("enums.d.ts.tera", &context)?;
    Ok((js_code, dts_code))
}
