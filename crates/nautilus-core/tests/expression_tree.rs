//! Integration tests for nautilus-core expression tree composition.
//!
//! Tests AND/OR/NOT nesting, comparisons, IN lists, and subquery predicates.

use nautilus_core::{BinaryOp, ColumnMarker, Expr, Select, SelectItem, Value};

#[test]
fn eq_expr() {
    let expr = Expr::column("users__id").eq(Expr::param(Value::I64(1)));
    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::Eq,
            ..
        }
    ));
}

#[test]
fn ne_expr() {
    let expr = Expr::column("users__status").ne(Expr::param(Value::String("deleted".into())));
    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::Ne,
            ..
        }
    ));
}

#[test]
fn lt_gt_le_ge() {
    let lt = Expr::column("users__age").lt(Expr::param(Value::I64(18)));
    let gt = Expr::column("users__age").gt(Expr::param(Value::I64(65)));
    let le = Expr::column("users__score").le(Expr::param(Value::F64(100.0)));
    let ge = Expr::column("users__score").ge(Expr::param(Value::F64(0.0)));

    assert!(matches!(
        lt,
        Expr::Binary {
            op: BinaryOp::Lt,
            ..
        }
    ));
    assert!(matches!(
        gt,
        Expr::Binary {
            op: BinaryOp::Gt,
            ..
        }
    ));
    assert!(matches!(
        le,
        Expr::Binary {
            op: BinaryOp::Le,
            ..
        }
    ));
    assert!(matches!(
        ge,
        Expr::Binary {
            op: BinaryOp::Ge,
            ..
        }
    ));
}

#[test]
fn and_two_conditions() {
    let a = Expr::column("users__active").eq(Expr::param(Value::Bool(true)));
    let b = Expr::column("users__role").eq(Expr::param(Value::String("admin".into())));
    let expr = a.and(b);

    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::And,
            ..
        }
    ));
}

#[test]
fn or_two_conditions() {
    let a = Expr::column("users__role").eq(Expr::param(Value::String("admin".into())));
    let b = Expr::column("users__role").eq(Expr::param(Value::String("moderator".into())));
    let expr = a.or(b);

    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::Or,
            ..
        }
    ));
}

#[test]
fn nested_and_or() {
    let role_admin = Expr::column("users__role").eq(Expr::param(Value::String("admin".into())));
    let role_mod = Expr::column("users__role").eq(Expr::param(Value::String("moderator".into())));
    let active = Expr::column("users__active").eq(Expr::param(Value::Bool(true)));

    let expr = role_admin.or(role_mod).and(active);

    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::And,
            ..
        }
    ));
}

#[test]
fn not_wraps_inner() {
    let inner = Expr::column("users__deleted").eq(Expr::param(Value::Bool(true)));
    let expr = Expr::Not(Box::new(inner.clone()));

    assert!(matches!(expr, Expr::Not(_)));
    if let Expr::Not(boxed) = expr {
        assert_eq!(*boxed, inner);
    }
}

#[test]
fn in_list() {
    let ids = vec![
        Expr::param(Value::I64(1)),
        Expr::param(Value::I64(2)),
        Expr::param(Value::I64(3)),
    ];
    let expr = Expr::column("users__id").in_list(ids);

    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::In,
            ..
        }
    ));
}

#[test]
fn not_in_list() {
    let ids = vec![Expr::param(Value::I64(10)), Expr::param(Value::I64(20))];
    let expr = Expr::column("users__id").not_in_list(ids);

    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::NotIn,
            ..
        }
    ));
}

#[test]
fn like_expr() {
    let expr =
        Expr::column("users__email").like(Expr::param(Value::String("%@example.com".into())));
    assert!(matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::Like,
            ..
        }
    ));
}

#[test]
fn exists_subquery() {
    let subquery = Select::from_table("posts")
        .items(vec![SelectItem::column(ColumnMarker::new("posts", "id"))])
        .filter(Expr::column("posts__author_id").eq(Expr::column("users__id")))
        .build()
        .unwrap();

    let expr = Expr::Exists(Box::new(subquery));
    assert!(matches!(expr, Expr::Exists(_)));
}

#[test]
fn not_exists_subquery() {
    let subquery = Select::from_table("bans")
        .items(vec![SelectItem::column(ColumnMarker::new("bans", "id"))])
        .filter(Expr::column("bans__user_id").eq(Expr::column("users__id")))
        .build()
        .unwrap();

    let expr = Expr::NotExists(Box::new(subquery));
    assert!(matches!(expr, Expr::NotExists(_)));
}
