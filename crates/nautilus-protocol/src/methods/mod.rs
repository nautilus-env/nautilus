//! Nautilus protocol method definitions.
//!
//! This module defines stable method names and their request/response payloads,
//! one module per family of methods. The names below are the public surface:
//! the modules themselves are an internal arrangement.

mod aggregate;
mod engine;
mod raw;
mod read;
mod schema;
mod transaction;
mod write;

pub use aggregate::{
    AggregateParams, CountParams, GroupByParams, QUERY_AGGREGATE, QUERY_COUNT, QUERY_GROUP_BY,
};
pub use engine::{
    EngineMetricsParams, EngineMetricsResult, HandshakeParams, HandshakeResult, MethodMetrics,
    PlanCacheMetrics, PlanCacheSectionMetrics, PoolMetrics, RequestCancelParams,
    RequestCancelResult, ENGINE_HANDSHAKE, ENGINE_METRICS, REQUEST_CANCEL,
};
pub use raw::{RawQueryParams, RawStmtQueryParams, QUERY_RAW, QUERY_RAW_STMT};
pub use read::{
    ExplainParams, ExplainResult, FindFirstOrThrowParams, FindFirstParams, FindManyParams,
    FindUniqueOrThrowParams, FindUniqueParams, QueryResult, QUERY_EXPLAIN, QUERY_FIND_FIRST,
    QUERY_FIND_FIRST_OR_THROW, QUERY_FIND_MANY, QUERY_FIND_UNIQUE, QUERY_FIND_UNIQUE_OR_THROW,
};
pub use schema::{SchemaValidateParams, SchemaValidateResult, SCHEMA_VALIDATE};
pub use transaction::{
    BatchOperation, IsolationLevel, TransactionBatchParams, TransactionBatchResult,
    TransactionCommitParams, TransactionCommitResult, TransactionRollbackParams,
    TransactionRollbackResult, TransactionStartParams, TransactionStartResult, TRANSACTION_BATCH,
    TRANSACTION_COMMIT, TRANSACTION_ROLLBACK, TRANSACTION_START,
};
pub use write::{
    CreateManyParams, CreateParams, DeleteManyParams, DeleteParams, MutationResult,
    UpdateManyParams, UpdateParams, UpsertParams, QUERY_CREATE, QUERY_CREATE_MANY, QUERY_DELETE,
    QUERY_DELETE_MANY, QUERY_UPDATE, QUERY_UPDATE_MANY, QUERY_UPSERT,
};
