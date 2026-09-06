//! Python relation hydration and include inputs.

use crate::backend::LanguageBackend;
use crate::extension_types::ExtensionRegistry;
use crate::model_view::ModelView;
use crate::python::backend::PythonBackend;
use heck::ToSnakeCase;
use nautilus_schema::ir::SchemaIr;
use serde::Serialize;

use super::fields::PythonFieldContext;
use super::types::output_base_python_type;

#[derive(Debug, Clone, Serialize)]
pub(super) struct PythonRelationContext {
    field_name: String,
    target_model: String,
    target_table: String,
    is_array: bool,
    fields: Vec<String>,
    references: Vec<String>,
    fields_db: Vec<String>,
    references_db: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IncludeFieldContext {
    name: String,
    logical_name: String,
    target_model: String,
    /// snake_case module name of the target model (e.g. "post" for Post)
    target_snake: String,
    /// true if this is a one-to-many relation (List/array)
    is_array: bool,
}

/// Relation fields are hydrated separately, so they carry no column metadata
/// and always default to empty.
pub(super) fn build_relation_fields(
    view: &ModelView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Vec<PythonFieldContext> {
    view.relations
        .iter()
        .map(|relation| {
            let field = relation.field;
            let python_type = PythonBackend
                .wrap_field_type(field, output_base_python_type(field, &ir.enums, extensions));
            let default_val = if relation.is_array() {
                "Field(default_factory=list)".to_string()
            } else {
                "None".to_string()
            };

            PythonFieldContext {
                name: relation.snake_name(),
                logical_name: relation.logical_name().to_string(),
                db_name: field.db_name.clone(),
                input_python_type: python_type.clone(),
                model_python_type: python_type.clone(),
                python_type,
                base_type: String::new(),
                raw_base_type: String::new(),
                extension_coercer: String::new(),
                extension_input_serializer: String::new(),
                is_optional: true,
                is_array: relation.is_array(),
                is_enum: false,
                has_default: true,
                default: default_val,
                model_has_default: true,
                model_default: "None".to_string(),
                is_pk: false,
                doc_comment: crate::schema_docs::field_modifier_doc(view.model, field),
                index: relation.index,
            }
        })
        .collect()
}

pub(super) fn build_relations(view: &ModelView<'_>) -> Vec<PythonRelationContext> {
    view.resolved_relations()
        .map(|(relation, target)| PythonRelationContext {
            field_name: relation.snake_name(),
            target_model: relation.target_model_name().to_string(),
            target_table: target.db_name.clone(),
            is_array: relation.is_array(),
            fields_db: relation.fields_db.clone(),
            references_db: relation.references_db.clone(),
            fields: relation.fields.clone(),
            references: relation.references.clone(),
        })
        .collect()
}

pub(super) fn build_include_fields(view: &ModelView<'_>) -> Vec<IncludeFieldContext> {
    view.relations
        .iter()
        .map(|relation| IncludeFieldContext {
            name: relation.snake_name(),
            logical_name: relation.logical_name().to_string(),
            target_model: relation.target_model_name().to_string(),
            target_snake: relation.target_model_name().to_snake_case(),
            is_array: relation.is_array(),
        })
        .collect()
}
