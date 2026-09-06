//! The relations where neither model holds a foreign key.
//!
//! The links live in a join table Nautilus owns, so these operations run after
//! the parent statement and add or remove rows there instead of writing a
//! foreign key anywhere. The parent row exists by the time this runs, so every
//! operation is at most two writes: the child row itself, and the link. The
//! operations that reach existing children resolve the relation's current
//! members first and narrow to them, which is what keeps a caller-supplied
//! `where` from reaching a row linked to a different parent.
use nautilus_connector::Row;
use nautilus_core::Value;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{ManyToManyJoinIr, ModelIr};
use serde_json::Value as JsonValue;

use super::binding::{row_key_values, RelationBinding};
use super::execute::{
    create_related, delete_related, find_all_related, find_related, first_row, not_found,
    update_related,
};
use super::payload::{child_filters, payload_items, require_member, require_object, scoped_filter};
use super::plan::NestedWrite;
use crate::handlers::crud::read::{find_all_rows_by_filter, find_one_row};
use crate::state::EngineState;

/// Run the operations of one many-to-many relation.
pub(super) async fn apply(
    state: &EngineState,
    model: &ModelIr,
    write: &NestedWrite<'_>,
    parent: &Row,
    tx: &str,
) -> Result<(), ProtocolError> {
    let binding = &write.binding;
    let join = binding
        .via
        .as_ref()
        .expect("a many-to-many binding always carries its join table");
    let parent_key = row_key_values(state, model, parent, &binding.referenced)?
        .into_iter()
        .next()
        .expect("a many-to-many binding references exactly one key field");

    for (operation, payload) in &write.operations {
        match *operation {
            "create" => {
                for item in payload_items(payload) {
                    let child =
                        create_one(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "createMany" => {
                let object = require_object(payload, "createMany")?;
                let items = require_member(object, "data", "createMany")?;
                for item in payload_items(items) {
                    let child =
                        create_one(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "connect" => {
                for item in payload_items(payload) {
                    let child =
                        connected_key(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "connectOrCreate" => {
                for item in payload_items(payload) {
                    let object = require_object(item, "connectOrCreate")?;
                    let filter = require_member(object, "where", "connectOrCreate")?;
                    let child = match find_related(state, &binding.target_model, filter, tx).await?
                    {
                        Some(child) => child_key_value(state, binding, join, &child)?,
                        None => {
                            let data = require_member(object, "create", "connectOrCreate")?;
                            create_one(state, binding, join, write.field_name, data, tx).await?
                        }
                    };
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "disconnect" => {
                let scope = member_filter(join, &linked_keys(state, join, &parent_key, tx).await?);
                for filter in child_filters(payload, &scope) {
                    let matched =
                        find_all_related(state, &binding.target_model, &filter, tx).await?;
                    unlink_children(state, binding, join, &parent_key, &matched, tx).await?;
                }
            }
            "set" => {
                unlink_all(state, join, &parent_key, tx).await?;
                for item in payload_items(payload) {
                    let child =
                        connected_key(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "update" | "updateMany" => {
                let scope = member_filter(join, &linked_keys(state, join, &parent_key, tx).await?);
                for item in payload_items(payload) {
                    let object = require_object(item, operation)?;
                    let data = require_member(object, "data", operation)?;
                    let filter = scoped_filter(scope.clone(), object.get("where"));
                    let affected =
                        update_related(state, &binding.target_model, filter, data.clone(), tx)
                            .await?;
                    if *operation == "update" && affected == 0 {
                        return Err(not_found("update", write.field_name, binding));
                    }
                }
            }
            "delete" | "deleteMany" => {
                let scope = member_filter(join, &linked_keys(state, join, &parent_key, tx).await?);
                for filter in child_filters(payload, &scope) {
                    let affected = delete_related(state, &binding.target_model, filter, tx).await?;
                    if *operation == "delete" && affected == 0 {
                        return Err(not_found("delete", write.field_name, binding));
                    }
                }
            }
            other => {
                return Err(ProtocolError::UnsupportedOperation(format!(
                    "Nested '{}' is not available on '{}.{}'",
                    other, model.logical_name, write.field_name
                )))
            }
        }
    }

    Ok(())
}

/// Create one child of the relation and answer with the key the join table
/// stores for it.
async fn create_one(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    field_name: &str,
    data: &JsonValue,
    tx: &str,
) -> Result<Value, ProtocolError> {
    let rows = create_related(state, &binding.target_model, data.clone(), tx).await?;
    let child = first_row(&rows, field_name)?;
    child_key_value(state, binding, join, child)
}

/// Resolve an existing child by the filter a `connect` or `set` names.
async fn connected_key(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    field_name: &str,
    filter: &JsonValue,
    tx: &str,
) -> Result<Value, ProtocolError> {
    let child = find_related(state, &binding.target_model, filter, tx)
        .await?
        .ok_or_else(|| not_found("connect", field_name, binding))?;
    child_key_value(state, binding, join, &child)
}

/// Read the key the join table stores for a child row.
fn child_key_value(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    child: &Row,
) -> Result<Value, ProtocolError> {
    let target = state
        .models()
        .get(&binding.target_model)
        .ok_or_else(|| ProtocolError::InvalidModel(binding.target_model.clone()))?;
    row_key_values(
        state,
        target,
        child,
        std::slice::from_ref(&join.target_reference),
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        ProtocolError::Internal("Many-to-many link could not read the child's key back".to_string())
    })
}

/// The model Nautilus synthesised for the links of this relation.
fn join_model<'a>(
    state: &'a EngineState,
    join: &ManyToManyJoinIr,
) -> Result<&'a ModelIr, ProtocolError> {
    state.models().get(&join.table).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!("Join table '{}' not found", join.table))
    })
}

/// Read one column out of a row of the join table.
fn join_column(join: &ManyToManyJoinIr, row: &Row, column: &str) -> Option<Value> {
    row.get(&format!("{}__{}", join.table, column)).cloned()
}

/// The keys of every child currently linked to `parent_key`.
async fn linked_keys(
    state: &EngineState,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    tx: &str,
) -> Result<Vec<JsonValue>, ProtocolError> {
    let model = join_model(state, join)?;
    let filter = serde_json::json!({ &join.self_column: parent_key.to_json_plain() });
    let rows = find_all_rows_by_filter(state, model, &filter, Some(tx)).await?;

    Ok(rows
        .iter()
        .filter_map(|row| join_column(join, row, &join.target_column))
        .map(|value| value.to_json_plain())
        .collect())
}

/// A filter matching exactly the children the relation currently holds.
///
/// An empty member list stays expressible on purpose: narrowing to nothing is
/// the right answer for an operation aimed at an empty relation, and it is what
/// makes a `where` supplied by the caller unable to widen the reach.
fn member_filter(join: &ManyToManyJoinIr, members: &[JsonValue]) -> JsonValue {
    serde_json::json!({
        &join.target_reference: { "in": JsonValue::Array(members.to_vec()) }
    })
}

/// Link `child` to the parent, unless the two are linked already.
///
/// Connecting twice is not an error the caller can act on — the relation ends
/// up the same either way — so the second link is skipped rather than left to
/// violate the join table's primary key.
async fn link_child(
    state: &EngineState,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    child_key: &Value,
    tx: &str,
) -> Result<(), ProtocolError> {
    let model = join_model(state, join)?;
    let link = serde_json::json!({
        &join.self_column: parent_key.to_json_plain(),
        &join.target_column: child_key.to_json_plain(),
    });

    if find_one_row(state, model, &link, Some(tx)).await?.is_some() {
        return Ok(());
    }

    create_related(state, &model.logical_name, link, tx).await?;
    Ok(())
}

/// Drop the links between the parent and each of `children`.
async fn unlink_children(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    children: &[Row],
    tx: &str,
) -> Result<(), ProtocolError> {
    if children.is_empty() {
        return Ok(());
    }

    let keys: Vec<JsonValue> = children
        .iter()
        .map(|child| child_key_value(state, binding, join, child).map(|key| key.to_json_plain()))
        .collect::<Result<_, _>>()?;

    let filter = serde_json::json!({
        &join.self_column: parent_key.to_json_plain(),
        &join.target_column: { "in": JsonValue::Array(keys) },
    });
    delete_related(state, &join.table, filter, tx).await?;
    Ok(())
}

/// Drop every link the parent holds through this relation.
async fn unlink_all(
    state: &EngineState,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    tx: &str,
) -> Result<(), ProtocolError> {
    let filter = serde_json::json!({ &join.self_column: parent_key.to_json_plain() });
    delete_related(state, &join.table, filter, tx).await?;
    Ok(())
}
