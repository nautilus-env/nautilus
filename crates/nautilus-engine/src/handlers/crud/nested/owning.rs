//! The side where the written model holds the foreign key.
//!
//! The related row has to exist before the parent statement runs, so these
//! operations resolve first and contribute foreign-key columns to the payload
//! the parent writes. A `delete` is the exception in the other direction: the
//! parent still points at the row while its statement runs, so the delete waits
//! until it no longer does.
use nautilus_connector::Row;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::ModelIr;
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::binding::{key_filter, row_key_values, RelationBinding};
use super::execute::{
    create_related, delete_related, find_related, first_row, not_found, update_related,
};
use super::payload::{require_member, require_object};
use super::plan::{NestedPlan, NestedWrite};
use crate::state::EngineState;

/// Resolve the owning-side writes of `plan` and answer with the payload the
/// parent statement should write, plus the deletes it defers.
///
/// `current` is the row being updated, and is `None` on a create; the
/// operations that act on an already-connected row need it to find that row.
pub(in crate::handlers::crud) async fn prepare_parent_data(
    state: &EngineState,
    model: &ModelIr,
    plan: &NestedPlan<'_>,
    current: Option<&Row>,
    tx: &str,
) -> Result<(JsonValue, DeferredDeletes), ProtocolError> {
    let mut patch = ParentPatch::default();
    for write in &plan.owning {
        resolve_write(state, model, write, current, tx, &mut patch).await?;
    }

    Ok((
        plan.scalar_data_with(patch.assignments),
        DeferredDeletes(patch.deferred),
    ))
}

/// What the owning-side writes contribute to the parent statement.
#[derive(Default)]
struct ParentPatch {
    assignments: JsonMap<String, JsonValue>,
    deferred: Vec<DeferredDelete>,
}

impl ParentPatch {
    /// Point the parent at `row` by copying the columns the relation references.
    fn point_at(
        &mut self,
        state: &EngineState,
        binding: &RelationBinding,
        row: &Row,
    ) -> Result<(), ProtocolError> {
        let target = state
            .models()
            .get(&binding.target_model)
            .ok_or_else(|| ProtocolError::InvalidModel(binding.target_model.clone()))?;
        let values = row_key_values(state, target, row, &binding.referenced)?;
        for (name, value) in binding.foreign_keys.iter().zip(values) {
            self.assignments.insert(name.clone(), value.to_json_plain());
        }
        Ok(())
    }

    /// Leave the parent pointing at nothing through this relation.
    fn clear(&mut self, binding: &RelationBinding) {
        for name in &binding.foreign_keys {
            self.assignments.insert(name.clone(), JsonValue::Null);
        }
    }
}

async fn resolve_write(
    state: &EngineState,
    model: &ModelIr,
    write: &NestedWrite<'_>,
    current: Option<&Row>,
    tx: &str,
    patch: &mut ParentPatch,
) -> Result<(), ProtocolError> {
    let binding = &write.binding;
    for (operation, payload) in &write.operations {
        match *operation {
            "create" => {
                let rows =
                    create_related(state, &binding.target_model, (*payload).clone(), tx).await?;
                patch.point_at(state, binding, first_row(&rows, write.field_name)?)?;
            }
            "connect" => {
                let row = find_related(state, &binding.target_model, payload, tx)
                    .await?
                    .ok_or_else(|| not_found("connect", write.field_name, binding))?;
                patch.point_at(state, binding, &row)?;
            }
            "connectOrCreate" => {
                let object = require_object(payload, "connectOrCreate")?;
                let filter = require_member(object, "where", "connectOrCreate")?;
                match find_related(state, &binding.target_model, filter, tx).await? {
                    Some(row) => patch.point_at(state, binding, &row)?,
                    None => {
                        let data = require_member(object, "create", "connectOrCreate")?;
                        let rows =
                            create_related(state, &binding.target_model, data.clone(), tx).await?;
                        patch.point_at(state, binding, first_row(&rows, write.field_name)?)?;
                    }
                }
            }
            "disconnect" => patch.clear(binding),
            "update" => {
                let row = require_current(current, write.field_name, operation)?;
                let filter = connected_filter(state, model, binding, row)?;
                update_related(state, &binding.target_model, filter, (*payload).clone(), tx)
                    .await?;
            }
            "delete" => {
                let row = require_current(current, write.field_name, operation)?;
                let filter = connected_filter(state, model, binding, row)?;
                patch.clear(binding);
                patch.deferred.push(DeferredDelete {
                    model: binding.target_model.clone(),
                    filter,
                });
            }
            other => {
                return Err(ProtocolError::UnsupportedOperation(format!(
                    "Nested '{}' is not available on '{}.{}', which is the side holding the foreign key",
                    other, model.logical_name, write.field_name
                )))
            }
        }
    }

    Ok(())
}

/// A delete held back until the parent no longer references the row.
struct DeferredDelete {
    model: String,
    filter: JsonValue,
}

/// The deletes a nested write postponed until the parent stopped referencing
/// their rows.
pub(in crate::handlers::crud) struct DeferredDeletes(Vec<DeferredDelete>);

impl DeferredDeletes {
    /// Run the postponed deletes.
    pub(in crate::handlers::crud) async fn run(
        self,
        state: &EngineState,
        tx: &str,
    ) -> Result<(), ProtocolError> {
        for delete in self.0 {
            delete_related(state, &delete.model, delete.filter, tx).await?;
        }
        Ok(())
    }
}

fn require_current<'a>(
    current: Option<&'a Row>,
    field_name: &str,
    operation: &str,
) -> Result<&'a Row, ProtocolError> {
    current.ok_or_else(|| {
        ProtocolError::InvalidParams(format!(
            "Nested '{}' on '{}' needs an existing row, so it is only available on update",
            operation, field_name
        ))
    })
}

/// Filter matching the row the parent's foreign key currently points at.
fn connected_filter(
    state: &EngineState,
    model: &ModelIr,
    binding: &RelationBinding,
    current: &Row,
) -> Result<JsonValue, ProtocolError> {
    let values = row_key_values(state, model, current, &binding.foreign_keys)?;
    Ok(key_filter(&binding.referenced, &values))
}
