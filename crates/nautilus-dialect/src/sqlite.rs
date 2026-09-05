//! SQLite SQL dialect renderer.

use crate::{Dialect, Sql};
use nautilus_core::{BinaryOp, Delete, Expr, Insert, OnConflict, Result, Select, Update, Value};

/// SQLite SQL dialect renderer.
#[derive(Debug, Clone, Copy)]
pub struct SqliteDialect;

/// Renders query ASTs into SQLite-compatible SQL with `?` placeholders
/// and double-quoted identifiers.
impl Dialect for SqliteDialect {
    fn render_select_owned(&self, mut select: Select) -> Result<Sql> {
        let mut ctx = RenderContext::with_estimate(crate::estimate_select_render(&select));
        render_select_body_core_mut!(&mut ctx, &mut select, '"', render_expr_owned, false, "-1");
        Ok(Sql {
            text: ctx.sql,
            params: ctx.params,
        })
    }

    fn render_insert_owned(&self, mut insert: Insert) -> Result<Sql> {
        let mut ctx = RenderContext::with_estimate(crate::estimate_insert_render(&insert));
        render_insert_body_mut!(
            &mut ctx,
            &mut insert,
            '"',
            true,
            crate::no_param_cast,
            render_on_conflict
        );
        Ok(Sql {
            text: ctx.sql,
            params: ctx.params,
        })
    }

    fn render_update_owned(&self, mut update: Update) -> Result<Sql> {
        let mut ctx = RenderContext::with_estimate(crate::estimate_update_render(&update));
        render_update_body_mut!(
            &mut ctx,
            &mut update,
            '"',
            render_expr_owned,
            true,
            crate::no_param_cast
        );
        Ok(Sql {
            text: ctx.sql,
            params: ctx.params,
        })
    }

    fn render_delete_owned(&self, mut delete: Delete) -> Result<Sql> {
        let mut ctx = RenderContext::with_estimate(crate::estimate_delete_render(&delete));
        render_delete_body_mut!(&mut ctx, &mut delete, '"', render_expr_owned, true);
        Ok(Sql {
            text: ctx.sql,
            params: ctx.params,
        })
    }
}

struct RenderContext {
    sql: String,
    params: Vec<Value>,
}

impl RenderContext {
    fn with_estimate(estimate: crate::RenderEstimate) -> Self {
        Self {
            sql: String::with_capacity(estimate.sql_capacity),
            params: Vec::with_capacity(estimate.params_capacity),
        }
    }

    fn push_param(&mut self, value: Value) {
        self.params.push(value);
        self.sql.push('?');
    }

    fn take_param(&mut self, value: &mut Value) {
        self.push_param(std::mem::replace(value, Value::Null));
    }
}

fn render_on_conflict(ctx: &mut RenderContext, on_conflict: &mut OnConflict) {
    render_on_conflict_body_mut!(
        ctx,
        on_conflict,
        '"',
        render_expr_owned,
        crate::no_param_cast
    );
}

fn render_select_body_owned(ctx: &mut RenderContext, select: &mut crate::Select) {
    render_select_body_core_mut!(ctx, select, '"', render_expr_owned, false, "-1");
}

fn render_expr_owned(ctx: &mut RenderContext, expr: &mut Expr) {
    render_expr_common_mut!(ctx, expr, '"', render_expr_owned, render_select_body_owned, {
        Expr::CompositeField {
            table,
            column,
            json_key,
            ..
        } => {
            ctx.sql.push_str("json_extract(");
            crate::push_qualified_identifier(&mut ctx.sql, table, column, '"');
            ctx.sql.push_str(", ");
            crate::push_json_object_path_literal(&mut ctx.sql, json_key);
            ctx.sql.push(')');
        }
        Expr::Param(value) => {
            if matches!(value, Value::Null) {
                ctx.sql.push_str("NULL");
            } else {
                ctx.take_param(value);
            }
        }
        Expr::Binary { left, op, right } => {
            if matches!(*op, BinaryOp::In | BinaryOp::NotIn) {
                ctx.sql.push('(');
                render_expr_owned(ctx, left.as_mut());
                ctx.sql.push(' ');
                ctx.sql
                    .push_str(if matches!(*op, BinaryOp::In) { "IN" } else { "NOT IN" });
                ctx.sql.push_str(" (");
                if let Expr::List(exprs) = right.as_mut() {
                    for (i, e) in exprs.iter_mut().enumerate() {
                        if i > 0 {
                            ctx.sql.push_str(", ");
                        }
                        render_expr_owned(ctx, e);
                    }
                } else {
                    render_expr_owned(ctx, right.as_mut());
                }
                ctx.sql.push(')');
                ctx.sql.push(')');
            } else if matches!(
                *op,
                BinaryOp::ArrayContains | BinaryOp::ArrayContainedBy | BinaryOp::ArrayOverlaps
            ) {
                match *op {
                    BinaryOp::ArrayContains => {
                        ctx.sql.push_str("NOT EXISTS (SELECT 1 FROM json_each(");
                        render_expr_owned(ctx, right.as_mut());
                        ctx.sql.push_str(") AS _rhs WHERE NOT EXISTS (SELECT 1 FROM json_each(");
                        render_expr_owned(ctx, left.as_mut());
                        ctx.sql.push_str(") AS _col WHERE _col.value IS _rhs.value))");
                    }
                    BinaryOp::ArrayContainedBy => {
                        ctx.sql.push_str("NOT EXISTS (SELECT 1 FROM json_each(");
                        render_expr_owned(ctx, left.as_mut());
                        ctx.sql.push_str(") AS _col WHERE NOT EXISTS (SELECT 1 FROM json_each(");
                        render_expr_owned(ctx, right.as_mut());
                        ctx.sql.push_str(") AS _rhs WHERE _col.value IS _rhs.value))");
                    }
                    BinaryOp::ArrayOverlaps => {
                        ctx.sql.push_str("EXISTS (SELECT 1 FROM json_each(");
                        render_expr_owned(ctx, left.as_mut());
                        ctx.sql.push_str(") AS _col WHERE EXISTS (SELECT 1 FROM json_each(");
                        render_expr_owned(ctx, right.as_mut());
                        ctx.sql.push_str(") AS _rhs WHERE _col.value IS _rhs.value))");
                    }
                    _ => unreachable!(),
                }
            } else {
                ctx.sql.push('(');
                render_expr_owned(ctx, left.as_mut());
                ctx.sql.push(' ');
                ctx.sql.push_str(crate::binary_op_sql(op));
                ctx.sql.push(' ');
                render_expr_owned(ctx, right.as_mut());
                if matches!(*op, BinaryOp::LikeEscape) {
                    ctx.sql.push_str(" ESCAPE '\\'");
                }
                ctx.sql.push(')');
            }
        }
        Expr::FunctionCall { name, args } => {
            let sqlite_name = match name.as_str() {
                "json_agg" => "json_group_array",
                "json_build_object" => "json_object",
                _ => name,
            };
            ctx.sql.push_str(sqlite_name);
            ctx.sql.push('(');
            for (i, arg) in args.iter_mut().enumerate() {
                if i > 0 {
                    ctx.sql.push_str(", ");
                }
                render_expr_owned(ctx, arg);
            }
            ctx.sql.push(')');
        }
        Expr::Filter { expr, predicate } => {
            render_expr_owned(ctx, expr.as_mut());
            ctx.sql.push_str(" FILTER (WHERE ");
            render_expr_owned(ctx, predicate.as_mut());
            ctx.sql.push(')');
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_renders_on_conflict_do_update() {
        let insert = Insert::into_table("users")
            .columns(vec![
                nautilus_core::ColumnMarker::new("users", "email"),
                nautilus_core::ColumnMarker::new("users", "name"),
            ])
            .values(vec![
                Value::String("alice@example.com".to_string()),
                Value::String("Alice".to_string()),
            ])
            .on_conflict(nautilus_core::OnConflict::do_update(
                vec![nautilus_core::ColumnMarker::new("users", "email")],
                vec![(
                    nautilus_core::ColumnMarker::new("users", "name"),
                    nautilus_core::Assignment::Value(Value::String("Alice II".to_string())),
                )],
            ))
            .build()
            .unwrap();

        let sql = SqliteDialect.render_insert(&insert).unwrap();

        assert_eq!(
            sql.text,
            "INSERT INTO \"users\" (\"email\", \"name\") VALUES (?, ?) \
             ON CONFLICT (\"email\") DO UPDATE SET \"name\" = ?"
        );
        assert_eq!(sql.params.len(), 3);
    }

    #[test]
    fn test_array_contains_operator() {
        let dialect = SqliteDialect;
        let expr = Expr::Binary {
            left: Box::new(Expr::column("posts__tags")),
            op: BinaryOp::ArrayContains,
            right: Box::new(Expr::param(Value::Array(vec![Value::String(
                "rust".to_string(),
            )]))),
        };
        let select = Select::from_table("posts").filter(expr).build().unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"posts\" WHERE NOT EXISTS (SELECT 1 FROM json_each(?) AS _rhs WHERE NOT EXISTS (SELECT 1 FROM json_each(\"posts\".\"tags\") AS _col WHERE _col.value IS _rhs.value))"
        );
        assert_eq!(sql.params.len(), 1);
        match &sql.params[0] {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0], Value::String("rust".to_string()));
            }
            _ => panic!("Expected Array value"),
        }
    }

    #[test]
    fn test_array_contained_by_operator() {
        let dialect = SqliteDialect;
        let expr = Expr::Binary {
            left: Box::new(Expr::column("posts__tags")),
            op: BinaryOp::ArrayContainedBy,
            right: Box::new(Expr::param(Value::Array(vec![
                Value::String("rust".to_string()),
                Value::String("go".to_string()),
            ]))),
        };
        let select = Select::from_table("posts").filter(expr).build().unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"posts\" WHERE NOT EXISTS (SELECT 1 FROM json_each(\"posts\".\"tags\") AS _col WHERE NOT EXISTS (SELECT 1 FROM json_each(?) AS _rhs WHERE _col.value IS _rhs.value))"
        );
        assert_eq!(sql.params.len(), 1);
        match &sql.params[0] {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], Value::String("rust".to_string()));
                assert_eq!(arr[1], Value::String("go".to_string()));
            }
            _ => panic!("Expected Array value"),
        }
    }

    #[test]
    fn test_array_overlaps_operator() {
        let dialect = SqliteDialect;
        let expr = Expr::Binary {
            left: Box::new(Expr::column("posts__tags")),
            op: BinaryOp::ArrayOverlaps,
            right: Box::new(Expr::param(Value::Array(vec![
                Value::String("rust".to_string()),
                Value::String("python".to_string()),
            ]))),
        };
        let select = Select::from_table("posts").filter(expr).build().unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"posts\" WHERE EXISTS (SELECT 1 FROM json_each(\"posts\".\"tags\") AS _col WHERE EXISTS (SELECT 1 FROM json_each(?) AS _rhs WHERE _col.value IS _rhs.value))"
        );
        assert_eq!(sql.params.len(), 1);
        match &sql.params[0] {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], Value::String("rust".to_string()));
                assert_eq!(arr[1], Value::String("python".to_string()));
            }
            _ => panic!("Expected Array value"),
        }
    }

    #[test]
    fn composite_field_ordering_uses_json_extract() {
        let dialect = SqliteDialect;
        let select = Select::from_table("shipments")
            .order_by_expr(
                Expr::composite_field(
                    "shipments",
                    "delivery_snapshot",
                    "eta_minutes",
                    "etaMinutes",
                    nautilus_core::JsonPathCast::Signed,
                ),
                nautilus_core::OrderDir::Asc,
            )
            .build()
            .unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"shipments\" ORDER BY json_extract(\"shipments\".\"delivery_snapshot\", '$.\"etaMinutes\"') ASC"
        );
    }
}
