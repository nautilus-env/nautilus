//! Include hydration: load relation children for a set of parent rows and attach
//! them to each row as a `<field>_json` column.
//!
//! Two execution strategies coexist:
//!
//! - **Batched**: a single `WHERE child_fk IN (parent_pks...)` query loads
//!   children for every parent at once, then groups them in memory. This
//!   eliminates the N+1 query pattern on includes without per-parent pagination.
//! - **Per-parent fallback**: used when the include node carries `take`/`skip`,
//!   because per-parent pagination cannot be expressed by a single batched query
//!   without window functions. Each parent triggers its own child query.
//!
//! Nested includes recurse through `execute_find_many_rows`, so each nesting
//! level is itself batched whenever it qualifies.
use std::collections::hash_map::Entry;
use std::collections::HashMap;

use futures::stream::{self, StreamExt, TryStreamExt};
use nautilus_connector::Row;
use nautilus_core::{Expr, Value};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::ModelIr;
use serde_json::Value as JsonValue;

use super::read::execute_find_many_rows;
use crate::filter::{IncludeNode, QueryArgs, RelationInfo};
use crate::state::EngineState;

/// Concurrency cap for the per-parent fallback path (include with
/// `take`/`skip`). Bounds how many of the pool's connections one hydration
/// can occupy at a time; sibling relations on top of this are few in
/// practice, and the pool itself backstops the total.
const PER_PARENT_CONCURRENCY: usize = 8;

fn include_alias(field_name: &str) -> String {
    format!("{}_json", field_name)
}

fn parent_join_column(rel_info: &RelationInfo) -> String {
    format!("{}__{}", rel_info.parent_table, rel_info.pk_db)
}

fn child_join_column(rel_info: &RelationInfo) -> String {
    format!("{}__{}", rel_info.target_table, rel_info.fk_db)
}

fn empty_include_value(is_array: bool) -> Value {
    if is_array {
        Value::Json(JsonValue::Array(vec![]))
    } else {
        Value::Null
    }
}

fn empty_relation_value(rel_info: &RelationInfo) -> Value {
    empty_include_value(rel_info.is_array)
}

/// Lightweight grouping key derived from a PK/FK `Value` without rendering it
/// to JSON. Integer widths are normalized so `I32(5)` and `I64(5)` group
/// together, matching the historic JSON-string key behavior.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupKey {
    Int(i64),
    Uuid(uuid::Uuid),
    Str(String),
}

/// Stable key for grouping by a SQL value. PKs are I32/I64/Uuid/String in
/// real schemas; anything else falls back to its plain-JSON rendering.
pub fn group_key(value: &Value) -> GroupKey {
    match value {
        Value::I32(v) => GroupKey::Int(i64::from(*v)),
        Value::I64(v) => GroupKey::Int(*v),
        Value::Uuid(v) => GroupKey::Uuid(*v),
        Value::String(v) => GroupKey::Str(v.clone()),
        Value::Enum { value, .. } => GroupKey::Str(value.clone()),
        other => GroupKey::Str(other.to_json_plain().to_string()),
    }
}

/// Precomputed projection for converting child rows into JSON objects: one
/// `(qualified column alias, logical field name)` pair per scalar field,
/// built once per relation load instead of re-formatting the alias for every
/// cell of every row.
pub struct IncludeProjection {
    fields: Vec<(String, String)>,
}

impl IncludeProjection {
    /// Build a projection from explicit `(alias, logical name)` pairs.
    pub fn new(fields: Vec<(String, String)>) -> Self {
        Self { fields }
    }

    /// Build the projection for a model's scalar fields.
    pub fn for_model(model: &ModelIr) -> Self {
        Self {
            fields: model
                .scalar_fields()
                .map(|field| {
                    (
                        format!("{}__{}", model.db_name, field.db_name),
                        field.logical_name.clone(),
                    )
                })
                .collect(),
        }
    }
}

/// Convert a child row into the JSON object attached to its parent, mapping
/// qualified column aliases back to logical field names and carrying through
/// any nested `<field>_json` include payloads.
pub fn row_to_json_value(projection: &IncludeProjection, row: &Row) -> JsonValue {
    let mut obj = serde_json::Map::with_capacity(row.len());
    for (alias, logical_name) in &projection.fields {
        if let Some(value) = row.get(alias) {
            obj.insert(logical_name.clone(), value.to_json_plain());
        }
    }
    for (name, value) in row.iter() {
        if name.ends_with("_json") {
            obj.insert(name.to_string(), value.to_json_plain());
        }
    }
    JsonValue::Object(obj)
}

/// Group `child_rows` by FK and produce the per-parent include value, in the
/// same order as `row_keys`. `key_counts` carries how many parents share each
/// key so the grouped children can be moved out (not cloned) on the last use;
/// cloning only happens for keys genuinely shared by multiple parents.
pub fn build_include_values(
    row_keys: Vec<Option<GroupKey>>,
    mut key_counts: HashMap<GroupKey, usize>,
    child_rows: &[Row],
    child_join: &str,
    projection: &IncludeProjection,
    is_array: bool,
) -> Vec<Value> {
    let mut grouped: HashMap<GroupKey, Vec<JsonValue>> = HashMap::with_capacity(key_counts.len());
    for child_row in child_rows {
        let Some(fk_value) = child_row.get(child_join) else {
            continue;
        };
        grouped
            .entry(group_key(fk_value))
            .or_default()
            .push(row_to_json_value(projection, child_row));
    }

    row_keys
        .into_iter()
        .map(|maybe_key| {
            let Some(key) = maybe_key else {
                return empty_include_value(is_array);
            };
            let shared = key_counts
                .get_mut(&key)
                .map(|count| {
                    *count -= 1;
                    *count > 0
                })
                .unwrap_or(false);
            if is_array {
                let children = if shared {
                    grouped.get(&key).cloned().unwrap_or_default()
                } else {
                    grouped.remove(&key).unwrap_or_default()
                };
                Value::Json(JsonValue::Array(children))
            } else if shared {
                grouped
                    .get(&key)
                    .and_then(|children| children.first().cloned())
                    .map(Value::Json)
                    .unwrap_or(Value::Null)
            } else {
                grouped
                    .remove(&key)
                    .into_iter()
                    .flatten()
                    .next()
                    .map(Value::Json)
                    .unwrap_or(Value::Null)
            }
        })
        .collect()
}

/// Single-parent fallback path. Used when the batched path cannot run safely
/// (e.g. include node carries per-parent `take`/`skip`).
async fn load_relation_include_value(
    state: &EngineState,
    parent_row: &Row,
    rel_info: &RelationInfo,
    include_node: &IncludeNode,
    tx_id: Option<&str>,
) -> Result<Value, ProtocolError> {
    let parent_join = parent_join_column(rel_info);
    let Some(parent_value) = parent_row.get(&parent_join).cloned() else {
        return Ok(empty_relation_value(rel_info));
    };

    let target_model = state
        .models()
        .get(&rel_info.target_logical_name)
        .ok_or_else(|| {
            ProtocolError::QueryPlanning(format!(
                "Model '{}' not found",
                rel_info.target_logical_name
            ))
        })?;

    let join_filter = Expr::column(child_join_column(rel_info)).eq(Expr::param(parent_value));
    let filter = Some(if let Some(child_filter) = include_node.filter.clone() {
        join_filter.and(child_filter)
    } else {
        join_filter
    });

    let query_args = QueryArgs {
        filter,
        order_by: include_node.order_by.clone(),
        take: if rel_info.is_array {
            include_node.take
        } else {
            include_node.take.or(Some(1))
        },
        skip: include_node.skip,
        include: include_node.nested.clone(),
        select: std::collections::HashSet::new(),
        cursor: None,
        backward: false,
        distinct: vec![],
        nearest: None,
    };

    let child_rows = Box::pin(execute_find_many_rows(
        state,
        target_model,
        query_args,
        tx_id,
    ))
    .await?;

    let projection = IncludeProjection::for_model(target_model);
    if rel_info.is_array {
        Ok(Value::Json(JsonValue::Array(
            child_rows
                .iter()
                .map(|row| row_to_json_value(&projection, row))
                .collect(),
        )))
    } else {
        Ok(child_rows
            .first()
            .map(|row| Value::Json(row_to_json_value(&projection, row)))
            .unwrap_or(Value::Null))
    }
}

/// Batched path: load all children for `parent_rows` in one query, then group
/// in memory. Returns the per-parent include value in the same order as
/// `parent_rows`. Returns `Ok(None)` when the caller should fall back to
/// per-parent execution.
async fn batch_load_relation_include(
    state: &EngineState,
    parent_rows: &[Row],
    rel_info: &RelationInfo,
    include_node: &IncludeNode,
    tx_id: Option<&str>,
) -> Result<Option<Vec<Value>>, ProtocolError> {
    if include_node.take.is_some() || include_node.skip.is_some() {
        return Ok(None);
    }

    let parent_join = parent_join_column(rel_info);

    let mut row_keys: Vec<Option<GroupKey>> = Vec::with_capacity(parent_rows.len());
    let mut key_counts: HashMap<GroupKey, usize> = HashMap::with_capacity(parent_rows.len());
    let mut unique_values: Vec<Value> = Vec::with_capacity(parent_rows.len());

    for parent_row in parent_rows {
        match parent_row.get(&parent_join) {
            Some(Value::Null) | None => row_keys.push(None),
            Some(value) => {
                let key = group_key(value);
                match key_counts.entry(key.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(1);
                        unique_values.push(value.clone());
                    }
                    Entry::Occupied(mut entry) => *entry.get_mut() += 1,
                }
                row_keys.push(Some(key));
            }
        }
    }

    if unique_values.is_empty() {
        return Ok(Some(
            parent_rows
                .iter()
                .map(|_| empty_relation_value(rel_info))
                .collect(),
        ));
    }

    let target_model = state
        .models()
        .get(&rel_info.target_logical_name)
        .ok_or_else(|| {
            ProtocolError::QueryPlanning(format!(
                "Model '{}' not found",
                rel_info.target_logical_name
            ))
        })?;

    let in_predicate = Expr::column(child_join_column(rel_info))
        .in_list(unique_values.into_iter().map(Expr::param).collect());
    let filter = Some(if let Some(child_filter) = include_node.filter.clone() {
        in_predicate.and(child_filter)
    } else {
        in_predicate
    });

    let query_args = QueryArgs {
        filter,
        order_by: include_node.order_by.clone(),
        take: None,
        skip: None,
        include: include_node.nested.clone(),
        select: std::collections::HashSet::new(),
        cursor: None,
        backward: false,
        distinct: vec![],
        nearest: None,
    };

    let child_rows = Box::pin(execute_find_many_rows(
        state,
        target_model,
        query_args,
        tx_id,
    ))
    .await?;

    let projection = IncludeProjection::for_model(target_model);
    let child_join = child_join_column(rel_info);
    Ok(Some(build_include_values(
        row_keys,
        key_counts,
        &child_rows,
        &child_join,
        &projection,
        rel_info.is_array,
    )))
}

/// Load the per-parent include values for one relation: batched query when
/// possible, per-parent fallback otherwise. The fallback runs its child
/// queries concurrently (bounded, order-preserving) outside transactions and
/// sequentially inside them, where a single connection is available.
async fn load_relation_values(
    state: &EngineState,
    rows: &[Row],
    rel_info: &RelationInfo,
    include_node: &IncludeNode,
    tx_id: Option<&str>,
) -> Result<Vec<Value>, ProtocolError> {
    if let Some(values) =
        batch_load_relation_include(state, rows, rel_info, include_node, tx_id).await?
    {
        return Ok(values);
    }

    if tx_id.is_none() {
        let child_loads: Vec<_> = rows
            .iter()
            .map(|parent_row| {
                load_relation_include_value(state, parent_row, rel_info, include_node, None)
            })
            .collect();
        return stream::iter(child_loads)
            .buffered(PER_PARENT_CONCURRENCY)
            .try_collect()
            .await;
    }

    let mut values = Vec::with_capacity(rows.len());
    for parent_row in rows {
        values.push(
            load_relation_include_value(state, parent_row, rel_info, include_node, tx_id).await?,
        );
    }
    Ok(values)
}

/// Attach include payloads (one column per relation field) to every row in
/// `rows`, preferring a single batched child query per relation. Falls back to
/// per-parent execution when the include node carries `take`/`skip`.
/// Relations are processed in field-name order so the appended `<field>_json`
/// columns (and the child queries) are deterministic across runs. Outside
/// transactions, sibling relations load in parallel (they are independent
/// queries on pooled connections); inside a transaction the single connection
/// forces sequential execution.
pub(super) async fn hydrate_rows_with_includes(
    state: &EngineState,
    model: &ModelIr,
    rows: Vec<Row>,
    includes: &HashMap<String, IncludeNode>,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    if rows.is_empty() || includes.is_empty() {
        return Ok(rows);
    }

    let relation_map = state.relation_map_for_model(model)?;

    let mut include_entries: Vec<(&String, &IncludeNode)> = includes.iter().collect();
    include_entries.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    let mut per_relation_values: Vec<(String, Vec<Value>)> = if tx_id.is_none() {
        let rows_ref = &rows;
        let loads: Vec<_> = include_entries
            .into_iter()
            .filter_map(|(field_name, include_node)| {
                let rel_info = relation_map.get(field_name)?;
                Some(async move {
                    let values =
                        load_relation_values(state, rows_ref, rel_info, include_node, None).await?;
                    Ok::<_, ProtocolError>((field_name.clone(), values))
                })
            })
            .collect();
        futures::future::try_join_all(loads).await?
    } else {
        let mut acc = Vec::with_capacity(includes.len());
        for (field_name, include_node) in include_entries {
            let Some(rel_info) = relation_map.get(field_name) else {
                continue;
            };
            let values = load_relation_values(state, &rows, rel_info, include_node, tx_id).await?;
            acc.push((field_name.clone(), values));
        }
        acc
    };

    let mut hydrated = Vec::with_capacity(rows.len());
    for (idx, row) in rows.into_iter().enumerate() {
        let mut hydrated_row = row;
        for (field_name, values) in &mut per_relation_values {
            let value = std::mem::replace(&mut values[idx], Value::Null);
            hydrated_row.push_column(include_alias(field_name), value);
        }
        hydrated.push(hydrated_row);
    }

    Ok(hydrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child_row(fk: i64, title: &str) -> Row {
        Row::new(vec![
            (
                "blog_posts__title".to_string(),
                Value::String(title.to_string()),
            ),
            ("blog_posts__author_id".to_string(), Value::I64(fk)),
        ])
    }

    fn test_projection() -> IncludeProjection {
        IncludeProjection::new(vec![
            ("blog_posts__title".to_string(), "title".to_string()),
            ("blog_posts__author_id".to_string(), "authorId".to_string()),
        ])
    }

    #[test]
    fn group_key_normalizes_integer_widths() {
        assert_eq!(group_key(&Value::I32(5)), group_key(&Value::I64(5)));
        assert_ne!(group_key(&Value::I64(5)), group_key(&Value::I64(6)));
        assert_ne!(
            group_key(&Value::I64(5)),
            group_key(&Value::String("5".to_string()))
        );
    }

    #[test]
    fn group_key_matches_enum_and_string_variants() {
        assert_eq!(
            group_key(&Value::Enum {
                value: "ADMIN".to_string(),
                type_name: "role".to_string(),
            }),
            group_key(&Value::String("ADMIN".to_string()))
        );
    }

    #[test]
    fn build_include_values_groups_children_per_parent() {
        let row_keys = vec![
            Some(GroupKey::Int(1)),
            None,
            Some(GroupKey::Int(2)),
            Some(GroupKey::Int(3)),
        ];
        let key_counts = HashMap::from([
            (GroupKey::Int(1), 1usize),
            (GroupKey::Int(2), 1),
            (GroupKey::Int(3), 1),
        ]);
        let child_rows = vec![child_row(1, "a1"), child_row(1, "a2"), child_row(3, "c1")];

        let values = build_include_values(
            row_keys,
            key_counts,
            &child_rows,
            "blog_posts__author_id",
            &test_projection(),
            true,
        );

        assert_eq!(values.len(), 4);
        let Value::Json(JsonValue::Array(first)) = &values[0] else {
            panic!(
                "array relation should produce a JSON array, got {:?}",
                values[0]
            );
        };
        assert_eq!(first.len(), 2);
        assert_eq!(first[0]["title"], "a1");
        assert_eq!(first[0]["authorId"], 1);
        assert_eq!(values[1], Value::Json(JsonValue::Array(vec![])));
        assert_eq!(values[2], Value::Json(JsonValue::Array(vec![])));
        let Value::Json(JsonValue::Array(last)) = &values[3] else {
            panic!(
                "array relation should produce a JSON array, got {:?}",
                values[3]
            );
        };
        assert_eq!(last.len(), 1);
        assert_eq!(last[0]["title"], "c1");
    }

    #[test]
    fn build_include_values_duplicates_children_for_shared_keys() {
        // Two parents share PK value 1 (e.g. self-join shapes); both must get
        // the full child set even though the last use moves instead of cloning.
        let row_keys = vec![Some(GroupKey::Int(1)), Some(GroupKey::Int(1))];
        let key_counts = HashMap::from([(GroupKey::Int(1), 2usize)]);
        let child_rows = vec![child_row(1, "a1"), child_row(1, "a2")];

        let values = build_include_values(
            row_keys,
            key_counts,
            &child_rows,
            "blog_posts__author_id",
            &test_projection(),
            true,
        );

        for value in &values {
            let Value::Json(JsonValue::Array(children)) = value else {
                panic!("array relation should produce a JSON array, got {value:?}");
            };
            assert_eq!(children.len(), 2);
        }
    }

    #[test]
    fn build_include_values_takes_first_child_for_to_one_relations() {
        let row_keys = vec![Some(GroupKey::Int(1)), Some(GroupKey::Int(2))];
        let key_counts = HashMap::from([(GroupKey::Int(1), 1usize), (GroupKey::Int(2), 1)]);
        let child_rows = vec![child_row(1, "first"), child_row(1, "second")];

        let values = build_include_values(
            row_keys,
            key_counts,
            &child_rows,
            "blog_posts__author_id",
            &test_projection(),
            false,
        );

        let Value::Json(JsonValue::Object(obj)) = &values[0] else {
            panic!(
                "to-one relation should produce a JSON object, got {:?}",
                values[0]
            );
        };
        assert_eq!(obj["title"], "first");
        assert_eq!(values[1], Value::Null);
    }
}
