//! SELECT planning for the read paths, and the plan-cache keys that let a
//! rendered statement be replayed with fresh parameters.
//!
//! Planning stops at the [`FindManyPlan`]: it carries the rendered SQL, the
//! decoding hints for the projected columns and the transformations the
//! dialect could not express, leaving each entry point free to choose how to
//! consume the rows.

use std::collections::HashMap;

use nautilus_connector::Row;
use nautilus_core::{
    build_cursor_predicate, Expr, OrderDir, Select, SelectCapacity, SelectItem, Value,
};
use nautilus_dialect::Sql;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::ModelIr;

use super::ordering::{resolve_order_target, ResolvedOrderTarget};
use crate::conversion::ValueHint;
use crate::filter::{qualify_filter_columns, IncludeNode, QueryArgs};
use crate::state::EngineState;

/// Pre-execution artefact produced by [`build_find_many_plan`].
///
/// Splitting plan-building from row consumption lets the buffered fast path
/// ([`super::find_many::execute_find_many_rows`]) and the chunked streaming
/// path ([`super::stream::stream_find_many_chunked`]) share the same SQL/hint
/// computation while each owning its consumption strategy. Streaming requires
/// `backward = false` and `include.is_empty()` because both transformations
/// need the full row set in memory before any output can leave the engine.
pub(super) struct FindManyPlan {
    pub(super) sql: Sql,
    pub(super) row_hints: Vec<Option<ValueHint>>,
    pub(super) backward: bool,
    pub(super) include: HashMap<String, IncludeNode>,
    /// Set when `distinct` has to be honoured after the rows come back, because
    /// the dialect has no `DISTINCT ON`.
    pub(super) distinct: Option<DistinctFallback>,
}

/// Deduplication the engine performs itself, standing in for `DISTINCT ON`.
///
/// `SELECT DISTINCT` compares whole rows, and the engine always projects the
/// primary key, so on SQLite and MySQL nothing would ever collapse. The query
/// is therefore rendered without `LIMIT`/`OFFSET` and both the deduplication
/// and the window are applied to the decoded rows, in the order the database
/// already sorted them.
pub(super) struct DistinctFallback {
    columns: Vec<String>,
    skip: u32,
    take: Option<i32>,
}

impl DistinctFallback {
    pub(super) fn apply(&self, rows: Vec<Row>) -> Vec<Row> {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut kept = Vec::new();
        let mut skipped = 0u32;

        for row in rows {
            let key = self
                .columns
                .iter()
                .map(|column| format!("{:?}", row.get(column)))
                .collect::<Vec<_>>()
                .join("\u{1f}");
            if !seen.insert(key) {
                continue;
            }
            if skipped < self.skip {
                skipped += 1;
                continue;
            }
            kept.push(row);
            if self.take.is_some_and(|take| kept.len() >= take as usize) {
                break;
            }
        }

        kept
    }
}

pub(super) fn build_find_many_plan(
    state: &EngineState,
    model: &ModelIr,
    query_args: QueryArgs,
) -> Result<FindManyPlan, ProtocolError> {
    let QueryArgs {
        filter,
        order_by,
        take,
        skip,
        include,
        select,
        cursor,
        backward,
        distinct,
        nearest,
        partition,
        join,
    } = query_args;

    let metadata = state.model_metadata(model);
    let logical_to_db = metadata.logical_to_db();
    let qualified_filter =
        filter.map(|expr| qualify_filter_columns(expr, &model.db_name, logical_to_db));
    let pk_fields = metadata.primary_key_fields();

    let mut builder =
        Select::from_table(crate::metadata::model_table(model)).with_capacity(SelectCapacity {
            items: metadata.scalar_fields().len(),
            joins: usize::from(join.is_some()),
            order_by_columns: order_by.len() + distinct.len() + pk_fields.len(),
            order_by_exprs: usize::from(nearest.is_some()),
            distinct: distinct.len(),
            ..SelectCapacity::default()
        });
    let mut row_hints = Vec::new();

    for field in metadata.scalar_fields() {
        if !select.is_empty()
            && !select.contains(field.logical_name())
            && !pk_fields
                .iter()
                .any(|pk_field| pk_field.logical_name() == field.logical_name())
        {
            continue;
        }
        builder = builder.item(SelectItem::from(field.marker().clone()));
        row_hints.push(field.hint());
    }

    let combined_filter = if let Some(ref cursor_map) = cursor {
        let pk_refs: Vec<(&str, &str)> = pk_fields
            .iter()
            .map(|field| (field.logical_name(), field.qualified_column()))
            .collect();

        let cursor_pred = build_cursor_predicate(&pk_refs, cursor_map, backward)
            .map_err(|e| ProtocolError::InvalidParams(format!("Invalid cursor: {}", e)))?;

        let existing_order_cols: std::collections::HashSet<&str> =
            order_by.iter().map(|order| order.column.as_str()).collect();
        for pk_field in pk_fields {
            if !existing_order_cols.contains(pk_field.db_name()) {
                let dir = if backward {
                    OrderDir::Desc
                } else {
                    OrderDir::Asc
                };
                builder = builder.order_by(pk_field.db_name().to_string(), dir);
            }
        }

        Some(match qualified_filter {
            Some(existing) => existing.and(cursor_pred),
            None => cursor_pred,
        })
    } else {
        qualified_filter
    };

    if let Some(filter_expr) = combined_filter {
        builder = builder.filter(filter_expr);
    }

    if let Some(nearest) = nearest {
        let db_col = logical_to_db
            .get(nearest.field.as_str())
            .cloned()
            .unwrap_or(nearest.field);
        let distance_expr = Expr::vector_distance(
            nearest.metric,
            Expr::column(format!("{}__{}", model.db_name, db_col)),
            Expr::param(Value::Vector(nearest.query)),
        );
        builder = builder.order_by_expr(distance_expr, OrderDir::Asc);
    }

    if !distinct.is_empty() {
        let existing_order_cols: std::collections::HashSet<&str> =
            order_by.iter().map(|order| order.column.as_str()).collect();
        for column in &distinct {
            let db_col = logical_to_db
                .get(column.as_str())
                .cloned()
                .unwrap_or_else(|| column.clone());
            if !existing_order_cols.contains(db_col.as_str()) {
                let dir = if backward {
                    OrderDir::Desc
                } else {
                    OrderDir::Asc
                };
                builder = builder.order_by(db_col, dir);
            }
        }
    }

    for order in order_by {
        let dir = if backward {
            match order.direction {
                OrderDir::Asc => OrderDir::Desc,
                OrderDir::Desc => OrderDir::Asc,
            }
        } else {
            order.direction
        };
        match resolve_order_target(state, model, logical_to_db, &order.column)? {
            ResolvedOrderTarget::Column(db_col) => {
                builder = builder.order_by(db_col, dir);
            }
            ResolvedOrderTarget::Expr(expr) => {
                builder = builder.order_by_expr(expr, dir);
            }
        }
    }

    let distinct_fallback =
        (!distinct.is_empty() && !state.dialect.supports_distinct_on()).then(|| DistinctFallback {
            columns: distinct
                .iter()
                .map(|column| {
                    format!(
                        "{}__{}",
                        model.db_name,
                        logical_to_db
                            .get(column.as_str())
                            .map_or(column.as_str(), String::as_str)
                    )
                })
                .collect(),
            skip: skip.unwrap_or(0),
            take,
        });

    if distinct_fallback.is_none() {
        if let Some(take) = take {
            builder = builder.take(take);
        }
        if let Some(skip) = skip {
            builder = builder.skip(skip);
        }
    }
    if distinct_fallback.is_none() && !distinct.is_empty() {
        let distinct_db: Vec<String> = distinct
            .iter()
            .map(|column| {
                logical_to_db
                    .get(column.as_str())
                    .cloned()
                    .unwrap_or_else(|| column.clone())
            })
            .collect();
        builder = builder.distinct(distinct_db);
    }

    if let Some(window) = partition {
        builder = builder.partition_window(window);
    }

    // The joined columns are appended to the select list after the model's own,
    // so their hints have to be appended in the same order for
    // `normalize_row_with_hints` to line up with the row it decodes.
    if let Some(join) = join {
        row_hints.extend(join.hints);
        builder = builder.join(join.clause);
    }

    let select = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build query: {}", e)))?;

    let sql = state
        .dialect
        .render_select_owned(select)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    Ok(FindManyPlan {
        sql,
        row_hints,
        backward,
        include,
        distinct: distinct_fallback,
    })
}

/// Canonicalise a projection for use in a plan-cache key: selected fields
/// plus implicit PK fields, sorted, or empty when all columns are selected.
fn resolved_projection(
    metadata: &crate::metadata::ModelMetadata,
    selected_fields: &std::collections::HashSet<&str>,
) -> Vec<String> {
    if selected_fields.is_empty() {
        return Vec::new();
    }
    let mut combined: Vec<String> = selected_fields.iter().map(|s| s.to_string()).collect();
    for pk in metadata.primary_key_fields() {
        let logical = pk.logical_name();
        if !selected_fields.contains(logical) {
            combined.push(logical.to_string());
        }
    }
    combined.sort();
    combined.dedup();
    combined
}

/// Build the plan-cache key (and the owned parameter values to bind on a hit)
/// for a `findMany`/`findFirst` request, or `None` when the request is not
/// cacheable: cursor, backward pagination, distinct, vector ordering, a joined
/// relation table and includes change the SQL or the post-processing in ways
/// the cached replay does not cover, and the filter must be a flat parametric
/// AND chain.
pub(super) fn find_many_cache_request(
    state: &EngineState,
    model: &ModelIr,
    query_args: &QueryArgs,
) -> Option<(crate::plan_cache::FindManyPlanKey, Vec<Value>)> {
    if query_args.cursor.is_some()
        || query_args.backward
        || query_args.nearest.is_some()
        || query_args.partition.is_some()
        || query_args.join.is_some()
        || !query_args.distinct.is_empty()
        || !query_args.include.is_empty()
    {
        return None;
    }

    let (filter_shape, params) = match &query_args.filter {
        None => (Vec::new(), Vec::new()),
        Some(filter) => {
            let shape = crate::plan_cache::extract_param_filter(filter)?;
            (
                shape
                    .predicates
                    .iter()
                    .map(|(column, op, variant)| ((*column).to_string(), op.clone(), *variant))
                    .collect(),
                shape.values.iter().map(|value| (*value).clone()).collect(),
            )
        }
    };

    let metadata = state.model_metadata(model);
    let selected_refs: std::collections::HashSet<&str> =
        query_args.select.iter().map(String::as_str).collect();

    Some((
        crate::plan_cache::FindManyPlanKey {
            model_db_name: crate::metadata::model_table(model).to_string(),
            selected_logical_fields: resolved_projection(metadata, &selected_refs),
            filter_shape,
            order_by: query_args
                .order_by
                .iter()
                .map(|order| (order.column.clone(), order.direction))
                .collect(),
            take: query_args.take,
            skip: query_args.skip,
        },
        params,
    ))
}

/// Build the [`crate::plan_cache::FindUniquePlanKey`] for a request matched
/// by [`crate::plan_cache::extract_simple_eq_filter`].
///
/// The resolved projection is canonicalised (selected fields plus implicit PK
/// fields, sorted) so semantically equivalent inputs share a cache entry.
pub(super) fn find_unique_plan_key(
    model: &ModelIr,
    metadata: &crate::metadata::ModelMetadata,
    selected_fields: &std::collections::HashSet<&str>,
    shape: &crate::plan_cache::EqFilterShape<'_>,
) -> crate::plan_cache::FindUniquePlanKey {
    crate::plan_cache::FindUniquePlanKey {
        model_db_name: crate::metadata::model_table(model).to_string(),
        selected_logical_fields: resolved_projection(metadata, selected_fields),
        filter_columns: shape.columns.iter().map(|s| s.to_string()).collect(),
    }
}
