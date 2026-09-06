//! JavaScript relation hydration and include inputs.

use crate::model_view::ModelView;
use heck::{ToLowerCamelCase, ToSnakeCase};
use serde::Serialize;

use super::fields::JsFieldContext;

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsIncludeFieldContext {
    name: String,
    target_model: String,
    target_snake: String,
    /// camelCase — property name on the generated Nautilus class.
    target_camel: String,
    is_array: bool,
}

/// Relation fields are hydrated separately, so they carry no column metadata
/// and always default to empty.
pub(super) fn build_relation_fields(view: &ModelView<'_>) -> Vec<JsFieldContext> {
    view.relations
        .iter()
        .map(|relation| {
            let target = relation.target_model_name();
            let (ts_type, base_type) = if relation.is_array() {
                (format!("{}Model[]", target), format!("{}Model", target))
            } else {
                (
                    format!("{}Model | null", target),
                    format!("{}Model", target),
                )
            };

            JsFieldContext {
                name: relation.logical_name().to_string(),
                logical_name: relation.logical_name().to_string(),
                db_name: relation.field.db_name.clone(),
                input_ts_type: ts_type.clone(),
                ts_type,
                raw_base_type: base_type.clone(),
                base_type,
                extension_coercer: String::new(),
                extension_input_serializer: String::new(),
                is_optional: true,
                is_array: relation.is_array(),
                is_enum: false,
                has_default: true,
                default: if relation.is_array() {
                    "[]".to_string()
                } else {
                    "null".to_string()
                },
                is_pk: false,
                doc_comment: crate::schema_docs::field_modifier_doc(view.model, relation.field),
                index: relation.index,
            }
        })
        .collect()
}

pub(super) fn build_include_fields(view: &ModelView<'_>) -> Vec<JsIncludeFieldContext> {
    view.relations
        .iter()
        .map(|relation| JsIncludeFieldContext {
            name: relation.logical_name().to_string(),
            target_model: relation.target_model_name().to_string(),
            target_snake: relation.target_model_name().to_snake_case(),
            target_camel: relation.target_model_name().to_lower_camel_case(),
            is_array: relation.is_array(),
        })
        .collect()
}
