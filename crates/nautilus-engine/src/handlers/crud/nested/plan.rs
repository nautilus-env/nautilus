//! Splitting a `data` payload into column entries and nested writes.
//!
//! This is the only place that reads operation names off the wire: everything
//! downstream works with the canonical spellings in [`CREATE_OPERATIONS`] and
//! [`UPDATE_ONLY_OPERATIONS`], already ordered so removals run before
//! additions.
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{FieldIr, ModelIr};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::binding::{binding_for, relation_field, RelationBinding, RelationSide};
use crate::state::EngineState;

/// The operations that create or attach a related row, legal on any write.
const CREATE_OPERATIONS: &[&str] = &["create", "createMany", "connect", "connectOrCreate"];
/// The operations that need a row to already exist, so only an update has them.
const UPDATE_ONLY_OPERATIONS: &[&str] = &[
    "disconnect",
    "set",
    "update",
    "updateMany",
    "delete",
    "deleteMany",
];

/// One relation field of a `data` payload together with its parsed operations.
pub(super) struct NestedWrite<'a> {
    pub(super) field_name: &'a str,
    pub(super) binding: RelationBinding,
    pub(super) operations: Vec<(&'static str, &'a JsonValue)>,
}

/// A `data` payload split into the columns of the written model and the nested
/// writes that run around the statement for it.
pub(in crate::handlers::crud) struct NestedPlan<'a> {
    scalar_data: JsonMap<String, JsonValue>,
    pub(super) owning: Vec<NestedWrite<'a>>,
    pub(super) inverse: Vec<NestedWrite<'a>>,
}

impl NestedPlan<'_> {
    /// Whether the payload was plain column data, with no relation entries.
    pub(in crate::handlers::crud) fn is_empty(&self) -> bool {
        self.owning.is_empty() && self.inverse.is_empty()
    }

    /// Whether any nested write needs the written row's key.
    pub(in crate::handlers::crud) fn writes_children(&self) -> bool {
        !self.inverse.is_empty()
    }

    /// The column entries plus the foreign keys the owning-side writes resolved.
    pub(super) fn scalar_data_with(&self, assignments: JsonMap<String, JsonValue>) -> JsonValue {
        let mut data = self.scalar_data.clone();
        for (key, value) in assignments {
            data.insert(key, value);
        }
        JsonValue::Object(data)
    }
}

/// Split a `data` payload into its column entries and its nested writes.
///
/// `allow_update_operations` admits the operations that only make sense against
/// a row that already exists; a `create` rejects them.
pub(in crate::handlers::crud) fn split<'a>(
    state: &EngineState,
    model: &'a ModelIr,
    data: &'a JsonValue,
    allow_update_operations: bool,
) -> Result<NestedPlan<'a>, ProtocolError> {
    let data_obj = data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;

    let mut plan = NestedPlan {
        scalar_data: JsonMap::new(),
        owning: Vec::new(),
        inverse: Vec::new(),
    };

    for (key, value) in data_obj {
        let Some((field, relation)) = relation_field(model, key) else {
            plan.scalar_data.insert(key.clone(), value.clone());
            continue;
        };

        let operations = parse_operations(model, field, value, allow_update_operations)?;
        if operations.is_empty() {
            continue;
        }

        let write = NestedWrite {
            field_name: field.logical_name.as_str(),
            binding: binding_for(state, model, field, relation)?,
            operations,
        };

        match write.binding.side {
            RelationSide::Owning => plan.owning.push(write),
            RelationSide::Inverse | RelationSide::ManyToMany => plan.inverse.push(write),
        }
    }

    Ok(plan)
}

fn parse_operations<'a>(
    model: &ModelIr,
    field: &FieldIr,
    value: &'a JsonValue,
    allow_update_operations: bool,
) -> Result<Vec<(&'static str, &'a JsonValue)>, ProtocolError> {
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::InvalidParams(format!(
            "'{}.{}' is a relation, so its data entry must be an object of nested-write operations",
            model.logical_name, field.logical_name
        ))
    })?;

    let mut operations = Vec::with_capacity(object.len());
    for (name, payload) in object {
        let canonical = canonical_operation(name).ok_or_else(|| {
            ProtocolError::InvalidParams(format!(
                "Unknown nested-write operation '{}' on '{}.{}'; supported operations are {}",
                name,
                model.logical_name,
                field.logical_name,
                supported_operations(allow_update_operations)
            ))
        })?;

        if !allow_update_operations && UPDATE_ONLY_OPERATIONS.contains(&canonical) {
            return Err(ProtocolError::InvalidParams(format!(
                "Nested '{}' on '{}.{}' needs an existing row, so it is only available on update",
                canonical, model.logical_name, field.logical_name
            )));
        }

        operations.push((canonical, payload));
    }

    operations.sort_by_key(|(name, _)| operation_order(name));
    Ok(operations)
}

/// Accept an operation name in either the wire spelling or the snake_case one a
/// Python or Rust caller would write, and answer with the wire spelling.
fn canonical_operation(name: &str) -> Option<&'static str> {
    let flattened: String = name
        .chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    CREATE_OPERATIONS
        .iter()
        .chain(UPDATE_ONLY_OPERATIONS)
        .find(|candidate| {
            candidate
                .chars()
                .flat_map(char::to_lowercase)
                .eq(flattened.chars())
        })
        .copied()
}

fn supported_operations(allow_update_operations: bool) -> String {
    let names: Vec<&str> = if allow_update_operations {
        CREATE_OPERATIONS
            .iter()
            .chain(UPDATE_ONLY_OPERATIONS)
            .copied()
            .collect()
    } else {
        CREATE_OPERATIONS.to_vec()
    };
    names.join(", ")
}

/// Order the operations of one relation so that removals run before additions.
///
/// `set` replaces the members of a relation wholesale, so it has to clear the
/// old ones before `connect` or `create` adds the new ones.
fn operation_order(name: &str) -> u8 {
    match name {
        "set" => 0,
        "disconnect" => 1,
        "delete" | "deleteMany" => 2,
        "update" | "updateMany" => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_operation, operation_order};

    #[test]
    fn operation_names_are_accepted_in_both_spellings() {
        assert_eq!(
            canonical_operation("connectOrCreate"),
            Some("connectOrCreate")
        );
        assert_eq!(
            canonical_operation("connect_or_create"),
            Some("connectOrCreate")
        );
        assert_eq!(canonical_operation("deleteMany"), Some("deleteMany"));
        assert_eq!(canonical_operation("nope"), None);
    }

    #[test]
    fn removals_are_ordered_before_additions() {
        let mut operations = vec!["create", "set", "connect", "disconnect", "deleteMany"];
        operations.sort_by_key(|name| operation_order(name));
        assert_eq!(
            operations,
            vec!["set", "disconnect", "deleteMany", "create", "connect"]
        );
    }
}
