//! Which model of a relation carries the foreign key, and the field lists that
//! follow from it.
//!
//! The side decides when a nested operation can run — before the parent
//! statement, after it, or against a join table — so it is resolved once, here,
//! and every operation reads it off the [`RelationBinding`] instead of
//! rediscovering it.
use nautilus_connector::Row;
use nautilus_core::Value;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{FieldIr, ManyToManyJoinIr, ModelIr, RelationIr, ResolvedFieldType};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::state::EngineState;

/// Which model carries the foreign-key columns of a relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelationSide {
    /// The written model holds them; the related row must exist first.
    Owning,
    /// The related model holds them; the parent row must exist first.
    Inverse,
    /// Neither does: the links live in a join table. See [`ManyToManyJoinIr`].
    ManyToMany,
}

/// The two field lists a nested write needs: the columns that hold the foreign
/// key and the columns it points at, both as logical names.
#[derive(Debug, Clone)]
pub(super) struct RelationBinding {
    pub(super) target_model: String,
    pub(super) side: RelationSide,
    /// Logical field names on the model that carries the foreign key. Empty on
    /// a many-to-many, where no model carries one.
    pub(super) foreign_keys: Vec<String>,
    /// Logical field names on the model the foreign key points at. On a
    /// many-to-many this is the written model's own key, which the join table
    /// stores.
    pub(super) referenced: Vec<String>,
    /// The join table, on a many-to-many.
    pub(super) via: Option<ManyToManyJoinIr>,
}

/// The relation field `key` names, in the wire spelling or the snake_case one.
pub(super) fn relation_field<'a>(
    model: &'a ModelIr,
    key: &str,
) -> Option<(&'a FieldIr, &'a RelationIr)> {
    model.fields.iter().find_map(|field| {
        let ResolvedFieldType::Relation(relation) = &field.field_type else {
            return None;
        };
        (field.logical_name == key
            || field.db_name == key
            || crate::metadata::to_snake_case(&field.logical_name) == key)
            .then_some((field, relation))
    })
}

/// Find the relation on `target` that is the other end of `relation`.
///
/// The inverse is the side that names foreign-key fields; a relation name
/// disambiguates when two relations connect the same pair of models.
fn inverse_relation<'a>(
    model: &ModelIr,
    field: &FieldIr,
    relation: &RelationIr,
    target: &'a ModelIr,
) -> Result<&'a RelationIr, ProtocolError> {
    let mut matches = target.relation_fields().filter_map(|candidate| {
        let ResolvedFieldType::Relation(inverse) = &candidate.field_type else {
            return None;
        };
        if inverse.target_model != model.logical_name || inverse.fields.is_empty() {
            return None;
        }
        match relation.name.as_deref() {
            Some(name) if inverse.name.as_deref() != Some(name) => None,
            _ => Some(inverse),
        }
    });

    let first = matches.next().ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Nested write on '{}.{}' needs the foreign-key side of the relation on model '{}', and none was found",
            model.logical_name, field.logical_name, target.logical_name
        ))
    })?;

    if matches.next().is_some() {
        return Err(ProtocolError::QueryPlanning(format!(
            "Nested write on '{}.{}' is ambiguous: model '{}' declares more than one relation back to '{}'",
            model.logical_name, field.logical_name, target.logical_name, model.logical_name
        )));
    }

    Ok(first)
}

/// Resolve the side of `relation` and the key fields its operations work with.
pub(super) fn binding_for(
    state: &EngineState,
    model: &ModelIr,
    field: &FieldIr,
    relation: &RelationIr,
) -> Result<RelationBinding, ProtocolError> {
    let target = state.models().get(&relation.target_model).ok_or_else(|| {
        ProtocolError::InvalidModel(format!(
            "Relation '{}.{}' targets unknown model '{}'",
            model.logical_name, field.logical_name, relation.target_model
        ))
    })?;

    if let Some(join) = &relation.join {
        return Ok(RelationBinding {
            target_model: target.logical_name.clone(),
            side: RelationSide::ManyToMany,
            foreign_keys: Vec::new(),
            referenced: vec![join.self_reference.clone()],
            via: Some(join.clone()),
        });
    }

    if relation.fields.is_empty() {
        let inverse = inverse_relation(model, field, relation, target)?;
        Ok(RelationBinding {
            target_model: target.logical_name.clone(),
            side: RelationSide::Inverse,
            foreign_keys: inverse.fields.clone(),
            referenced: inverse.references.clone(),
            via: None,
        })
    } else {
        Ok(RelationBinding {
            target_model: target.logical_name.clone(),
            side: RelationSide::Owning,
            foreign_keys: relation.fields.clone(),
            referenced: relation.references.clone(),
            via: None,
        })
    }
}

/// Read the columns named by `fields` out of a row of `model`.
pub(super) fn row_key_values(
    state: &EngineState,
    model: &ModelIr,
    row: &Row,
    fields: &[String],
) -> Result<Vec<Value>, ProtocolError> {
    fields
        .iter()
        .map(|logical_name| {
            let db_name = state
                .model_metadata(model)
                .logical_to_db()
                .get(logical_name)
                .ok_or_else(|| {
                    ProtocolError::QueryPlanning(format!(
                        "Relation on model '{}' references unknown field '{}'",
                        model.logical_name, logical_name
                    ))
                })?;
            row.get(&format!("{}__{}", model.db_name, db_name))
                .cloned()
                .ok_or_else(|| {
                    ProtocolError::Internal(format!(
                        "Nested write could not read '{}.{}' back from the written row",
                        model.logical_name, logical_name
                    ))
                })
        })
        .collect()
}

/// A filter matching the rows whose `fields` hold `values`.
pub(super) fn key_filter(fields: &[String], values: &[Value]) -> JsonValue {
    let mut filter = JsonMap::with_capacity(fields.len());
    for (name, value) in fields.iter().zip(values) {
        filter.insert(name.clone(), value.to_json_plain());
    }
    JsonValue::Object(filter)
}

/// A `data` payload clearing every one of `fields`.
pub(super) fn null_key_data(fields: &[String]) -> JsonValue {
    let mut data = JsonMap::with_capacity(fields.len());
    for name in fields {
        data.insert(name.clone(), JsonValue::Null);
    }
    JsonValue::Object(data)
}
