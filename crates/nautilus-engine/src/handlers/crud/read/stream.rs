//! Chunked delivery of `findMany` results.
//!
//! Both the row-by-row path and the buffered fallback split the result set the
//! same way: every chunk but the last leaves as a `partial: true` reply, and
//! the last one is handed back so the dispatcher emits it as the final answer.

use futures::stream::StreamExt;
use nautilus_connector::Row;
use nautilus_protocol::wire::ok_partial;
use nautilus_protocol::{ProtocolError, RpcId, RpcResponse};
use serde_json::value::RawValue;
use tokio::sync::mpsc;

use super::plan::FindManyPlan;
use crate::conversion::normalize_row_with_hints;
use crate::filter::QueryArgs;
use crate::handlers::crud::common::wrap_data_result;
use crate::state::{connector_to_protocol, EngineState};

/// Whether a parsed request can be answered as rows arrive.
///
/// Streaming is only safe when the result set needs no global transformation
/// before output: backward pagination reverses the whole `Vec<Row>`, include
/// hydration needs every parent row before it can issue the batch child query,
/// and the `distinct` fallback deduplicates across the entire result set. All
/// three fall back to [`emit_buffered_chunks`].
pub(super) fn is_streamable(state: &EngineState, query_args: &QueryArgs) -> bool {
    !query_args.backward
        && query_args.include.is_empty()
        && (query_args.distinct.is_empty() || state.dialect.supports_distinct_on())
}

async fn send_partial(
    sender: &mpsc::Sender<RpcResponse>,
    request_id: &Option<RpcId>,
    raw: Box<RawValue>,
) -> Result<(), ProtocolError> {
    sender
        .send(ok_partial(request_id.clone(), raw))
        .await
        .map_err(|_| ProtocolError::Internal("Channel closed during chunked response".to_string()))
}

/// Split an already-materialised result set into wire chunks.
///
/// The buffered paths still honour `chunkSize` even though the engine had to
/// hold every row first. Returns the last chunk for the caller to send as the
/// final reply, or `None` when there was nothing to chunk.
pub(super) async fn emit_buffered_chunks(
    rows: &[Row],
    chunk_size: usize,
    request_id: Option<RpcId>,
    sender: mpsc::Sender<RpcResponse>,
) -> Result<Option<Box<RawValue>>, ProtocolError> {
    let mut chunks = rows.chunks(chunk_size).peekable();

    while let Some(chunk) = chunks.next() {
        let raw = wrap_data_result(chunk, "findMany chunk")?;
        if chunks.peek().is_none() {
            return Ok(Some(raw));
        }
        send_partial(&sender, &request_id, raw).await?;
    }

    Ok(None)
}

/// Drive `findMany` row-by-row through the connector's owned-stream path,
/// emitting `partial: true` chunks as they fill up.
///
/// When the client sets `chunkSize` and a response channel is available, each
/// batch of at most `chunk_size` rows reaches the transport as soon as it
/// fills, instead of after the full result set is buffered. The final batch is
/// returned to the caller, so the outer dispatcher emits a non-partial reply
/// at the end. [`is_streamable`] decides which requests may take this path.
pub(super) async fn stream_find_many_chunked(
    state: &EngineState,
    plan: FindManyPlan,
    tx_id: Option<&str>,
    chunk_size: usize,
    request_id: Option<RpcId>,
    sender: mpsc::Sender<RpcResponse>,
) -> Result<Box<RawValue>, ProtocolError> {
    let mut row_stream = state
        .execute_query_stream_on(plan.sql, tx_id)
        .await
        .map_err(|e| match e {
            ProtocolError::ConnectionFailed(msg) => ProtocolError::ConnectionFailed(msg),
            other => other,
        })?;

    // `pending` holds the most recently *filled* chunk: it is held back until
    // we know whether another full chunk follows. If yes, `pending` becomes a
    // partial response on the wire; if not (i.e. it is the last chunk), it is
    // returned as the final result so the caller emits a non-partial reply.
    // `accum` collects rows until it reaches `chunk_size`.
    let mut pending: Vec<Row> = Vec::with_capacity(chunk_size);
    let mut accum: Vec<Row> = Vec::with_capacity(chunk_size);

    while let Some(item) = row_stream.next().await {
        let raw_row = item.map_err(|e| connector_to_protocol(e, "Query"))?;
        let row = normalize_row_with_hints(raw_row, &plan.row_hints)?;
        accum.push(row);
        if accum.len() >= chunk_size {
            if !pending.is_empty() {
                let raw = wrap_data_result(&pending, "findMany chunk")?;
                pending.clear();
                send_partial(&sender, &request_id, raw).await?;
            }
            std::mem::swap(&mut pending, &mut accum);
        }
    }

    // End-of-stream: at most one of `pending` (a fully-filled chunk) and
    // `accum` (partial leftover) is non-empty. Whichever holds rows last is
    // returned as the final non-partial reply; if both are non-empty, flush
    // `pending` as a partial frame first so the leftover can be the final.
    let final_chunk = if accum.is_empty() {
        pending
    } else {
        if !pending.is_empty() {
            let raw = wrap_data_result(&pending, "findMany chunk")?;
            send_partial(&sender, &request_id, raw).await?;
        }
        accum
    };

    wrap_data_result(&final_chunk, "findMany result")
}
