//! PostgreSQL SQL dialect renderer.

use crate::{Dialect, Sql};
use nautilus_core::{BinaryOp, Delete, Expr, Insert, Result, Select, Update, Value};

/// PostgreSQL SQL dialect renderer.
///
/// Uses `$1, $2, ...` numbered parameter placeholders and double-quoted identifiers.
/// Supports `RETURNING`, `DISTINCT ON`, PostgreSQL array operators, UUID type casts,
/// and `FILTER (WHERE ...)` on aggregates.
#[derive(Debug, Clone, Copy)]
pub struct PostgresDialect;

impl Dialect for PostgresDialect {
    fn render_select_owned(&self, mut select: Select) -> Result<Sql> {
        let mut ctx = RenderContext::with_estimate(crate::estimate_select_render(&select));
        render_select_body_core_mut!(&mut ctx, &mut select, '"', render_expr_owned, true, false);
        Ok(Sql {
            text: ctx.sql,
            params: ctx.params,
        })
    }

    fn render_insert_owned(&self, mut insert: Insert) -> Result<Sql> {
        let mut ctx = RenderContext::with_estimate(crate::estimate_insert_render(&insert));
        render_insert_body_mut!(&mut ctx, &mut insert, '"', true, postgres_assignment_cast);
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
            postgres_assignment_cast
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
        self.sql.push('$');
        crate::push_usize(&mut self.sql, self.params.len());
    }

    fn take_param(&mut self, value: &mut Value) {
        self.push_param(std::mem::replace(value, Value::Null));
    }
}

fn render_select_body_owned(ctx: &mut RenderContext, select: &mut crate::Select) {
    render_select_body_core_mut!(ctx, select, '"', render_expr_owned, true, false);
}

fn render_expr_owned(ctx: &mut RenderContext, expr: &mut Expr) {
    render_expr_common_mut!(ctx, expr, '"', render_expr_owned, render_select_body_owned, {
        Expr::CompositeField {
            table,
            column,
            field,
            ..
        } => {
            crate::push_composite_field_reference(&mut ctx.sql, table, column, field, '"');
        }
        Expr::Param(value) => {
            // NULL is emitted literally; PostgreSQL cannot implicitly resolve a
            // typed NULL sent as an unknown OID via the binary protocol.
            if matches!(value, Value::Null) {
                ctx.sql.push_str("NULL");
            } else {
                let cast = postgres_param_cast(value);
                ctx.take_param(value);
                if let Some(cast) = cast {
                    cast.push_sql(&mut ctx.sql);
                }
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
            } else {
                ctx.sql.push('(');
                render_expr_owned(ctx, left.as_mut());
                ctx.sql.push(' ');
                ctx.sql.push_str(match *op {
                    BinaryOp::ArrayContains => "@>",
                    BinaryOp::ArrayContainedBy => "<@",
                    BinaryOp::ArrayOverlaps => "&&",
                    _ => crate::binary_op_sql(op),
                });
                ctx.sql.push(' ');
                render_expr_owned(ctx, right.as_mut());
                ctx.sql.push(')');
            }
        }
        Expr::FunctionCall { name, args } => {
            if args.len() == 2 {
                let op = match name.as_str() {
                    nautilus_core::expr::VECTOR_L2_DISTANCE_FUNCTION => Some("<->"),
                    nautilus_core::expr::VECTOR_INNER_PRODUCT_FUNCTION => Some("<#>"),
                    nautilus_core::expr::VECTOR_COSINE_DISTANCE_FUNCTION => Some("<=>"),
                    _ => None,
                };
                if let Some(op) = op {
                    ctx.sql.push('(');
                    render_expr_owned(ctx, &mut args[0]);
                    ctx.sql.push(' ');
                    ctx.sql.push_str(op);
                    ctx.sql.push(' ');
                    render_expr_owned(ctx, &mut args[1]);
                    ctx.sql.push(')');
                    return;
                }
            }
            ctx.sql.push_str(name);
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

enum ParamCast {
    Static(&'static str),
    Enum(String),
    Composite(String),
}

impl ParamCast {
    fn push_sql(&self, sql: &mut String) {
        match self {
            Self::Static(name) => {
                sql.push_str("::");
                sql.push_str(name);
            }
            Self::Enum(type_name) | Self::Composite(type_name) => {
                sql.push_str("::");
                crate::push_quoted_identifier(sql, type_name, '"');
            }
        }
    }
}

/// The rendered `::type` suffix a bound parameter needs in an INSERT or UPDATE,
/// or `None` when the parameter binds as its own type already.
fn postgres_assignment_cast(value: &Value) -> Option<String> {
    let cast = postgres_param_cast(value)?;
    let mut sql = String::new();
    cast.push_sql(&mut sql);
    Some(sql)
}

fn postgres_param_cast(value: &Value) -> Option<ParamCast> {
    match value {
        Value::Uuid(_) => Some(ParamCast::Static("uuid")),
        Value::Json(_) => Some(ParamCast::Static("json")),
        Value::Vector(_) => Some(ParamCast::Static("vector")),
        Value::Geometry(_) => Some(ParamCast::Static("geometry")),
        Value::Geography(_) => Some(ParamCast::Static("geography")),
        value if is_homogeneous_geometry_array(value) => Some(ParamCast::Static("geometry[]")),
        value if is_homogeneous_geography_array(value) => Some(ParamCast::Static("geography[]")),
        Value::Enum { type_name, .. } => Some(ParamCast::Enum(type_name.clone())),
        Value::Composite { type_name, .. } => Some(ParamCast::Composite(type_name.clone())),
        _ => None,
    }
}

fn is_homogeneous_geometry_array(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(items) if !items.is_empty() && items.iter().all(|item| matches!(item, Value::Geometry(_)))
    )
}

fn is_homogeneous_geography_array(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(items) if !items.is_empty() && items.iter().all(|item| matches!(item, Value::Geography(_)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote_identifier(name: &str) -> String {
        let mut sql = String::new();
        crate::push_quoted_identifier(&mut sql, name, '"');
        sql
    }

    #[test]
    fn test_quote_identifier() {
        assert_eq!(quote_identifier("users"), "\"users\"");
        assert_eq!(quote_identifier("email"), "\"email\"");
        assert_eq!(quote_identifier("foo\"bar"), "\"foo\"\"bar\"");
        assert_eq!(quote_identifier("a\"b\"c"), "\"a\"\"b\"\"c\"");
    }

    #[test]
    fn test_array_contains_operator() {
        let dialect = PostgresDialect;
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
            "SELECT * FROM \"posts\" WHERE (\"posts\".\"tags\" @> $1)"
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
        let dialect = PostgresDialect;
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
            "SELECT * FROM \"posts\" WHERE (\"posts\".\"tags\" <@ $1)"
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
        let dialect = PostgresDialect;
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
            "SELECT * FROM \"posts\" WHERE (\"posts\".\"tags\" && $1)"
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
    fn test_array_operators_with_integers() {
        let dialect = PostgresDialect;
        let expr = Expr::Binary {
            left: Box::new(Expr::column("posts__scores")),
            op: BinaryOp::ArrayContains,
            right: Box::new(Expr::param(Value::Array(vec![
                Value::I32(100),
                Value::I32(200),
            ]))),
        };
        let select = Select::from_table("posts").filter(expr).build().unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"posts\" WHERE (\"posts\".\"scores\" @> $1)"
        );
        assert_eq!(sql.params.len(), 1);
        match &sql.params[0] {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], Value::I32(100));
                assert_eq!(arr[1], Value::I32(200));
            }
            _ => panic!("Expected Array value"),
        }
    }

    #[test]
    fn vector_params_are_cast_to_pgvector_type() {
        let dialect = PostgresDialect;
        let select = Select::from_table("embeddings")
            .filter(
                Expr::column("embeddings__vector")
                    .eq(Expr::param(Value::Vector(vec![1.0, 2.0, 3.0]))),
            )
            .build()
            .unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"embeddings\" WHERE (\"embeddings\".\"vector\" = $1::vector)"
        );
        assert_eq!(sql.params, vec![Value::Vector(vec![1.0, 2.0, 3.0])]);
    }

    #[test]
    fn postgis_params_are_cast_to_spatial_types() {
        let dialect = PostgresDialect;
        let select = Select::from_table("places")
            .filter(
                Expr::column("places__geom")
                    .eq(Expr::param(Value::Geometry("POINT(1 2)".to_string()))),
            )
            .build()
            .unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"places\" WHERE (\"places\".\"geom\" = $1::geometry)"
        );
        assert_eq!(sql.params, vec![Value::Geometry("POINT(1 2)".to_string())]);

        let select = Select::from_table("places")
            .filter(
                Expr::column("places__geog")
                    .eq(Expr::param(Value::Geography("POINT(1 2)".to_string()))),
            )
            .build()
            .unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"places\" WHERE (\"places\".\"geog\" = $1::geography)"
        );
        assert_eq!(sql.params, vec![Value::Geography("POINT(1 2)".to_string())]);
    }

    #[test]
    fn composite_params_are_cast_to_their_type_name() {
        let dialect = PostgresDialect;
        let composite = Value::Composite {
            type_name: "ChampionStatsT".to_string(),
            fields: vec![Value::I32(0), Value::I32(0)],
        };
        let select = Select::from_table("champions")
            .filter(Expr::column("champions__stats").eq(Expr::param(composite.clone())))
            .build()
            .unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"champions\" WHERE (\"champions\".\"stats\" = $1::\"ChampionStatsT\")"
        );
        assert_eq!(sql.params, vec![composite]);
    }

    #[test]
    fn composite_insert_and_update_params_are_cast_to_their_type_name() {
        let dialect = PostgresDialect;
        let composite = Value::Composite {
            type_name: "ChampionStatsT".to_string(),
            fields: vec![Value::I32(0), Value::I32(0)],
        };

        let insert = Insert::into_table("champions")
            .column(nautilus_core::ColumnMarker::new("champions", "stats"))
            .values(vec![composite.clone()])
            .build()
            .unwrap();
        let sql = dialect.render_insert(&insert).unwrap();

        assert_eq!(
            sql.text,
            "INSERT INTO \"champions\" (\"stats\") VALUES ($1::\"ChampionStatsT\")"
        );
        assert_eq!(sql.params, vec![composite.clone()]);

        let update = Update::table("champions")
            .set(
                nautilus_core::ColumnMarker::new("champions", "stats"),
                composite.clone(),
            )
            .build()
            .unwrap();
        let sql = dialect.render_update(&update).unwrap();

        assert_eq!(
            sql.text,
            "UPDATE \"champions\" SET \"stats\" = $1::\"ChampionStatsT\""
        );
        assert_eq!(sql.params, vec![composite]);
    }

    #[test]
    fn text_bound_insert_and_update_params_are_cast_to_their_column_type() {
        let dialect = PostgresDialect;
        let vector = Value::Vector(vec![1.0, 0.0, 0.0]);

        let insert = Insert::into_table("docs")
            .column(nautilus_core::ColumnMarker::new("docs", "embedding"))
            .values(vec![vector.clone()])
            .build()
            .unwrap();

        assert_eq!(
            dialect.render_insert(&insert).unwrap().text,
            "INSERT INTO \"docs\" (\"embedding\") VALUES ($1::vector)",
            "pgvector, PostGIS and JSON parameters all bind as text, so PostgreSQL \
             rejects them against their real column type without an explicit cast"
        );

        let update = Update::table("docs")
            .set(
                nautilus_core::ColumnMarker::new("docs", "embedding"),
                vector,
            )
            .build()
            .unwrap();

        assert_eq!(
            dialect.render_update(&update).unwrap().text,
            "UPDATE \"docs\" SET \"embedding\" = $1::vector"
        );
    }

    #[test]
    fn composite_field_ordering_uses_native_attribute_syntax() {
        let dialect = PostgresDialect;
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
            "SELECT * FROM \"shipments\" ORDER BY (\"shipments\".\"delivery_snapshot\").\"eta_minutes\" ASC"
        );
    }

    #[test]
    fn vector_distance_ordering_uses_pgvector_operator() {
        let dialect = PostgresDialect;
        let select = Select::from_table("embeddings")
            .order_by_expr(
                Expr::vector_distance(
                    nautilus_core::VectorMetric::Cosine,
                    Expr::column("embeddings__vector"),
                    Expr::param(Value::Vector(vec![1.0, 2.0, 3.0])),
                ),
                nautilus_core::OrderDir::Asc,
            )
            .take(5)
            .build()
            .unwrap();
        let sql = dialect.render_select(&select).unwrap();

        assert_eq!(
            sql.text,
            "SELECT * FROM \"embeddings\" ORDER BY (\"embeddings\".\"vector\" <=> $1::vector) ASC LIMIT 5"
        );
        assert_eq!(sql.params, vec![Value::Vector(vec![1.0, 2.0, 3.0])]);
    }
}
