//! Rust relation hydration and nested write inputs.

use crate::extension_types::ExtensionRegistry;
use crate::model_view::ModelView;
use crate::type_helpers::{field_to_rust_base_type, field_to_rust_type};
use heck::ToPascalCase;
use nautilus_schema::ir::SchemaIr;
use serde::Serialize;

use super::fields::{scalar_field_context, FieldContext};

/// Template context for the nested-write entry a relation adds to
/// `{Model}CreateInput` and `{Model}UpdateInput`.
///
/// Which operations the generated struct carries depends on `is_owning`: the
/// side holding the foreign key writes a single related row and so takes one
/// operation at a time, while the side pointed at takes lists of them.
#[derive(Debug, Clone, Serialize)]
pub(super) struct NestedWriteContext {
    /// Rust field name on the create/update input.
    pub(super) field_name: String,
    /// Name the engine matches the relation by.
    pub(super) wire_name: String,
    pub(super) target_model: String,
    pub(super) create_nested_name: String,
    pub(super) update_nested_name: String,
    pub(super) target_create_input: String,
    pub(super) target_update_input: String,
    pub(super) is_owning: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RelationContext {
    field_name: String,
    target_model: String,
    target_table: String,
    is_array: bool,
    fields: Vec<String>,
    references: Vec<String>,
    fields_db: Vec<String>,
    references_db: Vec<String>,
    /// The join table, when the relation is an implicit many-to-many.
    join: Option<JoinTableContext>,
    target_scalar_fields: Vec<FieldContext>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JoinTableContext {
    table: String,
    parent_column: String,
    child_column: String,
}

/// Relation fields carry no column of their own: they are always hydrated
/// separately, so they get no column type, no read hint and are optional.
pub(super) fn build_relation_fields(
    view: &ModelView<'_>,
    extensions: &ExtensionRegistry,
) -> Vec<FieldContext> {
    view.relations
        .iter()
        .map(|relation| {
            let field = relation.field;
            FieldContext {
                name: relation.snake_name(),
                logical_name: field.logical_name.clone(),
                db_name: field.db_name.clone(),
                rust_type: field_to_rust_type(field, extensions),
                base_rust_type: field_to_rust_base_type(field, extensions),
                column_type: String::new(),
                read_hint_expr: "None".to_string(),
                variant_name: field.logical_name.to_pascal_case(),
                is_array: field.is_array,
                index: 0,
                is_pk: false,
                is_optional: true,
                is_updated_at: false,
                is_computed: false,
                accepts_arithmetic: false,
                doc_comment: crate::schema_docs::field_modifier_doc(view.model, field),
            }
        })
        .collect()
}

/// Build the nested-write entry of every relation whose target model exists.
///
/// A relation pointing at a model the schema does not declare gets no entry:
/// there would be no input type to nest.
pub(super) fn build_nested_writes(view: &ModelView<'_>) -> Vec<NestedWriteContext> {
    view.resolved_relations()
        .map(|(relation, target)| {
            let relation_pascal = relation.logical_name().to_pascal_case();
            NestedWriteContext {
                field_name: relation.snake_name(),
                wire_name: relation.logical_name().to_string(),
                target_model: target.logical_name.clone(),
                create_nested_name: format!(
                    "{}{}CreateNested",
                    view.logical_name(),
                    relation_pascal
                ),
                update_nested_name: format!(
                    "{}{}UpdateNested",
                    view.logical_name(),
                    relation_pascal
                ),
                target_create_input: format!("{}CreateInput", target.logical_name),
                target_update_input: format!("{}UpdateInput", target.logical_name),
                is_owning: relation.is_owning(),
            }
        })
        .collect()
}

/// The `{Model}CreateInput` / `{Model}UpdateInput` types a model file has to
/// import to nest writes, minus the ones it declares itself.
pub(super) fn nested_write_imports(
    view: &ModelView<'_>,
    nested_writes: &[NestedWriteContext],
) -> Vec<String> {
    let mut names: Vec<String> = nested_writes
        .iter()
        .filter(|nested| nested.target_model != view.logical_name())
        .flat_map(|nested| {
            [
                nested.target_create_input.clone(),
                nested.target_update_input.clone(),
            ]
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

pub(super) fn build_relations(
    view: &ModelView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Vec<RelationContext> {
    view.resolved_relations()
        .map(|(relation, target)| {
            let target_view = ModelView::new(target, ir, extensions);
            RelationContext {
                field_name: relation.snake_name(),
                target_model: relation.target_model_name().to_string(),
                target_table: target.db_name.clone(),
                is_array: relation.is_array(),
                fields_db: relation.fields_db.clone(),
                references_db: relation.references_db.clone(),
                join: relation.join().map(|join| JoinTableContext {
                    table: join.table.clone(),
                    parent_column: join.self_column.clone(),
                    child_column: join.target_column.clone(),
                }),
                fields: relation.fields.clone(),
                references: relation.references.clone(),
                target_scalar_fields: target_view
                    .scalars
                    .iter()
                    .map(|scalar| scalar_field_context(scalar, extensions))
                    .collect(),
            }
        })
        .collect()
}
