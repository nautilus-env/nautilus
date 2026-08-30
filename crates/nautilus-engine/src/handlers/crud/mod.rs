//! CRUD query handlers: findMany, findFirst, findUnique, create, createMany, update, delete
//! and their `*OrThrow` variants.

use nautilus_connector::Row;
use nautilus_core::{
    build_cursor_predicate, Delete, DeleteCapacity, Expr, Insert, InsertCapacity, OrderDir, Select,
    SelectCapacity, SelectItem, Update, UpdateCapacity, Value,
};
use nautilus_dialect::Sql;
use nautilus_protocol::wire::ok_partial;
use nautilus_protocol::{
    AggregateParams, CountParams, CreateManyParams, CreateParams, DeleteManyParams, DeleteParams,
    ExplainParams, FindFirstParams, FindManyParams, FindUniqueParams, GroupByParams, ProtocolError,
    RpcRequest, RpcResponse, UpdateManyParams, UpdateParams, UpsertParams,
};
use nautilus_schema::ir::{DefaultValue, FieldIr, ModelIr, ResolvedFieldType};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::sync::mpsc;

use super::{field_marker, get_model_or_error, parse_params};
use crate::conversion::{
    check_protocol_version, json_to_value, json_to_value_field, normalize_rows_with_hints,
    rows_to_raw_json, ValueHint,
};
use crate::filter::{parse_group_by_order_by, parse_having, qualify_filter_columns, QueryArgs};
use crate::state::EngineState;

mod aggregation;
mod common;
pub(crate) mod include;
mod mutations;
mod nested;
mod raw;
mod read;

pub(super) use aggregation::{
    handle_aggregate, handle_aggregate_typed, handle_group_by, handle_group_by_embedded,
    handle_group_by_typed,
};
pub(super) use mutations::{
    handle_create, handle_create_embedded, handle_create_many, handle_create_many_embedded,
    handle_create_many_typed, handle_create_typed, handle_delete, handle_delete_many,
    handle_delete_many_typed, handle_update, handle_update_embedded, handle_update_many,
    handle_update_many_typed, handle_update_typed, handle_upsert, handle_upsert_embedded,
    handle_upsert_typed,
};
pub(super) use raw::{handle_raw_query, handle_raw_stmt_query};
pub(super) use read::{
    execute_explain_typed as handle_explain_typed,
    execute_find_many_typed as handle_find_many_typed,
    execute_find_unique_typed as handle_find_unique_typed, handle_count, handle_count_embedded,
    handle_count_typed, handle_explain, handle_find_first, handle_find_first_or_throw,
    handle_find_many, handle_find_many_embedded, handle_find_unique, handle_find_unique_or_throw,
};
