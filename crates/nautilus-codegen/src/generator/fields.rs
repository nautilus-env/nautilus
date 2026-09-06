//! Rust scalar field contexts, read hints and aggregate types.

use crate::extension_types::ExtensionRegistry;
use crate::model_view::{FieldView, ModelView};
use crate::type_helpers::{
    field_to_rust_avg_type, field_to_rust_base_type, field_to_rust_sum_type, field_to_rust_type,
    scalar_to_rust_type,
};
use heck::ToPascalCase;
use nautilus_schema::ast::StorageStrategy;
use nautilus_schema::ir::{FieldIr, ResolvedFieldType, ScalarType};
use serde::Serialize;

/// Rust field types, column markers and decoding expressions for the templates.
#[derive(Debug, Clone, Serialize)]
pub(super) struct FieldContext {
    pub(super) name: String,
    pub(super) logical_name: String,
    pub(super) db_name: String,
    pub(super) rust_type: String,
    pub(super) base_rust_type: String,
    pub(super) column_type: String,
    pub(super) read_hint_expr: String,
    pub(super) variant_name: String,
    pub(super) is_array: bool,
    pub(super) index: usize,
    pub(super) is_pk: bool,
    /// `true` when the field maps to an `Option<T>` Rust type
    /// (i.e. the schema field is not required and is not a relation).
    pub(super) is_optional: bool,
    /// `true` when the field has `@updatedAt` — auto-defaults to `now()` if not provided.
    pub(super) is_updated_at: bool,
    /// `true` when the field is a `@computed` generated column (read-only from client side).
    pub(super) is_computed: bool,
    /// `true` when the column's type lets an update derive the new value from
    /// the current one, which is what the `increment` / `decrement` /
    /// `multiply` / `divide` operators need. Mirrors the engine's own rule.
    pub(super) accepts_arithmetic: bool,
    pub(super) doc_comment: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AggregateFieldContext {
    name: String,
    logical_name: String,
    rust_type: String,
    avg_rust_type: String,
    sum_rust_type: String,
    variant_name: String,
}

/// The per-field template contexts a model needs, collected in a single pass
/// over the shared [`ModelView`].
#[derive(Default)]
pub(super) struct ScalarFieldContexts {
    pub(super) scalar: Vec<FieldContext>,
    pub(super) create: Vec<FieldContext>,
    pub(super) updated_at: Vec<FieldContext>,
    pub(super) numeric: Vec<AggregateFieldContext>,
    pub(super) orderable: Vec<AggregateFieldContext>,
}

fn field_read_hint_expr(field: &FieldIr) -> String {
    if field.is_array && field.storage_strategy == Some(StorageStrategy::Json) {
        return "Some(crate::ValueHint::Json)".to_string();
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => {
            "Some(crate::ValueHint::Decimal)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::DateTime) => {
            "Some(crate::ValueHint::DateTime)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::Json | ScalarType::Jsonb) => {
            "Some(crate::ValueHint::Json)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::Uuid) => "Some(crate::ValueHint::Uuid)".to_string(),
        ResolvedFieldType::Scalar(ScalarType::Geometry) => {
            "Some(crate::ValueHint::Geometry)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::Geography) => {
            "Some(crate::ValueHint::Geography)".to_string()
        }
        ResolvedFieldType::CompositeType { .. }
            if field.storage_strategy == Some(StorageStrategy::Json) =>
        {
            "Some(crate::ValueHint::Json)".to_string()
        }
        _ => "None".to_string(),
    }
}

pub(super) fn build_scalar_fields(
    view: &ModelView<'_>,
    extensions: &ExtensionRegistry,
) -> ScalarFieldContexts {
    let mut contexts = ScalarFieldContexts::default();

    for scalar in &view.scalars {
        let field = scalar.field;
        let field_ctx = scalar_field_context(scalar, extensions);
        let base_rust_type = field_ctx.base_rust_type.clone();

        contexts.create.push(field_ctx.clone());
        if field.is_updated_at {
            contexts.updated_at.push(field_ctx.clone());
        }
        contexts.scalar.push(field_ctx);

        if scalar.numeric_scalar().is_some() {
            contexts.numeric.push(AggregateFieldContext {
                name: scalar.snake_name(),
                logical_name: field.logical_name.clone(),
                rust_type: base_rust_type.clone(),
                avg_rust_type: field_to_rust_avg_type(field),
                sum_rust_type: field_to_rust_sum_type(field, extensions),
                variant_name: field.logical_name.to_pascal_case(),
            });
        }

        if scalar.is_orderable() {
            contexts.orderable.push(AggregateFieldContext {
                name: scalar.snake_name(),
                logical_name: field.logical_name.clone(),
                rust_type: base_rust_type,
                avg_rust_type: String::new(),
                sum_rust_type: String::new(),
                variant_name: field.logical_name.to_pascal_case(),
            });
        }
    }

    contexts
}

pub(super) fn scalar_field_context(
    scalar: &FieldView<'_>,
    extensions: &ExtensionRegistry,
) -> FieldContext {
    let field = scalar.field;
    let column_type = match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_rust_type(scalar, extensions),
        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        _ => String::new(),
    };

    FieldContext {
        name: scalar.snake_name(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        rust_type: field_to_rust_type(field, extensions),
        base_rust_type: field_to_rust_base_type(field, extensions),
        column_type,
        read_hint_expr: field_read_hint_expr(field),
        variant_name: field.logical_name.to_pascal_case(),
        is_array: field.is_array,
        index: scalar.index,
        is_pk: scalar.is_pk,
        is_optional: !field.is_required && !field.is_array,
        is_updated_at: field.is_updated_at,
        is_computed: field.computed.is_some(),
        accepts_arithmetic: scalar.accepts_arithmetic(),
        doc_comment: scalar.doc_comment.clone(),
    }
}
