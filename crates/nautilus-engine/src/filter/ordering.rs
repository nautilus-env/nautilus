use super::where_filter::{combine_conditions, parse_field_operators};
use super::*;

pub(super) fn parse_order_by(
    order_value: &JsonValue,
    field_types: Option<&FieldTypeMap>,
) -> Result<Vec<OrderBy>, ProtocolError> {
    let order_array = order_value
        .as_array()
        .ok_or_else(|| ProtocolError::InvalidFilter("orderBy must be an array".to_string()))?;

    let mut result = Vec::new();

    for item in order_array {
        let obj = item.as_object().ok_or_else(|| {
            ProtocolError::InvalidFilter("orderBy items must be objects".to_string())
        })?;

        for (field, direction) in obj {
            let dir_str = direction.as_str().ok_or_else(|| {
                ProtocolError::InvalidFilter("orderBy direction must be a string".to_string())
            })?;

            let dir = match dir_str.to_lowercase().as_str() {
                "asc" => OrderDir::Asc,
                "desc" => OrderDir::Desc,
                _ => {
                    return Err(ProtocolError::InvalidFilter(format!(
                        "Invalid order direction: {}",
                        dir_str
                    )));
                }
            };

            if let Some(types) = field_types.filter(|types| !types.is_empty()) {
                let root = field
                    .split_once("__")
                    .map_or(field.as_str(), |(_, column)| column);
                let root = root.split_once('.').map_or(root, |(parent, _)| parent);
                if !types.contains_key(root) {
                    return Err(ProtocolError::InvalidFilter(format!(
                        "Unknown orderBy field '{}'",
                        field
                    )));
                }
            }

            if field_types
                .and_then(|types| types.get(field))
                .is_some_and(|field_type| {
                    matches!(
                        field_type,
                        ResolvedFieldType::Scalar(
                            ScalarType::Vector { .. }
                                | ScalarType::Geometry
                                | ScalarType::Geography
                        )
                    )
                })
            {
                return Err(ProtocolError::InvalidFilter(format!(
                    "Field '{}' cannot be used with classic orderBy because it is not orderable",
                    field
                )));
            }

            result.push(OrderBy {
                column: field.clone(),
                direction: dir,
            });
        }
    }

    Ok(result)
}

pub(crate) enum GroupByOrderItem {
    Column(OrderBy),
    Expr(Expr, OrderDir),
}

pub(crate) fn parse_group_by_order_by(
    order_value: &JsonValue,
    table: &str,
    logical_to_db: &HashMap<String, String>,
) -> Result<Vec<GroupByOrderItem>, ProtocolError> {
    let order_array = order_value
        .as_array()
        .ok_or_else(|| ProtocolError::InvalidFilter("orderBy must be an array".to_string()))?;

    let mut orders = Vec::new();

    for item in order_array {
        let obj = item.as_object().ok_or_else(|| {
            ProtocolError::InvalidFilter("orderBy items must be objects".to_string())
        })?;

        for (key, value) in obj {
            match key.as_str() {
                "_count" | "_avg" | "_sum" | "_min" | "_max" => {
                    let agg_fn = match key.as_str() {
                        "_count" => "COUNT",
                        "_avg" => "AVG",
                        "_sum" => "SUM",
                        "_min" => "MIN",
                        _ => "MAX",
                    };
                    let inner = value.as_object().ok_or_else(|| {
                        ProtocolError::InvalidFilter(format!(
                            "{} orderBy value must be an object",
                            key
                        ))
                    })?;
                    for (field, dir_val) in inner {
                        let dir_str = dir_val.as_str().ok_or_else(|| {
                            ProtocolError::InvalidFilter(
                                "orderBy direction must be a string".to_string(),
                            )
                        })?;
                        let dir = parse_order_dir(dir_str)?;
                        let agg_arg = if field == "_all" {
                            Expr::Star
                        } else {
                            let db_col = logical_to_db
                                .get(field.as_str())
                                .cloned()
                                .unwrap_or_else(|| field.clone());
                            Expr::Column(format!("{}__{}", table, db_col))
                        };
                        let agg_expr = Expr::function_call(agg_fn, vec![agg_arg]);
                        orders.push(GroupByOrderItem::Expr(agg_expr, dir));
                    }
                }
                _ => {
                    let dir_str = value.as_str().ok_or_else(|| {
                        ProtocolError::InvalidFilter(
                            "orderBy direction must be a string".to_string(),
                        )
                    })?;
                    let dir = parse_order_dir(dir_str)?;
                    let db_col = logical_to_db
                        .get(key.as_str())
                        .cloned()
                        .unwrap_or_else(|| key.clone());
                    let qualified = format!("{}__{}", table, db_col);
                    orders.push(GroupByOrderItem::Column(OrderBy::new(qualified, dir)));
                }
            }
        }
    }

    Ok(orders)
}

pub(super) fn parse_order_dir(s: &str) -> Result<OrderDir, ProtocolError> {
    match s.to_lowercase().as_str() {
        "asc" => Ok(OrderDir::Asc),
        "desc" => Ok(OrderDir::Desc),
        _ => Err(ProtocolError::InvalidFilter(format!(
            "Invalid order direction: {}",
            s
        ))),
    }
}

fn having_aggregate_fn(key: &str) -> Option<&'static str> {
    match key {
        "_count" => Some("COUNT"),
        "_avg" => Some("AVG"),
        "_sum" => Some("SUM"),
        "_min" => Some("MIN"),
        "_max" => Some("MAX"),
        _ => None,
    }
}

fn having_column(table: &str, logical_to_db: &HashMap<String, String>, field: &str) -> Expr {
    if field == "_all" {
        return Expr::Star;
    }
    let db_col = logical_to_db
        .get(field)
        .cloned()
        .unwrap_or_else(|| field.to_string());
    Expr::Column(format!("{}__{}", table, db_col))
}

fn having_operators<'a>(
    value: &'a JsonValue,
    path: &str,
) -> Result<&'a serde_json::Map<String, JsonValue>, ProtocolError> {
    value.as_object().ok_or_else(|| {
        ProtocolError::InvalidFilter(format!("having.{} must be an operator object", path))
    })
}

/// Parse a `having` payload in either accepted shape.
///
/// Aggregate first (`{"_sum": {"views": {"gt": 10}}}`) is Nautilus's own shape;
/// field first (`{"views": {"_sum": {"gt": 10}}}`) is the other spelling in
/// common use. A grouped column may also be filtered directly
/// (`{"role": {"eq": "ADMIN"}}`), which neither aggregate shape can express.
pub(crate) fn parse_having(
    having_value: &JsonValue,
    table: &str,
    logical_to_db: &HashMap<String, String>,
) -> Result<Expr, ProtocolError> {
    let obj = having_value
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidFilter("having must be an object".to_string()))?;

    let mut conditions = Vec::new();

    for (key, value) in obj {
        if let Some(agg_fn) = having_aggregate_fn(key) {
            let fields_obj = value.as_object().ok_or_else(|| {
                ProtocolError::InvalidFilter(format!("having.{} must be an object", key))
            })?;
            for (field, filter_val) in fields_obj {
                let agg_expr =
                    Expr::function_call(agg_fn, vec![having_column(table, logical_to_db, field)]);
                let operators = having_operators(filter_val, &format!("{}.{}", key, field))?;
                conditions.push(parse_field_operators(agg_expr, operators, None)?);
            }
            continue;
        }

        let inner = value.as_object().ok_or_else(|| {
            ProtocolError::InvalidFilter(format!("having.{} must be an object", key))
        })?;

        let column = having_column(table, logical_to_db, key);
        let mut nested = Vec::new();
        for (inner_key, filter_val) in inner {
            match having_aggregate_fn(inner_key) {
                Some(agg_fn) => {
                    let agg_expr = Expr::function_call(agg_fn, vec![column.clone()]);
                    let operators =
                        having_operators(filter_val, &format!("{}.{}", key, inner_key))?;
                    nested.push(parse_field_operators(agg_expr, operators, None)?);
                }
                None => {
                    nested.push(parse_field_operators(column.clone(), inner, None)?);
                    break;
                }
            }
        }
        conditions.extend(nested);
    }

    combine_conditions(conditions, BinaryOp::And)
}

pub(super) fn parse_int(value: &JsonValue, field_name: &str) -> Result<u32, ProtocolError> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| {
            ProtocolError::InvalidParams(format!("{} must be a non-negative integer", field_name))
        })
}

pub(super) fn parse_signed_int(value: &JsonValue, field_name: &str) -> Result<i64, ProtocolError> {
    value
        .as_i64()
        .ok_or_else(|| ProtocolError::InvalidParams(format!("{} must be an integer", field_name)))
}
