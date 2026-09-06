//! The side where the related model holds the foreign key.
//!
//! The parent row has to exist before its children can point at it, so these
//! operations run after the parent statement. Every one of them is scoped to
//! the parent's key, so a filter supplied by the caller can only narrow the
//! rows reached through the relation, never widen them to rows belonging to
//! another parent.
use nautilus_connector::Row;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::ModelIr;
use serde_json::Value as JsonValue;

use super::binding::{key_filter, null_key_data, row_key_values, RelationBinding, RelationSide};
use super::execute::{create_related, delete_related, find_related, not_found, update_related};
use super::many_to_many;
use super::payload::{
    child_filters, merge_link, payload_items, require_member, require_object, scoped_filter,
    unwrap_where,
};
use super::plan::{NestedPlan, NestedWrite};
use crate::state::EngineState;

/// Run the writes of `plan` whose rows point at the parent row just written.
pub(in crate::handlers::crud) async fn apply_children(
    state: &EngineState,
    model: &ModelIr,
    plan: &NestedPlan<'_>,
    parent: &Row,
    tx: &str,
) -> Result<(), ProtocolError> {
    for write in &plan.inverse {
        if write.binding.side == RelationSide::ManyToMany {
            many_to_many::apply(state, model, write, parent, tx).await?;
        } else {
            apply_write(state, model, write, parent, tx).await?;
        }
    }

    Ok(())
}

async fn apply_write(
    state: &EngineState,
    model: &ModelIr,
    write: &NestedWrite<'_>,
    parent: &Row,
    tx: &str,
) -> Result<(), ProtocolError> {
    let binding = &write.binding;
    let parent_key = row_key_values(state, model, parent, &binding.referenced)?;
    let link = key_filter(&binding.foreign_keys, &parent_key);

    for (operation, payload) in &write.operations {
        match *operation {
            "create" => {
                for item in payload_items(payload) {
                    let data = merge_link(item, &link, write.field_name)?;
                    create_related(state, &binding.target_model, data, tx).await?;
                }
            }
            "createMany" => {
                let object = require_object(payload, "createMany")?;
                let rows = require_member(object, "data", "createMany")?;
                for item in payload_items(rows) {
                    let data = merge_link(item, &link, write.field_name)?;
                    create_related(state, &binding.target_model, data, tx).await?;
                }
            }
            "connect" => {
                for item in payload_items(payload) {
                    connect_child(state, binding, write.field_name, item, &link, tx).await?;
                }
            }
            "connectOrCreate" => {
                for item in payload_items(payload) {
                    let object = require_object(item, "connectOrCreate")?;
                    let filter = require_member(object, "where", "connectOrCreate")?;
                    if find_related(state, &binding.target_model, filter, tx)
                        .await?
                        .is_some()
                    {
                        connect_child(state, binding, write.field_name, filter, &link, tx).await?;
                    } else {
                        let create = require_member(object, "create", "connectOrCreate")?;
                        let data = merge_link(create, &link, write.field_name)?;
                        create_related(state, &binding.target_model, data, tx).await?;
                    }
                }
            }
            "disconnect" => {
                let nulls = null_key_data(&binding.foreign_keys);
                for filter in child_filters(payload, &link) {
                    update_related(state, &binding.target_model, filter, nulls.clone(), tx).await?;
                }
            }
            "set" => {
                let nulls = null_key_data(&binding.foreign_keys);
                update_related(state, &binding.target_model, link.clone(), nulls, tx).await?;
                for item in payload_items(payload) {
                    connect_child(state, binding, write.field_name, item, &link, tx).await?;
                }
            }
            "update" | "updateMany" => {
                for item in payload_items(payload) {
                    let object = require_object(item, operation)?;
                    let data = require_member(object, "data", operation)?;
                    let filter = scoped_filter(link.clone(), object.get("where"));
                    let affected =
                        update_related(state, &binding.target_model, filter, data.clone(), tx)
                            .await?;
                    if *operation == "update" && affected == 0 {
                        return Err(not_found("update", write.field_name, binding));
                    }
                }
            }
            "delete" | "deleteMany" => {
                for filter in child_filters(payload, &link) {
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

/// Point an existing child at the parent by writing the link into it.
async fn connect_child(
    state: &EngineState,
    binding: &RelationBinding,
    field_name: &str,
    filter: &JsonValue,
    link: &JsonValue,
    tx: &str,
) -> Result<(), ProtocolError> {
    let affected = update_related(
        state,
        &binding.target_model,
        unwrap_where(filter),
        link.clone(),
        tx,
    )
    .await?;

    if affected == 0 {
        return Err(not_found("connect", field_name, binding));
    }
    Ok(())
}
