//! Every nested operation reaches the database through the handler a top-level
//! request would use, on the caller's transaction.
//!
//! That is what makes value conversion, defaults, `RETURNING` handling and one
//! more level of nesting shared with the flat paths instead of reimplemented
//! for children.
use nautilus_connector::Row;
use nautilus_protocol::{
    CreateParams, DeleteParams, ProtocolError, UpdateParams, PROTOCOL_VERSION,
};
use serde_json::Value as JsonValue;

use super::binding::RelationBinding;
use super::payload::unwrap_where;
use crate::handlers::crud::read::{find_all_rows_by_filter, find_one_row};
use crate::handlers::crud::write::{execute_create, execute_delete, execute_update};
use crate::state::EngineState;

pub(super) async fn create_related(
    state: &EngineState,
    target_model: &str,
    data: JsonValue,
    tx: &str,
) -> Result<Vec<Row>, ProtocolError> {
    let params = CreateParams {
        protocol_version: PROTOCOL_VERSION,
        model: target_model.to_string(),
        data,
        transaction_id: Some(tx.to_string()),
        return_data: true,
    };
    Box::pin(execute_create(state, params))
        .await?
        .into_rows("nested create")
}

pub(super) async fn update_related(
    state: &EngineState,
    target_model: &str,
    filter: JsonValue,
    data: JsonValue,
    tx: &str,
) -> Result<usize, ProtocolError> {
    let params = UpdateParams {
        protocol_version: PROTOCOL_VERSION,
        model: target_model.to_string(),
        filter,
        data,
        transaction_id: Some(tx.to_string()),
        return_data: false,
    };
    Ok(Box::pin(execute_update(state, params)).await?.into_count())
}

pub(super) async fn delete_related(
    state: &EngineState,
    target_model: &str,
    filter: JsonValue,
    tx: &str,
) -> Result<usize, ProtocolError> {
    let params = DeleteParams {
        protocol_version: PROTOCOL_VERSION,
        model: target_model.to_string(),
        filter,
        transaction_id: Some(tx.to_string()),
        return_data: false,
    };
    Ok(Box::pin(execute_delete(state, params)).await?.into_count())
}

/// The first row `filter` selects, which is how `connect` finds its target.
pub(super) async fn find_related(
    state: &EngineState,
    target_model: &str,
    filter: &JsonValue,
    tx: &str,
) -> Result<Option<Row>, ProtocolError> {
    let target = model_or_error(state, target_model)?;
    find_one_row(state, target, &unwrap_where(filter), Some(tx)).await
}

/// Every row `filter` selects, already narrowed to the parent's children.
pub(super) async fn find_all_related(
    state: &EngineState,
    target_model: &str,
    filter: &JsonValue,
    tx: &str,
) -> Result<Vec<Row>, ProtocolError> {
    let target = model_or_error(state, target_model)?;
    find_all_rows_by_filter(state, target, filter, Some(tx)).await
}

fn model_or_error<'a>(
    state: &'a EngineState,
    target_model: &str,
) -> Result<&'a nautilus_schema::ir::ModelIr, ProtocolError> {
    state
        .models()
        .get(target_model)
        .ok_or_else(|| ProtocolError::InvalidModel(target_model.to_string()))
}

/// The row a nested create was supposed to produce.
pub(super) fn first_row<'a>(rows: &'a [Row], field_name: &str) -> Result<&'a Row, ProtocolError> {
    rows.first().ok_or_else(|| {
        ProtocolError::Internal(format!(
            "Nested create on '{}' returned no row to link the parent to",
            field_name
        ))
    })
}

/// The error for an operation that named a row the relation does not reach.
pub(super) fn not_found(
    operation: &str,
    field_name: &str,
    binding: &RelationBinding,
) -> ProtocolError {
    ProtocolError::RecordNotFound(format!(
        "Nested {} on '{}' matched no '{}' record",
        operation, field_name, binding.target_model
    ))
}
