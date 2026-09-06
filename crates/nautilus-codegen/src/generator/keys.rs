//! Rust cursor, unique constraint and vector search metadata.

use crate::model_view::ModelView;
use heck::ToSnakeCase;
use nautilus_schema::ir::{FieldIr, ModelIr};
use serde::Serialize;
use std::collections::HashSet;

/// A pgvector column a caller may target with `FindManyArgs::nearest`.
#[derive(Debug, Clone, Serialize)]
pub(super) struct VectorFieldContext {
    /// Snake-case logical name, used to name the generated accessor.
    name: String,
    /// Original logical field name from the schema.
    logical_name: String,
    /// Database column name of the vector field.
    db_name: String,
    /// Every spelling of the field a caller may pass in `nearest.field`:
    /// logical name, database column, and qualified `table__column`.
    aliases: Vec<String>,
}

/// Serialisable (logical_name, db_name) pair for primary-key fields.
/// Used in templates to generate cursor predicate slices.
#[derive(Debug, Clone, Serialize)]
pub(super) struct PkFieldContext {
    /// Snake-case logical name — used as the cursor map key in generated code.
    name: String,
    /// Original logical field name from the schema.
    logical_name: String,
    /// Database column name — used to build the `table__db_col` column reference.
    db_name: String,
}

pub(super) fn build_pk_fields(view: &ModelView<'_>) -> Vec<PkFieldContext> {
    view.primary_key_fields
        .iter()
        .filter_map(|logical| {
            view.scalars
                .iter()
                .find(|scalar| scalar.logical_name() == *logical)
                .map(|scalar| pk_field_context(scalar.field))
        })
        .collect()
}

pub(super) fn build_vector_fields(view: &ModelView<'_>) -> Vec<VectorFieldContext> {
    view.scalars
        .iter()
        .filter(|scalar| scalar.field.is_vector())
        .map(|scalar| {
            let db_name = scalar.field.db_name.clone();
            let mut aliases = vec![
                scalar.logical_name().to_string(),
                db_name.clone(),
                format!("{}__{}", view.db_name(), db_name),
            ];
            aliases.sort();
            aliases.dedup();
            VectorFieldContext {
                name: scalar.snake_name(),
                logical_name: scalar.logical_name().to_string(),
                db_name,
                aliases,
            }
        })
        .collect()
}

fn pk_field_context(field: &FieldIr) -> PkFieldContext {
    PkFieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
    }
}

/// Every column set that identifies at most one row: the primary key plus each
/// unique constraint, deduplicated by database column names.
pub(super) fn build_single_record_constraints(
    model: &ModelIr,
    pk_fields: &[PkFieldContext],
) -> Vec<Vec<PkFieldContext>> {
    let mut constraints = Vec::new();
    let mut seen_keys = HashSet::new();
    let key_of = |fields: &[PkFieldContext]| -> Vec<String> {
        fields.iter().map(|field| field.db_name.clone()).collect()
    };

    if !pk_fields.is_empty() && seen_keys.insert(key_of(pk_fields)) {
        constraints.push(pk_fields.to_vec());
    }

    for constraint in &model.unique_constraints {
        let fields: Vec<PkFieldContext> = constraint
            .fields
            .iter()
            .filter_map(|logical| {
                model
                    .scalar_fields()
                    .find(|f| f.logical_name == *logical)
                    .map(pk_field_context)
            })
            .collect();

        if fields.len() != constraint.fields.len() || fields.is_empty() {
            continue;
        }
        if seen_keys.insert(key_of(&fields)) {
            constraints.push(fields);
        }
    }

    constraints
}
