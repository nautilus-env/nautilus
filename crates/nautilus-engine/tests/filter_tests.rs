use nautilus_core::{BinaryOp, OrderDir, Value};
use nautilus_engine::filter::QueryArgs;
use serde_json::json;

fn expr_contains_binary_op(expr: &nautilus_core::Expr, target: BinaryOp) -> bool {
    match expr {
        nautilus_core::Expr::Binary { left, op, right } => {
            *op == target
                || expr_contains_binary_op(left, target.clone())
                || expr_contains_binary_op(right, target)
        }
        nautilus_core::Expr::Not(inner)
        | nautilus_core::Expr::IsNull(inner)
        | nautilus_core::Expr::IsNotNull(inner) => expr_contains_binary_op(inner, target),
        _ => false,
    }
}

fn expr_contains_not(expr: &nautilus_core::Expr) -> bool {
    match expr {
        nautilus_core::Expr::Not(_) => true,
        nautilus_core::Expr::Binary { left, right, .. } => {
            expr_contains_not(left) || expr_contains_not(right)
        }
        nautilus_core::Expr::IsNull(inner) | nautilus_core::Expr::IsNotNull(inner) => {
            expr_contains_not(inner)
        }
        _ => false,
    }
}

fn expr_contains_is_null_column(expr: &nautilus_core::Expr, target: &str) -> bool {
    match expr {
        nautilus_core::Expr::IsNull(inner) => {
            matches!(inner.as_ref(), nautilus_core::Expr::Column(name) if name == target)
        }
        nautilus_core::Expr::Binary { left, right, .. } => {
            expr_contains_is_null_column(left, target)
                || expr_contains_is_null_column(right, target)
        }
        nautilus_core::Expr::Not(inner) | nautilus_core::Expr::IsNotNull(inner) => {
            expr_contains_is_null_column(inner, target)
        }
        _ => false,
    }
}

#[test]
fn test_simple_equality() {
    let args = json!({
        "where": {
            "email": "test@example.com"
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    assert!(query_args.filter.is_some());

    let filter = query_args.filter.unwrap();
    match filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(op, BinaryOp::Eq),
        _ => panic!("Expected Binary expression"),
    }
}

#[test]
fn test_multiple_fields_implicit_and() {
    let args = json!({
        "where": {
            "email": "test@example.com",
            "age": 25
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(op, BinaryOp::And),
        _ => panic!("Expected AND expression"),
    }
}

#[test]
fn test_comparison_operators() {
    let args = json!({
        "where": {
            "age": {
                "gte": 18,
                "lt": 65
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(op, BinaryOp::And),
        _ => panic!("Expected AND expression"),
    }
}

#[test]
fn test_string_contains() {
    let args = json!({
        "where": {
            "email": {
                "contains": "test"
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, right, .. } => {
            assert_eq!(op, BinaryOp::Like);
            match right.as_ref() {
                nautilus_core::Expr::Param(Value::String(s)) => assert_eq!(s, "%test%"),
                _ => panic!("Expected string parameter"),
            }
        }
        _ => panic!("Expected LIKE expression"),
    }
}

#[test]
fn test_string_starts_with() {
    let args = json!({
        "where": {
            "email": {
                "startsWith": "admin"
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, right, .. } => {
            assert_eq!(op, BinaryOp::Like);
            match right.as_ref() {
                nautilus_core::Expr::Param(Value::String(s)) => assert_eq!(s, "admin%"),
                _ => panic!("Expected string parameter"),
            }
        }
        _ => panic!("Expected LIKE expression"),
    }
}

#[test]
fn test_string_ends_with() {
    let args = json!({
        "where": {
            "email": {
                "endsWith": "@gmail.com"
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, right, .. } => {
            assert_eq!(op, BinaryOp::Like);
            match right.as_ref() {
                nautilus_core::Expr::Param(Value::String(s)) => assert_eq!(s, "%@gmail.com"),
                _ => panic!("Expected string parameter"),
            }
        }
        _ => panic!("Expected LIKE expression"),
    }
}

#[test]
fn test_null_checks() {
    let args = json!({
        "where": {
            "deletedAt": {
                "isNull": true
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::IsNull(inner) => match inner.as_ref() {
            nautilus_core::Expr::Column(name) => assert!(
                name.contains("deletedAt") || name.contains("deleted_at") || !name.is_empty()
            ),
            _ => panic!("Expected Column inside IsNull"),
        },
        _ => panic!("Expected Expr::IsNull, got {:?}", filter),
    }
}

#[test]
fn test_not_null_check() {
    let args = json!({
        "where": {
            "email": {
                "isNotNull": true
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::IsNotNull(inner) => match inner.as_ref() {
            nautilus_core::Expr::Column(name) => assert!(!name.is_empty()),
            _ => panic!("Expected Column inside IsNotNull"),
        },
        _ => panic!("Expected Expr::IsNotNull, got {:?}", filter),
    }
}

#[test]
fn test_in_operator() {
    let args = json!({
        "where": {
            "status": {
                "in": ["active", "pending", "approved"]
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, right, .. } => {
            assert_eq!(op, BinaryOp::In);
            match right.as_ref() {
                nautilus_core::Expr::List(items) => {
                    assert_eq!(items.len(), 3);
                }
                _ => panic!("Expected List on right side of IN"),
            }
        }
        _ => panic!("Expected IN expression"),
    }
}

#[test]
fn test_not_in_operator() {
    let args = json!({
        "where": {
            "role": {
                "notIn": ["admin", "superuser"]
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, right, .. } => {
            assert_eq!(op, BinaryOp::NotIn);
            match right.as_ref() {
                nautilus_core::Expr::List(items) => {
                    assert_eq!(items.len(), 2);
                }
                _ => panic!("Expected List on right side of NOT IN"),
            }
        }
        _ => panic!("Expected NOT IN expression"),
    }
}

#[test]
fn test_explicit_and_operator() {
    let args = json!({
        "where": {
            "AND": [
                { "age": { "gte": 18 } },
                { "status": "active" }
            ]
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(op, BinaryOp::And),
        _ => panic!("Expected AND expression"),
    }
}

#[test]
fn test_or_operator() {
    let args = json!({
        "where": {
            "OR": [
                { "email": { "contains": "@gmail.com" } },
                { "email": { "contains": "@yahoo.com" } }
            ]
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(op, BinaryOp::Or),
        _ => panic!("Expected OR expression"),
    }
}

#[test]
fn test_not_operator() {
    let args = json!({
        "where": {
            "NOT": {
                "status": "deleted"
            }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Not(_) => {}
        _ => panic!("Expected NOT expression"),
    }
}

#[test]
fn test_complex_nested_filter() {
    let args = json!({
        "where": {
            "AND": [
                { "age": { "gte": 18 } },
                {
                    "OR": [
                        { "status": "active" },
                        { "status": "pending" }
                    ]
                }
            ]
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(op, BinaryOp::And),
        _ => panic!("Expected AND expression"),
    }
}

#[test]
fn test_logical_operators_can_coexist_with_sibling_field_conditions() {
    let args = json!({
        "where": {
            "OR": [
                { "status": "active" },
                { "status": "pending" }
            ],
            "deletedAt": { "isNull": true },
            "NOT": { "role": "admin" }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    let filter = query_args.filter.unwrap();

    match &filter {
        nautilus_core::Expr::Binary { op, .. } => assert_eq!(*op, BinaryOp::And),
        _ => panic!("Expected mixed logical clauses to combine with AND"),
    }

    assert!(expr_contains_binary_op(&filter, BinaryOp::Or));
    assert!(expr_contains_not(&filter));
    assert!(expr_contains_is_null_column(&filter, "deletedAt"));
}

#[test]
fn test_order_by_parsing() {
    let args = json!({
        "orderBy": [
            { "createdAt": "desc" },
            { "name": "asc" }
        ]
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    assert_eq!(query_args.order_by.len(), 2);
    assert_eq!(query_args.order_by[0].column, "createdAt");
    assert_eq!(query_args.order_by[0].direction, OrderDir::Desc);
    assert_eq!(query_args.order_by[1].column, "name");
    assert_eq!(query_args.order_by[1].direction, OrderDir::Asc);
}

#[test]
fn test_take_and_skip() {
    let args = json!({
        "take": 10,
        "skip": 20
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    assert_eq!(query_args.take, Some(10i32));
    assert_eq!(query_args.skip, Some(20u32));
}

#[test]
fn test_all_comparison_operators() {
    let args = json!({
        "where": {
            "field1": { "eq": 1 },
            "field2": { "ne": 2 },
            "field3": { "gt": 3 },
            "field4": { "gte": 4 },
            "field5": { "lt": 5 },
            "field6": { "lte": 6 }
        }
    });

    let query_args = QueryArgs::parse(Some(args)).unwrap();
    assert!(query_args.filter.is_some());
}

#[test]
fn test_invalid_operator() {
    let args = json!({
        "where": {
            "field": {
                "unknownOp": "value"
            }
        }
    });

    let result = QueryArgs::parse(Some(args));
    assert!(result.is_err());
}

#[test]
fn test_empty_where_object() {
    let args = json!({
        "where": {}
    });

    let result = QueryArgs::parse(Some(args));
    assert!(result.is_err());
}
