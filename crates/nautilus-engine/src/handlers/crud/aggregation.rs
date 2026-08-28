use std::collections::HashMap;

use super::common::wrap_data_result;
use super::*;

fn collect_agg_fields(model: &ModelIr, value: &serde_json::Value) -> Vec<String> {
    if value.as_bool() == Some(true) {
        model
            .scalar_fields()
            .map(|field| field.logical_name.clone())
            .collect()
    } else if let Some(obj) = value.as_object() {
        obj.iter()
            .filter(|(_, flag)| flag.as_bool() == Some(true))
            .map(|(field, _)| field.clone())
            .collect()
    } else {
        vec![]
    }
}

pub(super) async fn execute_group_by_rows(
    state: &EngineState,
    params: GroupByParams,
) -> Result<Vec<Row>, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let logical_to_db = metadata.logical_to_db();
    let args = params.args.as_ref();

    let by_fields = parse_by_fields(args)?;

    let qualified_filter = args
        .and_then(|value| value.get("where"))
        .map(|where_val| {
            crate::filter::parse_where_filter(
                where_val,
                &crate::filter::RelationMap::new(),
                metadata.field_types(),
                crate::filter::SchemaContext::none(),
            )
            .map(|expr| qualify_filter_columns(expr, &model.db_name, logical_to_db))
        })
        .transpose()?;

    let having_expr = args
        .and_then(|value| value.get("having"))
        .map(|value| parse_having(value, &model.db_name, logical_to_db))
        .transpose()?;

    let group_orders = args
        .and_then(|value| value.get("orderBy"))
        .map(|value| parse_group_by_order_by(value, &model.db_name, logical_to_db))
        .transpose()?
        .unwrap_or_default();

    let select = build_group_by_select(GroupBySelect {
        model,
        logical_to_db,
        by_fields: &by_fields,
        aggregate_items: build_aggregate_items(model, args, logical_to_db),
        filter: qualified_filter,
        having: having_expr,
        orders: group_orders,
        take: args
            .and_then(|value| value.get("take"))
            .and_then(|value| value.as_i64())
            .map(|value| value as i32),
        skip: args
            .and_then(|value| value.get("skip"))
            .and_then(|value| value.as_u64())
            .map(|value| value as u32),
    })?;

    let sql = state.dialect.render_select_owned(select).map_err(|e| {
        ProtocolError::QueryPlanning(format!("Failed to render groupBy query: {}", e))
    })?;

    let rows = state
        .execute_query_on(&sql, "GroupBy", tx_id.as_deref())
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| shape_group_row(row, metadata.db_to_logical()))
        .collect())
}

fn parse_by_fields(args: Option<&serde_json::Value>) -> Result<Vec<String>, ProtocolError> {
    let by_fields: Vec<String> = args
        .and_then(|value| value.get("by"))
        .and_then(|value| value.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if by_fields.is_empty() {
        return Err(ProtocolError::InvalidParams(
            "groupBy requires at least one field in `by`".to_string(),
        ));
    }
    Ok(by_fields)
}

/// Build the `(alias, expression)` pairs for every requested aggregate.
///
/// Aliases carry the aggregate kind (`_count__`, `_avg_`, …) so
/// [`shape_group_row`] can fold the flat result columns back into the nested
/// `_count` / `_avg` / … objects the clients expect.
fn build_aggregate_items(
    model: &ModelIr,
    args: Option<&serde_json::Value>,
    logical_to_db: &HashMap<String, String>,
) -> Vec<(String, Expr)> {
    let mut items: Vec<(String, Expr)> = Vec::new();

    if let Some(count_val) = args.and_then(|value| value.get("count")) {
        if count_val.as_bool() == Some(true) {
            items.push(count_all_item());
        } else if let Some(obj) = count_val.as_object() {
            for (field, flag) in obj {
                if flag.as_bool() != Some(true) {
                    continue;
                }
                if field == "_all" {
                    items.push(count_all_item());
                } else {
                    items.push((
                        format!("_count__{}", field),
                        Expr::function_call(
                            "COUNT",
                            vec![aggregate_column(model, logical_to_db, field)],
                        ),
                    ));
                }
            }
        }
    }

    for (agg_key, agg_fn) in [
        ("avg", "AVG"),
        ("sum", "SUM"),
        ("min", "MIN"),
        ("max", "MAX"),
    ] {
        let Some(agg_val) = args.and_then(|value| value.get(agg_key)) else {
            continue;
        };
        for field in collect_agg_fields(model, agg_val) {
            items.push((
                format!("_{}_{}", agg_key, field),
                Expr::function_call(agg_fn, vec![aggregate_column(model, logical_to_db, &field)]),
            ));
        }
    }

    items
}

fn count_all_item() -> (String, Expr) {
    (
        "_count___all".to_string(),
        Expr::function_call("COUNT", vec![Expr::Star]),
    )
}

fn aggregate_column(model: &ModelIr, logical_to_db: &HashMap<String, String>, field: &str) -> Expr {
    Expr::Column(format!(
        "{}__{}",
        model.db_name,
        db_column_for(logical_to_db, field)
    ))
}

fn db_column_for(logical_to_db: &HashMap<String, String>, field: &str) -> String {
    logical_to_db
        .get(field)
        .cloned()
        .unwrap_or_else(|| field.to_string())
}

struct GroupBySelect<'a> {
    model: &'a ModelIr,
    logical_to_db: &'a HashMap<String, String>,
    by_fields: &'a [String],
    aggregate_items: Vec<(String, Expr)>,
    filter: Option<Expr>,
    having: Option<Expr>,
    orders: Vec<crate::filter::GroupByOrderItem>,
    take: Option<i32>,
    skip: Option<u32>,
}

fn build_group_by_select(spec: GroupBySelect<'_>) -> Result<Select, ProtocolError> {
    use nautilus_core::ColumnMarker;

    let order_by_columns = spec
        .orders
        .iter()
        .filter(|order| matches!(order, crate::filter::GroupByOrderItem::Column(_)))
        .count();

    let mut builder = Select::from_table(&spec.model.db_name).with_capacity(SelectCapacity {
        items: spec.by_fields.len() + spec.aggregate_items.len(),
        order_by_columns,
        order_by_exprs: spec.orders.len() - order_by_columns,
        group_by: spec.by_fields.len(),
        ..SelectCapacity::default()
    });

    for field_name in spec.by_fields {
        let marker = ColumnMarker::new(
            &spec.model.db_name,
            db_column_for(spec.logical_to_db, field_name),
        );
        builder = builder.item(SelectItem::from(marker.clone()));
        builder = builder.group_by_column(marker);
    }

    for (alias, expr) in spec.aggregate_items {
        builder = builder.item(SelectItem::computed(expr, alias));
    }

    if let Some(filter) = spec.filter {
        builder = builder.filter(filter);
    }
    if let Some(having) = spec.having {
        builder = builder.having(having);
    }

    for order in spec.orders {
        builder = match order {
            crate::filter::GroupByOrderItem::Column(order) => {
                builder.order_by(order.column, order.direction)
            }
            crate::filter::GroupByOrderItem::Expr(expr, dir) => builder.order_by_expr(expr, dir),
        };
    }

    if let Some(value) = spec.take {
        builder = builder.take(value);
    }
    if let Some(value) = spec.skip {
        builder = builder.skip(value);
    }

    builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build groupBy query: {}", e)))
}

/// Fold the flat aggregate columns of one result row into the nested shape the
/// clients consume: grouped columns keep their logical names, `_count__*`,
/// `_avg_*`, … columns become `_count`, `_avg`, … objects.
fn shape_group_row(row: Row, db_to_logical: &HashMap<String, String>) -> Row {
    const AGGREGATE_PREFIXES: [(&str, &str); 4] = [
        ("_avg_", "_avg"),
        ("_sum_", "_sum"),
        ("_min_", "_min"),
        ("_max_", "_max"),
    ];

    let mut shaped_row = Row::with_capacity(row.len() + 5);
    let mut count_map = serde_json::Map::new();
    let mut aggregate_maps: [serde_json::Map<String, JsonValue>; 4] = Default::default();

    'columns: for (col_name, value) in row.into_columns_iter() {
        if let Some(rest) = col_name.strip_prefix("_count__") {
            count_map.insert(rest.to_string(), value.to_json_plain());
            continue;
        }
        for (index, (prefix, _)) in AGGREGATE_PREFIXES.iter().enumerate() {
            if let Some(rest) = col_name.strip_prefix(prefix) {
                aggregate_maps[index].insert(rest.to_string(), value.to_json_plain());
                continue 'columns;
            }
        }

        let field_key = col_name
            .split_once("__")
            .map(|(_, col_part)| col_part)
            .unwrap_or(col_name.as_ref());
        let field_key = db_to_logical
            .get(field_key)
            .cloned()
            .unwrap_or_else(|| field_key.to_string());
        shaped_row.push_column(field_key, value);
    }

    if !count_map.is_empty() {
        shaped_row.push_column(
            "_count".to_string(),
            Value::Json(JsonValue::Object(count_map)),
        );
    }
    for ((_, key), map) in AGGREGATE_PREFIXES.iter().zip(aggregate_maps) {
        if !map.is_empty() {
            shaped_row.push_column(key.to_string(), Value::Json(JsonValue::Object(map)));
        }
    }

    shaped_row
}

/// Handle `query.aggregate`.
///
/// One aggregate row over the whole filtered set. The select carries no
/// grouping key, so the database returns exactly one row even when the filter
/// matches nothing — the aggregates are then `0` for counts and `NULL` for the
/// rest.
pub(in crate::handlers) async fn handle_aggregate(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: AggregateParams = parse_params(&request, "aggregate")?;
    let rows = execute_aggregate_rows(state, params).await?;
    wrap_data_result(&rows, "aggregate result")
}

pub(in crate::handlers) async fn handle_aggregate_typed(
    state: &EngineState,
    params: AggregateParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_aggregate_rows(state, params).await
}

async fn execute_aggregate_rows(
    state: &EngineState,
    params: AggregateParams,
) -> Result<Vec<Row>, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let logical_to_db = metadata.logical_to_db();
    let args = params.args.as_ref();

    let aggregate_items = build_aggregate_items(model, args, logical_to_db);
    if aggregate_items.is_empty() {
        return Err(ProtocolError::InvalidParams(
            "aggregate requires at least one of count, avg, sum, min, max".to_string(),
        ));
    }

    let qualified_filter = args
        .and_then(|value| value.get("where"))
        .map(|where_val| {
            crate::filter::parse_where_filter(
                where_val,
                &crate::filter::RelationMap::new(),
                metadata.field_types(),
                crate::filter::SchemaContext::none(),
            )
            .map(|expr| qualify_filter_columns(expr, &model.db_name, logical_to_db))
        })
        .transpose()?;

    let select = build_group_by_select(GroupBySelect {
        model,
        logical_to_db,
        by_fields: &[],
        aggregate_items,
        filter: qualified_filter,
        having: None,
        orders: Vec::new(),
        take: None,
        skip: None,
    })?;

    let sql = state.dialect.render_select_owned(select).map_err(|e| {
        ProtocolError::QueryPlanning(format!("Failed to render aggregate query: {}", e))
    })?;

    Ok(state
        .execute_query_on(&sql, "Aggregate", tx_id.as_deref())
        .await?
        .into_iter()
        .map(|row| shape_group_row(row, metadata.db_to_logical()))
        .collect())
}

/// Handle `query.groupBy`.
pub(in crate::handlers) async fn handle_group_by(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: GroupByParams = parse_params(&request, "groupBy")?;
    let rows = execute_group_by_rows(state, params).await?;
    wrap_data_result(&rows, "groupBy result")
}

pub(in crate::handlers) async fn handle_group_by_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: GroupByParams = parse_params(&request, "groupBy")?;
    execute_group_by_rows(state, params).await
}

pub(in crate::handlers) async fn handle_group_by_typed(
    state: &EngineState,
    params: GroupByParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_group_by_rows(state, params).await
}
