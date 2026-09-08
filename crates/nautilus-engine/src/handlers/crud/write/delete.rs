//! `query.delete` and `query.deleteMany`: removing the rows a filter matches.
//!
//! `deleteMany` is the same operation with `return_data` pinned off, so the
//! `RETURNING` projection is never emitted and the answer is the affected-row
//! count alone. Without `RETURNING` the rows are read before the statement
//! runs, on its transaction, since afterwards they are gone.

use nautilus_core::{Delete, DeleteCapacity};
use nautilus_protocol::{
    check_protocol_version, DeleteManyParams, DeleteParams, ProtocolError, RpcRequest,
};

use crate::handlers::crud::common::{
    ensure_single_record_filter, execute_mutation_result, parse_optional_model_filter,
    wrap_count_result, wrap_mutation_result, MutationResultData,
};
use crate::handlers::crud::read::find_rows_by_expr;
use crate::handlers::{get_writable_model_or_error, parse_params};
use crate::state::EngineState;

pub(in crate::handlers::crud) async fn execute_delete(
    state: &EngineState,
    params: DeleteParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let model = get_writable_model_or_error(state, &params.model)?;
    let tx_id = params.transaction_id;
    let metadata = state.model_metadata(model);

    let qualified_filter = parse_optional_model_filter(
        model,
        &params.filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    let returns_inline = params.return_data && state.dialect.supports_returning();

    let mut builder =
        Delete::from_table(crate::metadata::model_table(model)).with_capacity(DeleteCapacity {
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        });
    if let Some(filter) = qualified_filter.clone() {
        builder = builder.filter(filter);
    }

    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let delete = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build delete: {}", e)))?;

    let sql = state
        .dialect
        .render_delete_owned(delete)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if returns_inline || !params.return_data {
        return execute_mutation_result(
            state,
            &sql,
            "Delete",
            tx_id.as_deref(),
            metadata.scalar_hints(),
            returns_inline,
        )
        .await;
    }

    state
        .in_transaction(tx_id.as_deref(), |tx| async move {
            let rows = find_rows_by_expr(state, model, qualified_filter, None, Some(&tx)).await?;
            state.execute_affected_on(&sql, "Delete", Some(&tx)).await?;
            Ok(MutationResultData::Rows(rows))
        })
        .await
}

/// Handle `query.delete`.
pub(in crate::handlers) async fn handle_delete(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: DeleteParams = parse_params(&request, "delete")?;
    ensure_single_record_filter("delete", &params.filter)?;

    match execute_delete(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "delete result"),
        MutationResultData::Count(count) => wrap_count_result(count, "delete result"),
    }
}

/// Handle `query.deleteMany`. See [`super::update::handle_update_many`].
pub(in crate::handlers) async fn handle_delete_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: DeleteManyParams = parse_params(&request, "deleteMany")?;

    wrap_count_result(
        execute_delete_many(state, params).await?,
        "deleteMany result",
    )
}

pub(in crate::handlers) async fn handle_delete_many_typed(
    state: &EngineState,
    params: DeleteManyParams,
) -> Result<usize, ProtocolError> {
    execute_delete_many(state, params).await
}

async fn execute_delete_many(
    state: &EngineState,
    params: DeleteManyParams,
) -> Result<usize, ProtocolError> {
    Ok(execute_delete(
        state,
        DeleteParams {
            protocol_version: params.protocol_version,
            model: params.model,
            filter: params.filter,
            transaction_id: params.transaction_id,
            return_data: false,
        },
    )
    .await?
    .into_count())
}
