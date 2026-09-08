//! The read arguments of a request: the object that carries `where`, `orderBy`,
//! `take`, `skip`, `include`, `select`, `cursor`, `distinct` and `nearest`.
//!
//! An argument that was never set is left out rather than written as null, and
//! an `include` entry is the same object one level down, so a nested relation
//! accepts the arguments the top level does.

use std::collections::HashMap;

use serde_json::{Map as JsonMap, Value as JsonValue};

use super::expressions::strip_column_qualifier;
use super::filters::expr_to_filter_json;
use crate::{FindManyArgs, IncludeRelation, OrderBy, OrderDir, Result, Value, VectorNearest};

/// Convert [`FindManyArgs`] into an object map matching the engine wire format.
pub fn find_many_args_to_protocol_object(
    args: &FindManyArgs,
) -> Result<JsonMap<String, JsonValue>> {
    let mut result = JsonMap::with_capacity(find_many_args_field_count(args));

    if let Some(where_) = &args.where_ {
        result.insert("where".to_string(), expr_to_filter_json(where_)?);
    }

    if !args.order_by.is_empty() {
        result.insert(
            "orderBy".to_string(),
            JsonValue::Array(order_by_list_to_json(&args.order_by)?),
        );
    }

    if let Some(take) = args.take {
        result.insert("take".to_string(), JsonValue::from(take));
    }

    if let Some(skip) = args.skip {
        result.insert("skip".to_string(), JsonValue::from(skip));
    }

    if !args.include.is_empty() {
        result.insert(
            "include".to_string(),
            JsonValue::Object(include_map_to_json_object(&args.include)?),
        );
    }

    if !args.select.is_empty() {
        result.insert(
            "select".to_string(),
            JsonValue::Object(select_map_to_json_object(&args.select)),
        );
    }

    if let Some(cursor) = &args.cursor {
        result.insert(
            "cursor".to_string(),
            JsonValue::Object(cursor_map_to_json_object(cursor)),
        );
    }

    if !args.distinct.is_empty() {
        result.insert(
            "distinct".to_string(),
            JsonValue::Array(distinct_fields_to_json(&args.distinct)),
        );
    }

    if let Some(nearest) = &args.nearest {
        result.insert("nearest".to_string(), nearest_to_json(nearest));
    }

    Ok(result)
}

/// Convert [`FindManyArgs`] into the same JSON payload shape used by thin clients.
///
/// This helper is intentionally conservative: if it encounters an expression
/// that cannot yet be represented in the engine wire format, it returns
/// [`Error::InvalidQuery`](crate::Error::InvalidQuery) so callers can decide
/// whether to fail or to fall back to a local execution path.
pub fn find_many_args_to_protocol_json(args: &FindManyArgs) -> Result<JsonValue> {
    Ok(JsonValue::Object(find_many_args_to_protocol_object(args)?))
}

fn find_many_args_field_count(args: &FindManyArgs) -> usize {
    let mut count = 0;
    if args.where_.is_some() {
        count += 1;
    }
    if !args.order_by.is_empty() {
        count += 1;
    }
    if args.take.is_some() {
        count += 1;
    }
    if args.skip.is_some() {
        count += 1;
    }
    if !args.include.is_empty() {
        count += 1;
    }
    if !args.select.is_empty() {
        count += 1;
    }
    if args.cursor.is_some() {
        count += 1;
    }
    if !args.distinct.is_empty() {
        count += 1;
    }
    if args.nearest.is_some() {
        count += 1;
    }
    count
}

fn include_relation_field_count(include: &IncludeRelation) -> usize {
    let mut count = 0;
    if include.where_.is_some() {
        count += 1;
    }
    if !include.order_by.is_empty() {
        count += 1;
    }
    if include.take.is_some() {
        count += 1;
    }
    if include.skip.is_some() {
        count += 1;
    }
    if include.cursor.is_some() {
        count += 1;
    }
    if !include.distinct.is_empty() {
        count += 1;
    }
    if !include.include.is_empty() {
        count += 1;
    }
    count
}

fn order_by_list_to_json(order_by: &[OrderBy]) -> Result<Vec<JsonValue>> {
    let mut result = Vec::with_capacity(order_by.len());
    for order in order_by {
        result.push(order_by_to_json(order)?);
    }
    Ok(result)
}

fn select_map_to_json_object(select: &HashMap<String, bool>) -> JsonMap<String, JsonValue> {
    let mut result = JsonMap::with_capacity(select.len());
    for (field, enabled) in select {
        result.insert(field.clone(), JsonValue::Bool(*enabled));
    }
    result
}

fn cursor_map_to_json_object(cursor: &HashMap<String, Value>) -> JsonMap<String, JsonValue> {
    let mut result = JsonMap::with_capacity(cursor.len());
    for (field, value) in cursor {
        result.insert(strip_column_qualifier(field), value.to_json_plain());
    }
    result
}

fn distinct_fields_to_json(distinct: &[String]) -> Vec<JsonValue> {
    let mut result = Vec::with_capacity(distinct.len());
    for field in distinct {
        result.push(JsonValue::String(strip_column_qualifier(field)));
    }
    result
}

fn include_map_to_json_object(
    include: &HashMap<String, IncludeRelation>,
) -> Result<JsonMap<String, JsonValue>> {
    let mut result = JsonMap::with_capacity(include.len());
    for (field, relation) in include {
        result.insert(
            field.clone(),
            JsonValue::Object(include_relation_to_json_object(relation)?),
        );
    }
    Ok(result)
}

fn include_relation_to_json_object(
    include: &IncludeRelation,
) -> Result<JsonMap<String, JsonValue>> {
    let mut result = JsonMap::with_capacity(include_relation_field_count(include));

    if let Some(where_) = &include.where_ {
        result.insert("where".to_string(), expr_to_filter_json(where_)?);
    }

    if !include.order_by.is_empty() {
        result.insert(
            "orderBy".to_string(),
            JsonValue::Array(order_by_list_to_json(&include.order_by)?),
        );
    }

    if let Some(take) = include.take {
        result.insert("take".to_string(), JsonValue::from(take));
    }

    if let Some(skip) = include.skip {
        result.insert("skip".to_string(), JsonValue::from(skip));
    }

    if let Some(cursor) = &include.cursor {
        result.insert(
            "cursor".to_string(),
            JsonValue::Object(cursor_map_to_json_object(cursor)),
        );
    }

    if !include.distinct.is_empty() {
        result.insert(
            "distinct".to_string(),
            JsonValue::Array(distinct_fields_to_json(&include.distinct)),
        );
    }

    if !include.include.is_empty() {
        result.insert(
            "include".to_string(),
            JsonValue::Object(include_map_to_json_object(&include.include)?),
        );
    }

    Ok(result)
}

fn order_by_to_json(order: &OrderBy) -> Result<JsonValue> {
    let mut result = JsonMap::new();
    result.insert(
        strip_column_qualifier(&order.column),
        JsonValue::String(match order.direction {
            OrderDir::Asc => "asc".to_string(),
            OrderDir::Desc => "desc".to_string(),
        }),
    );
    Ok(JsonValue::Object(result))
}

fn nearest_to_json(nearest: &VectorNearest) -> JsonValue {
    let mut result = JsonMap::with_capacity(3);
    result.insert(
        "field".to_string(),
        JsonValue::String(strip_column_qualifier(&nearest.field)),
    );
    result.insert(
        "query".to_string(),
        JsonValue::Array(
            nearest
                .query
                .iter()
                .map(|value| JsonValue::from(*value as f64))
                .collect(),
        ),
    );
    result.insert(
        "metric".to_string(),
        JsonValue::String(nearest.metric.as_str().to_string()),
    );
    JsonValue::Object(result)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::{Column, Expr, VectorMetric};

    #[test]
    fn find_many_args_serializes_supported_filters_and_includes() {
        let args = FindManyArgs {
            where_: Some(
                Column::<String>::new("Entry", "slug")
                    .contains("rust-entry-")
                    .and(Expr::column("Entry__id").gt(Expr::param(2))),
            ),
            order_by: vec![
                Column::<i32>::new("Entry", "id").asc(),
                Column::<String>::new("Entry", "slug").desc(),
            ],
            take: Some(2),
            skip: Some(1),
            include: HashMap::from([(
                "author".to_string(),
                IncludeRelation::with_filter(
                    Column::<String>::new("User", "email").eq("a@example.com"),
                )
                .with_order_by(Column::<i32>::new("User", "id").desc())
                .with_take(1)
                .with_include("posts", IncludeRelation::plain()),
            )]),
            select: HashMap::from([("id".to_string(), true), ("slug".to_string(), true)]),
            cursor: Some(HashMap::from([("id".to_string(), Value::I32(10))])),
            distinct: vec!["Entry__slug".to_string()],
            nearest: Some(VectorNearest {
                field: "Entry__embedding".to_string(),
                query: vec![1.0, 2.0, 3.0],
                metric: VectorMetric::Cosine,
            }),
        };

        let json = find_many_args_to_protocol_json(&args).expect("serialization should succeed");
        let object =
            find_many_args_to_protocol_object(&args).expect("object serialization should succeed");

        assert_eq!(
            json,
            json!({
                "where": {
                    "AND": [
                        { "slug": { "contains": "rust-entry-" } },
                        { "id": { "gt": 2 } }
                    ]
                },
                "orderBy": [{ "id": "asc" }, { "slug": "desc" }],
                "take": 2,
                "skip": 1,
                "include": {
                    "author": {
                        "where": { "email": "a@example.com" },
                        "orderBy": [{ "id": "desc" }],
                        "take": 1,
                        "include": {
                            "posts": {}
                        }
                    }
                },
                "select": {
                    "id": true,
                    "slug": true
                },
                "cursor": {
                    "id": 10
                },
                "distinct": ["slug"],
                "nearest": {
                    "field": "embedding",
                    "query": [1.0, 2.0, 3.0],
                    "metric": "cosine"
                }
            })
        );
        assert_eq!(JsonValue::Object(object), json);
    }
}
