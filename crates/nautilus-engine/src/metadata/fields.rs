//! Per-field metadata: name maps, column markers and decoding hints.

use std::collections::HashMap;

use nautilus_core::ColumnMarker;
use nautilus_schema::ast::StorageStrategy;
use nautilus_schema::ir::{CompositeTypeIr, FieldIr, ModelIr, ResolvedFieldType, ScalarType};

use crate::conversion::ValueHint;
use crate::filter::FieldTypeMap;

#[derive(Debug, Clone)]
pub(crate) struct ScalarFieldMetadata {
    logical_name: String,
    db_name: String,
    marker: ColumnMarker,
    hint: Option<ValueHint>,
}

impl ScalarFieldMetadata {
    /// Describe every scalar field of `model` in declaration order.
    pub(crate) fn collect(
        model: &ModelIr,
        composite_types: &HashMap<String, CompositeTypeIr>,
        native_composites: bool,
    ) -> Vec<Self> {
        model
            .scalar_fields()
            .map(|field| Self {
                logical_name: field.logical_name.clone(),
                db_name: field.db_name.clone(),
                marker: ColumnMarker::new(&model.db_name, &field.db_name),
                hint: field_value_hint(field, composite_types, native_composites),
            })
            .collect()
    }

    pub(crate) fn logical_name(&self) -> &str {
        &self.logical_name
    }

    pub(crate) fn db_name(&self) -> &str {
        &self.db_name
    }

    pub(crate) fn marker(&self) -> &ColumnMarker {
        &self.marker
    }

    pub(crate) fn hint(&self) -> Option<ValueHint> {
        self.hint.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PrimaryKeyFieldMetadata {
    logical_name: String,
    db_name: String,
    qualified_column: String,
}

impl PrimaryKeyFieldMetadata {
    /// Describe the primary-key fields of `model`, in key order, skipping any
    /// key part that has no scalar field backing it.
    pub(crate) fn collect(model: &ModelIr, scalar_fields: &[ScalarFieldMetadata]) -> Vec<Self> {
        model
            .primary_key
            .fields()
            .into_iter()
            .filter_map(|logical_name| {
                scalar_fields
                    .iter()
                    .find(|field| field.logical_name() == logical_name)
                    .map(|field| Self {
                        logical_name: field.logical_name.clone(),
                        db_name: field.db_name.clone(),
                        qualified_column: format!("{}__{}", model.db_name, field.db_name()),
                    })
            })
            .collect()
    }

    pub(crate) fn logical_name(&self) -> &str {
        &self.logical_name
    }

    pub(crate) fn db_name(&self) -> &str {
        &self.db_name
    }

    pub(crate) fn qualified_column(&self) -> &str {
        &self.qualified_column
    }
}

pub(crate) fn build_field_type_map(model: &ModelIr) -> FieldTypeMap {
    model
        .fields
        .iter()
        .filter(|field| !matches!(field.field_type, ResolvedFieldType::Relation(_)))
        .flat_map(|field| {
            let mut entries = vec![(field.logical_name.clone(), field.field_type.clone())];
            if field.db_name != field.logical_name {
                entries.push((field.db_name.clone(), field.field_type.clone()));
            }
            entries
        })
        .collect()
}

pub(crate) fn build_logical_to_db_map(model: &ModelIr) -> HashMap<String, String> {
    model
        .scalar_fields()
        .flat_map(|field| {
            let mut entries = vec![(field.logical_name.clone(), field.db_name.clone())];
            if field.db_name != field.logical_name {
                entries.push((field.db_name.clone(), field.db_name.clone()));
            }
            entries
        })
        .collect()
}

pub(crate) fn build_db_to_logical_map(model: &ModelIr) -> HashMap<String, String> {
    model
        .scalar_fields()
        .map(|field| (field.db_name.clone(), field.logical_name.clone()))
        .collect()
}

pub(crate) fn field_value_hint(
    field: &FieldIr,
    composite_types: &HashMap<String, CompositeTypeIr>,
    native_composites: bool,
) -> Option<ValueHint> {
    if field.is_array && field.storage_strategy == Some(StorageStrategy::Json) {
        return Some(ValueHint::Json);
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Boolean) if !field.is_array => Some(ValueHint::Bool),
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => Some(ValueHint::Decimal),
        ResolvedFieldType::Scalar(ScalarType::DateTime) => Some(ValueHint::DateTime),
        ResolvedFieldType::Scalar(ScalarType::Json | ScalarType::Jsonb) => Some(ValueHint::Json),
        ResolvedFieldType::Scalar(ScalarType::Uuid) => Some(ValueHint::Uuid),
        ResolvedFieldType::Scalar(ScalarType::Geometry) => Some(ValueHint::Geometry),
        ResolvedFieldType::Scalar(ScalarType::Geography) => Some(ValueHint::Geography),
        // Native (PostgreSQL) composites come back as a record-literal string and
        // need schema-aware decoding; non-array fields only. JSON-stored
        // composites round-trip as ordinary JSON.
        ResolvedFieldType::CompositeType { type_name, .. }
            if native_composites && !field.is_array =>
        {
            composite_types
                .get(type_name)
                .map(|composite| ValueHint::Composite(std::sync::Arc::new(composite.clone())))
        }
        ResolvedFieldType::CompositeType { .. }
            if field.storage_strategy == Some(StorageStrategy::Json) =>
        {
            Some(ValueHint::Json)
        }
        _ => None,
    }
}
