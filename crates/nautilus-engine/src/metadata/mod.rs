//! Per-model metadata computed once at engine start-up and reused by the hot
//! query paths.

mod fields;
mod relations;

use std::collections::HashMap;
use std::sync::OnceLock;

use nautilus_core::{ColumnMarker, TableName};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{CompositeTypeIr, ModelIr};

use crate::conversion::ValueHint;
use crate::filter::{FieldTypeMap, RelationMap};

pub(crate) use fields::{
    build_db_to_logical_map, build_field_type_map, build_logical_to_db_map, field_value_hint,
    PrimaryKeyFieldMetadata, ScalarFieldMetadata,
};
pub(crate) use relations::build_relation_map;

/// The physical table a model reads and writes, schema-qualified when the model
/// declares `@@schema("...")`.
pub(crate) fn model_table(model: &ModelIr) -> TableName {
    TableName::with_schema(model.schema.clone(), model.db_name.clone())
}

#[derive(Debug)]
pub(crate) struct ModelMetadata {
    field_types: FieldTypeMap,
    logical_to_db: HashMap<String, String>,
    db_to_logical: HashMap<String, String>,
    scalar_fields: Vec<ScalarFieldMetadata>,
    scalar_markers: Vec<ColumnMarker>,
    scalar_hints: Vec<Option<ValueHint>>,
    primary_key_fields: Vec<PrimaryKeyFieldMetadata>,
    relation_map: OnceLock<Result<RelationMap, String>>,
}

impl ModelMetadata {
    pub(crate) fn new(
        model: &ModelIr,
        composite_types: &HashMap<String, CompositeTypeIr>,
        native_composites: bool,
    ) -> Self {
        let scalar_fields = ScalarFieldMetadata::collect(model, composite_types, native_composites);

        let scalar_markers = scalar_fields
            .iter()
            .map(|field| field.marker().clone())
            .collect();
        let scalar_hints = scalar_fields
            .iter()
            .map(ScalarFieldMetadata::hint)
            .collect();
        let primary_key_fields = PrimaryKeyFieldMetadata::collect(model, &scalar_fields);

        Self {
            field_types: build_field_type_map(model),
            logical_to_db: build_logical_to_db_map(model),
            db_to_logical: build_db_to_logical_map(model),
            scalar_fields,
            scalar_markers,
            scalar_hints,
            primary_key_fields,
            relation_map: OnceLock::new(),
        }
    }

    pub(crate) fn field_types(&self) -> &FieldTypeMap {
        &self.field_types
    }

    pub(crate) fn logical_to_db(&self) -> &HashMap<String, String> {
        &self.logical_to_db
    }

    pub(crate) fn db_to_logical(&self) -> &HashMap<String, String> {
        &self.db_to_logical
    }

    pub(crate) fn scalar_fields(&self) -> &[ScalarFieldMetadata] {
        &self.scalar_fields
    }

    pub(crate) fn scalar_markers(&self) -> &[ColumnMarker] {
        &self.scalar_markers
    }

    pub(crate) fn scalar_hints(&self) -> &[Option<ValueHint>] {
        &self.scalar_hints
    }

    pub(crate) fn primary_key_fields(&self) -> &[PrimaryKeyFieldMetadata] {
        &self.primary_key_fields
    }

    /// The relation map for this model, built on first use and cached.
    pub(crate) fn relation_map<'a>(
        &'a self,
        model: &ModelIr,
        models: &HashMap<String, ModelIr>,
    ) -> Result<&'a RelationMap, ProtocolError> {
        match self.relation_map.get_or_init(|| {
            build_relation_map(model, models).map_err(|error| match error {
                ProtocolError::QueryPlanning(message) => message,
                other => other.to_string(),
            })
        }) {
            Ok(map) => Ok(map),
            Err(message) => Err(ProtocolError::QueryPlanning(message.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_schema::validate_schema_source;

    fn parse_ir(source: &str) -> nautilus_schema::ir::SchemaIr {
        validate_schema_source(source)
            .expect("validation failed")
            .ir
    }

    #[test]
    fn model_metadata_caches_mappings_hints_and_relation_map() {
        let ir = parse_ir(
            r#"
model User {
  id        Int      @id @default(autoincrement())
  createdAt DateTime @map("created_at")
  profile   Profile?
}

model Profile {
  id     Int  @id @default(autoincrement())
  userId Int  @unique @map("user_id")
  user   User @relation(fields: [userId], references: [id])
}
"#,
        );
        let user_model = ir.models.get("User").expect("User model missing");
        let metadata = ModelMetadata::new(user_model, &ir.composite_types, false);

        assert_eq!(
            metadata
                .logical_to_db()
                .get("createdAt")
                .map(String::as_str),
            Some("created_at")
        );
        assert_eq!(
            metadata
                .db_to_logical()
                .get("created_at")
                .map(String::as_str),
            Some("createdAt")
        );
        assert_eq!(metadata.scalar_hints().len(), 2);
        assert_eq!(metadata.scalar_hints()[1], Some(ValueHint::DateTime));
        assert_eq!(metadata.primary_key_fields().len(), 1);
        assert_eq!(
            metadata.primary_key_fields()[0].qualified_column(),
            "User__id"
        );

        let first = metadata
            .relation_map(user_model, &ir.models)
            .expect("relation map should build");
        let second = metadata
            .relation_map(user_model, &ir.models)
            .expect("relation map should be cached");

        assert!(std::ptr::eq(first, second));
        assert!(first.contains_key("profile"));
    }
}
