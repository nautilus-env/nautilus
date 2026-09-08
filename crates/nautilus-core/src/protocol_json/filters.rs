//! The `where` argument: a filter [`Expr`] written as the object the engine
//! parses back into an expression.
//!
//! A binary operator becomes `{ field: { op: value } }`, equality collapses to
//! `{ field: value }`, and `AND` / `OR` become arrays that flatten the nested
//! pairs the builder produced, so an expression built left to right keeps its
//! order on the wire.

use serde_json::{Map as JsonMap, Value as JsonValue};

use super::expressions::{
    expr_value_to_json, like_operator_and_value, list_expr_to_json_array, strip_column_qualifier,
};
use crate::expr::RelationFilterOp;
use crate::{BinaryOp, Error, Expr, Result};

/// Convert a single Rust filter expression into the engine wire-format `"where"` object.
pub fn where_expr_to_protocol_json(expr: &Expr) -> Result<JsonValue> {
    expr_to_filter_json(expr)
}

pub(super) fn expr_to_filter_json(expr: &Expr) -> Result<JsonValue> {
    match expr {
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
        } => logical_expr_to_json("AND", left, right),
        Expr::Binary {
            left,
            op: BinaryOp::Or,
            right,
        } => logical_expr_to_json("OR", left, right),
        Expr::Not(inner) => {
            let mut result = JsonMap::new();
            result.insert("NOT".to_string(), expr_to_filter_json(inner)?);
            Ok(JsonValue::Object(result))
        }
        Expr::Relation { op, relation } => {
            relation_predicate_to_json(&relation.field, *op, relation.filter.as_ref())
        }
        Expr::Binary { left, op, right } => field_predicate_to_json(left, op, right),
        Expr::IsNull(inner) => null_predicate_to_json(inner, true),
        Expr::IsNotNull(inner) => null_predicate_to_json(inner, false),
        other => Err(Error::InvalidQuery(format!(
            "query cannot be serialized to engine JSON: unsupported expression {:?}",
            other
        ))),
    }
}

fn relation_predicate_to_json(
    field: &str,
    op: RelationFilterOp,
    filter: &Expr,
) -> Result<JsonValue> {
    let mut relation_spec = JsonMap::with_capacity(1);
    relation_spec.insert(
        match op {
            RelationFilterOp::Some => "some".to_string(),
            RelationFilterOp::None => "none".to_string(),
            RelationFilterOp::Every => "every".to_string(),
        },
        expr_to_filter_json(filter)?,
    );

    let mut result = JsonMap::with_capacity(1);
    result.insert(field.to_string(), JsonValue::Object(relation_spec));
    Ok(JsonValue::Object(result))
}

fn logical_expr_to_json(name: &str, left: &Expr, right: &Expr) -> Result<JsonValue> {
    let mut items =
        Vec::with_capacity(logical_operand_count(name, left) + logical_operand_count(name, right));
    collect_logical_operands(name, left, &mut items)?;
    collect_logical_operands(name, right, &mut items)?;

    let mut result = JsonMap::with_capacity(1);
    result.insert(name.to_string(), JsonValue::Array(items));
    Ok(JsonValue::Object(result))
}

fn logical_operand_count(name: &str, expr: &Expr) -> usize {
    match (name, expr) {
        (
            "AND",
            Expr::Binary {
                left,
                op: BinaryOp::And,
                right,
            },
        ) => logical_operand_count(name, left) + logical_operand_count(name, right),
        (
            "OR",
            Expr::Binary {
                left,
                op: BinaryOp::Or,
                right,
            },
        ) => logical_operand_count(name, left) + logical_operand_count(name, right),
        _ => 1,
    }
}

fn collect_logical_operands(name: &str, expr: &Expr, out: &mut Vec<JsonValue>) -> Result<()> {
    match (name, expr) {
        (
            "AND",
            Expr::Binary {
                left,
                op: BinaryOp::And,
                right,
            },
        ) => {
            collect_logical_operands(name, left, out)?;
            collect_logical_operands(name, right, out)?;
            Ok(())
        }
        (
            "OR",
            Expr::Binary {
                left,
                op: BinaryOp::Or,
                right,
            },
        ) => {
            collect_logical_operands(name, left, out)?;
            collect_logical_operands(name, right, out)?;
            Ok(())
        }
        _ => {
            out.push(expr_to_filter_json(expr)?);
            Ok(())
        }
    }
}

fn field_predicate_to_json(left: &Expr, op: &BinaryOp, right: &Expr) -> Result<JsonValue> {
    let field = match left {
        Expr::Column(name) => strip_column_qualifier(name),
        other => {
            return Err(Error::InvalidQuery(format!(
                "query cannot be serialized to engine JSON: unsupported field operand {:?}",
                other
            )));
        }
    };

    let (operator, value) = match op {
        BinaryOp::Eq => (None, expr_value_to_json(right)?),
        BinaryOp::Ne => (Some("ne"), expr_value_to_json(right)?),
        BinaryOp::Lt => (Some("lt"), expr_value_to_json(right)?),
        BinaryOp::Le => (Some("lte"), expr_value_to_json(right)?),
        BinaryOp::Gt => (Some("gt"), expr_value_to_json(right)?),
        BinaryOp::Ge => (Some("gte"), expr_value_to_json(right)?),
        BinaryOp::Like => like_operator_and_value(right, false)?,
        BinaryOp::LikeEscape => like_operator_and_value(right, true)?,
        BinaryOp::In => (Some("in"), list_expr_to_json_array(right)?),
        BinaryOp::NotIn => (Some("notIn"), list_expr_to_json_array(right)?),
        other => {
            return Err(Error::InvalidQuery(format!(
                "query cannot be serialized to engine JSON: unsupported binary op {:?}",
                other
            )));
        }
    };

    let mut result = JsonMap::with_capacity(1);
    match operator {
        None => {
            result.insert(field, value);
        }
        Some(op_name) => {
            let mut operators = JsonMap::with_capacity(1);
            operators.insert(op_name.to_string(), value);
            result.insert(field, JsonValue::Object(operators));
        }
    }

    Ok(JsonValue::Object(result))
}

fn null_predicate_to_json(inner: &Expr, is_null: bool) -> Result<JsonValue> {
    let field = match inner {
        Expr::Column(name) => strip_column_qualifier(name),
        other => {
            return Err(Error::InvalidQuery(format!(
                "query cannot be serialized to engine JSON: unsupported null predicate {:?}",
                other
            )));
        }
    };

    let mut operators = JsonMap::with_capacity(1);
    operators.insert("isNull".to_string(), JsonValue::Bool(is_null));

    let mut result = JsonMap::with_capacity(1);
    result.insert(field, JsonValue::Object(operators));
    Ok(JsonValue::Object(result))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Column, Select};

    fn filter_json(expr: Expr) -> JsonValue {
        where_expr_to_protocol_json(&expr).expect("filter should serialize")
    }

    #[test]
    fn comparison_membership_and_null_operators_map_to_their_wire_names() {
        let views = || Expr::column("Entry__views");

        assert_eq!(
            filter_json(views().eq(Expr::param(1))),
            json!({ "views": 1 })
        );
        assert_eq!(
            filter_json(views().ne(Expr::param(1))),
            json!({ "views": { "ne": 1 } })
        );
        assert_eq!(
            filter_json(views().lt(Expr::param(1))),
            json!({ "views": { "lt": 1 } })
        );
        assert_eq!(
            filter_json(views().le(Expr::param(1))),
            json!({ "views": { "lte": 1 } })
        );
        assert_eq!(
            filter_json(views().gt(Expr::param(1))),
            json!({ "views": { "gt": 1 } })
        );
        assert_eq!(
            filter_json(views().ge(Expr::param(1))),
            json!({ "views": { "gte": 1 } })
        );
        assert_eq!(
            filter_json(views().in_list(vec![Expr::param(1), Expr::param(2)])),
            json!({ "views": { "in": [1, 2] } })
        );
        assert_eq!(
            filter_json(views().not_in_list(vec![Expr::param(1)])),
            json!({ "views": { "notIn": [1] } })
        );
        assert_eq!(
            filter_json(views().is_null()),
            json!({ "views": { "isNull": true } })
        );
        assert_eq!(
            filter_json(views().is_not_null()),
            json!({ "views": { "isNull": false } })
        );
        assert_eq!(
            filter_json(!views().is_null()),
            json!({ "NOT": { "views": { "isNull": true } } })
        );
    }

    #[test]
    fn nested_or_flattens_into_one_array_in_operand_order() {
        let published = Expr::column("Entry__published").eq(Expr::param(true));
        let views = Expr::column("Entry__views").gt(Expr::param(10));
        let slug = Expr::column("Entry__slug").eq(Expr::param("rust"));

        assert_eq!(
            filter_json(published.or(views).or(slug)),
            json!({
                "OR": [
                    { "published": true },
                    { "views": { "gt": 10 } },
                    { "slug": "rust" }
                ]
            })
        );
    }

    #[test]
    fn unsupported_expression_returns_invalid_query() {
        let expr = Expr::exists(
            Select::from_table("Post")
                .filter(Expr::column("Post__user_id").eq(Expr::column("User__id")))
                .build()
                .expect("valid select"),
        );

        let err = where_expr_to_protocol_json(&expr).expect_err("exists is not serializable");
        assert!(matches!(err, Error::InvalidQuery(_)));
    }

    #[test]
    fn relation_predicates_serialize_to_relation_where_objects() {
        let expr = Expr::relation_some(
            "posts",
            "User",
            "Post",
            "user_id",
            "id",
            Column::<String>::new("Post", "title")
                .contains("rust")
                .and(Expr::relation_none(
                    "comments",
                    "Post",
                    "Comment",
                    "post_id",
                    "id",
                    Column::<bool>::new("Comment", "flagged").eq(false),
                )),
        )
        .and(Expr::relation_every(
            "posts",
            "User",
            "Post",
            "user_id",
            "id",
            Column::<bool>::new("Post", "published").eq(true),
        ));

        assert_eq!(
            filter_json(expr),
            json!({
                "AND": [
                    {
                        "posts": {
                            "some": {
                                "AND": [
                                    { "title": { "contains": "rust" } },
                                    { "comments": { "none": { "flagged": false } } }
                                ]
                            }
                        }
                    },
                    {
                        "posts": {
                            "every": {
                                "published": true
                            }
                        }
                    }
                ]
            })
        );
    }
}
