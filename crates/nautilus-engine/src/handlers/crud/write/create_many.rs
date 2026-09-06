//! `query.createMany`: many rows in one `INSERT`.
//!
//! One statement means one column list, so every row has to resolve to the
//! same set of columns. Which columns those are is decided by the first row,
//! after dropping the fields whose explicit `null` asks the database for its
//! own default; a later row that resolves differently is rejected rather than
//! silently written with the wrong columns.

use nautilus_connector::Row;
use nautilus_core::{Insert, InsertCapacity, Value};
use nautilus_protocol::{CreateManyParams, ProtocolError, RpcRequest};
use nautilus_schema::ir::{FieldIr, ModelIr, ResolvedFieldType};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::input::{
    ensure_known_data_keys, field_input_value, row_field_json, should_omit_server_default,
    FieldInputMode,
};
use crate::conversion::check_protocol_version;
use crate::handlers::crud::common::{
    execute_mutation_result, wrap_count_result, wrap_mutation_result, MutationResultData,
};
use crate::handlers::{field_marker, get_writable_model_or_error, parse_params};
use crate::state::EngineState;

/// The columns one row of a `createMany` actually writes.
///
/// Every row has to resolve to the same list, which is what makes them
/// shareable by a single statement.
fn create_many_effective_fields<'a>(
    model: &'a ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Vec<&'a FieldIr> {
    model
        .fields
        .iter()
        .filter(|field| !matches!(field.field_type, ResolvedFieldType::Relation(_)))
        .filter(|field| {
            row_field_json(data_obj, field)
                .is_some_and(|json_val| !should_omit_server_default(json_val, field))
        })
        .collect()
}

async fn execute_create_many(
    state: &EngineState,
    params: CreateManyParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_writable_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);

    if params.data.is_empty() {
        return Err(ProtocolError::InvalidParams(
            "data array cannot be empty".to_string(),
        ));
    }

    let first_obj = params.data[0]
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data items must be objects".to_string()))?;

    let relevant_fields = create_many_effective_fields(model, first_obj);
    let expected_keys: Vec<&str> = relevant_fields
        .iter()
        .map(|field| field.logical_name.as_str())
        .collect();
    let expected_key_set: std::collections::HashSet<&str> = expected_keys.iter().copied().collect();

    let columns: Vec<_> = relevant_fields
        .iter()
        .map(|field| field_marker(model, field))
        .collect();

    let mut all_values = Vec::with_capacity(params.data.len());
    for (row_idx, json_value) in params.data.iter().enumerate() {
        let data_obj = json_value.as_object().ok_or_else(|| {
            ProtocolError::InvalidParams("data items must be objects".to_string())
        })?;
        ensure_known_data_keys(model, data_obj)?;

        let row_fields = create_many_effective_fields(model, data_obj);
        let row_keys: Vec<&str> = row_fields
            .iter()
            .map(|field| field.logical_name.as_str())
            .collect();

        if row_keys != expected_keys {
            let row_key_set: std::collections::HashSet<&str> = row_keys.iter().copied().collect();
            let missing: Vec<&str> = expected_keys
                .iter()
                .copied()
                .filter(|key| !row_key_set.contains(key))
                .collect();
            let extra: Vec<&str> = row_keys
                .iter()
                .copied()
                .filter(|key| !expected_key_set.contains(key))
                .collect();
            return Err(ProtocolError::InvalidParams(format!(
                "createMany rows must use the same key set after omitting server defaults; row {} differs from row 0 (missing: [{}], extra: [{}])",
                row_idx,
                missing.join(", "),
                extra.join(", "),
            )));
        }

        let mut row_values = Vec::with_capacity(relevant_fields.len());
        for field in &relevant_fields {
            if let Some(value) = field_input_value(state, data_obj, field, FieldInputMode::Create)?
            {
                row_values.push(value);
            } else {
                row_values.push(Value::Null);
            }
        }
        all_values.push(row_values);
    }

    let mut builder = Insert::into_table(crate::metadata::model_table(model))
        .with_capacity(InsertCapacity {
            columns: columns.len(),
            rows: all_values.len(),
            returning: usize::from(params.return_data) * metadata.scalar_markers().len(),
        })
        .columns(columns)
        .rows(all_values);
    if params.return_data {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build insert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert_owned(insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    execute_mutation_result(
        state,
        &sql,
        "Insert",
        tx_id.as_deref(),
        metadata.scalar_hints(),
        params.return_data,
    )
    .await
}

/// Handle `query.createMany`.
pub(in crate::handlers) async fn handle_create_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CreateManyParams = parse_params(&request, "createMany")?;

    match execute_create_many(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "createMany result"),
        MutationResultData::Count(count) => wrap_count_result(count, "createMany result"),
    }
}

pub(in crate::handlers) async fn handle_create_many_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: CreateManyParams = parse_params(&request, "createMany")?;
    execute_create_many(state, params)
        .await?
        .into_rows("createMany")
}

pub(in crate::handlers) async fn handle_create_many_typed(
    state: &EngineState,
    params: CreateManyParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_create_many(state, params)
        .await?
        .into_rows("createMany")
}
