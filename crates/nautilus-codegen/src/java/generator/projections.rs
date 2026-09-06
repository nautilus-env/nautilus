//! Java projections preserve the distinction between missing and null fields.

use crate::java::type_mapper::field_to_java_type;
use anyhow::Result;
use heck::ToUpperCamelCase;
use nautilus_schema::ir::{FieldIr, ModelIr, ResolvedFieldType};
use serde::Serialize;
use std::collections::BTreeSet;
use tera::Context;

use super::config::JavaConfig;
use super::readers::{array_reader_for_scalar, scalar_reader_for_type};
use super::templates::render;

#[derive(Debug, Serialize)]
struct ProjectionFieldContext {
    ty: String,
    name: String,
    has_method: String,
    doc_comment: String,
    read_expr: String,
    source_expr: String,
}

#[derive(Debug, Serialize)]
struct ProjectionTemplateContext {
    package_name: String,
    imports: Vec<String>,
    name: String,
    fields: Vec<ProjectionFieldContext>,
}

pub(super) fn generate_projection_file(config: &JavaConfig, model: &ModelIr) -> Result<String> {
    let projection_name = format!("{}Projection", model.logical_name);
    let mut imports = BTreeSet::new();
    imports.insert(format!("{}.internal.JsonSupport", config.root_package));
    imports.insert(format!("{}.internal.WireSerializable", config.root_package));
    imports.insert("com.fasterxml.jackson.databind.JsonNode".to_string());

    let fields: Vec<ProjectionFieldContext> = model
        .scalar_fields()
        .map(|field| {
            let (ty, field_imports) = field_to_java_type(
                field,
                &config.root_package,
                &model.logical_name,
                &config.extensions,
            );
            imports.extend(field_imports);
            ProjectionFieldContext {
                ty,
                name: field.logical_name.clone(),
                has_method: format!("has{}", field.logical_name.to_upper_camel_case()),
                doc_comment: crate::schema_docs::field_modifier_doc(model, field),
                read_expr: projection_field_read_expr(config, model, field),
                source_expr: projection_field_source_expr(model, field),
            }
        })
        .collect();

    let context = Context::from_serialize(&ProjectionTemplateContext {
        package_name: config.root_package.clone(),
        imports: imports.into_iter().collect(),
        name: projection_name,
        fields,
    })
    .expect("Java projection context should serialize");
    render("java_projection.tera", &context)
}

fn projection_field_source_expr(model: &ModelIr, field: &FieldIr) -> String {
    format!(
        "JsonSupport.firstPresent(this.row, \"{}__{}\", \"{}\")",
        model.db_name, field.db_name, field.logical_name
    )
}

fn projection_field_read_expr(config: &JavaConfig, model: &ModelIr, field: &FieldIr) -> String {
    let source = projection_field_source_expr(model, field);
    if field.is_array {
        match &field.field_type {
            ResolvedFieldType::Scalar(scalar) => {
                if let Some(ext) = config.extensions.type_for_scalar(scalar) {
                    return format!(
                        "JsonSupport.asList({source}, {ty}::fromJsonNode)",
                        ty = ext.type_name
                    );
                }
                let reader = array_reader_for_scalar(scalar);
                format!(
                    "JsonSupport.asList({source}, {reader})",
                    source = source,
                    reader = reader
                )
            }
            ResolvedFieldType::Enum { enum_name, .. } => {
                format!("JsonSupport.asList({source}, value -> JsonSupport.asEnum(value, {enum_name}.class))")
            }
            ResolvedFieldType::CompositeType { type_name, .. } => {
                format!("JsonSupport.asList({source}, {type_name}::fromJsonNode)")
            }
            ResolvedFieldType::Relation(_) => unreachable!(),
        }
    } else {
        match &field.field_type {
            ResolvedFieldType::Scalar(scalar) => {
                if let Some(ext) = config.extensions.type_for_scalar(scalar) {
                    return format!(
                        "{ty}.fromJsonNode({source})",
                        ty = ext.type_name,
                        source = source
                    );
                }
                let reader = scalar_reader_for_type(scalar);
                format!(
                    "JsonSupport.{reader}({source})",
                    reader = reader,
                    source = source
                )
            }
            ResolvedFieldType::Enum { enum_name, .. } => {
                format!("JsonSupport.asEnum({source}, {enum_name}.class)")
            }
            ResolvedFieldType::CompositeType { type_name, .. } => {
                format!("JsonSupport.asObject({source}, {type_name}::fromJsonNode)")
            }
            ResolvedFieldType::Relation(_) => unreachable!(),
        }
    }
}
