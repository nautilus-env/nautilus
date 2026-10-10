//! PostgreSQL SQL dialect renderer.

use crate::dialect::{Dialect, Sql, SqlOutput};
use crate::macros::expressions::render_expr_common_mut;
use crate::macros::select::render_select_body_core_mut;
use crate::macros::write::{
    render_delete_body_mut, render_insert_body_mut, render_on_conflict_body_mut,
    render_update_body_mut,
};
use crate::output_aliases::{column_name, OutputAliases};
use nautilus_core::{BinaryOp, Delete, Expr, Insert, OnConflict, Result, Select, Update, Value};

/// PostgreSQL SQL dialect renderer.
///
/// Uses `$1, $2, ...` numbered parameter placeholders and double-quoted identifiers.
/// Supports `RETURNING`, `DISTINCT ON`, PostgreSQL array operators, UUID type casts,
/// and `FILTER (WHERE ...)` on aggregates.
#[derive(Debug, Clone, Copy)]
pub struct PostgresDialect;

impl Dialect for PostgresDialect {
    fn supports_distinct_on(&self) -> bool {
        true
    }
    fn render_select_owned(&self, select: Select) -> Result<Sql> {
        Self::render_select_with_aliases(select, OutputAliases::default()).map(|output| output.sql)
    }
    fn render_select_output_owned(&self, select: Select) -> Result<SqlOutput> {
        let aliases = OutputAliases::for_select(&select);
        Self::render_select_with_aliases(select, aliases)
    }
    fn render_insert_owned(&self, insert: Insert) -> Result<Sql> {
        Self::render_insert_with_aliases(insert, OutputAliases::default()).map(|output| output.sql)
    }
    fn render_insert_output_owned(&self, insert: Insert) -> Result<SqlOutput> {
        let aliases = OutputAliases::for_returning(&insert.returning);
        Self::render_insert_with_aliases(insert, aliases)
    }
    fn render_update_owned(&self, update: Update) -> Result<Sql> {
        Self::render_update_with_aliases(update, OutputAliases::default()).map(|output| output.sql)
    }
    fn render_update_output_owned(&self, update: Update) -> Result<SqlOutput> {
        let aliases = OutputAliases::for_returning(&update.returning);
        Self::render_update_with_aliases(update, aliases)
    }
    fn render_delete_owned(&self, delete: Delete) -> Result<Sql> {
        Self::render_delete_with_aliases(delete, OutputAliases::default()).map(|output| output.sql)
    }
    fn render_delete_output_owned(&self, delete: Delete) -> Result<SqlOutput> {
        let aliases = OutputAliases::for_returning(&delete.returning);
        Self::render_delete_with_aliases(delete, aliases)
    }
}

impl PostgresDialect {
    fn render_select_with_aliases(mut select: Select, aliases: OutputAliases) -> Result<SqlOutput> {
        let mut ctx =
            RenderContext::with_estimate(crate::render_estimate::estimate_select_render(&select));
        ctx.aliases = aliases;
        render_select_body_core_mut!(&mut ctx, &mut select, '"', render_expr_owned, true, "");
        Ok(ctx.finish())
    }

    fn render_insert_with_aliases(mut insert: Insert, aliases: OutputAliases) -> Result<SqlOutput> {
        let mut ctx =
            RenderContext::with_estimate(crate::render_estimate::estimate_insert_render(&insert));
        ctx.aliases = aliases;
        render_insert_body_mut!(
            &mut ctx,
            &mut insert,
            '"',
            true,
            postgres_assignment_cast,
            render_on_conflict
        );
        Ok(ctx.finish())
    }

    fn render_update_with_aliases(mut update: Update, aliases: OutputAliases) -> Result<SqlOutput> {
        let mut ctx =
            RenderContext::with_estimate(crate::render_estimate::estimate_update_render(&update));
        ctx.aliases = aliases;
        render_update_body_mut!(
            &mut ctx,
            &mut update,
            '"',
            render_expr_owned,
            true,
            postgres_assignment_cast
        );
        Ok(ctx.finish())
    }

    fn render_delete_with_aliases(mut delete: Delete, aliases: OutputAliases) -> Result<SqlOutput> {
        let mut ctx =
            RenderContext::with_estimate(crate::render_estimate::estimate_delete_render(&delete));
        ctx.aliases = aliases;
        render_delete_body_mut!(&mut ctx, &mut delete, '"', render_expr_owned, true);
        Ok(ctx.finish())
    }
}

struct RenderContext {
    sql: String,
    params: Vec<Value>,
    aliases: OutputAliases,
}

impl RenderContext {
    fn finish(self) -> SqlOutput {
        SqlOutput {
            sql: Sql {
                text: self.sql,
                params: self.params,
            },
            aliases: self.aliases.into_restored(),
        }
    }

    fn push_result_alias(&mut self, original: &str, quote: char) {
        crate::ident::push_quoted_identifier(&mut self.sql, self.aliases.sql_name(original), quote);
    }

    fn push_column_order(&mut self, order: &nautilus_core::OrderBy, quote: char) {
        if let Some(alias) = self.aliases.computed_sql_name(&order.column) {
            crate::ident::push_quoted_identifier(&mut self.sql, alias, quote);
            crate::clauses::push_direction(&mut self.sql, order.direction);
        } else {
            crate::clauses::push_column_order(&mut self.sql, order, quote);
        }
    }

    fn push_distinct(&mut self, distinct: &[String], distinct_on: bool, quote: char) {
        if self.aliases.is_empty() || !distinct_on {
            crate::clauses::push_distinct(&mut self.sql, distinct, distinct_on, quote);
            return;
        }
        if distinct.is_empty() {
            return;
        }
        self.sql.push_str("DISTINCT ON (");
        for (index, column) in distinct.iter().enumerate() {
            if index > 0 {
                self.sql.push_str(", ");
            }
            if let Some(alias) = self.aliases.computed_sql_name(column) {
                crate::ident::push_quoted_identifier(&mut self.sql, alias, quote);
            } else {
                crate::ident::push_identifier_reference(&mut self.sql, column, quote);
            }
        }
        self.sql.push_str(") ");
    }

    fn push_aliased_column(&mut self, column: &nautilus_core::ColumnMarker, quote: char) {
        if self.aliases.is_empty() {
            crate::clauses::push_aliased_column(&mut self.sql, column, quote);
            return;
        }
        crate::ident::push_qualified_identifier(&mut self.sql, &column.table, &column.name, quote);
        self.sql.push_str(" AS ");
        self.push_result_alias(&column_name(column), quote);
    }

    fn push_window_projection(&mut self, select: &Select, quote: char) {
        if self.aliases.is_empty() {
            crate::clauses::push_window_projection(&mut self.sql, select, quote);
            return;
        }
        let mut first = true;
        for item in select
            .items
            .iter()
            .chain(select.joins.iter().flat_map(|join| &join.items))
        {
            if !first {
                self.sql.push_str(", ");
            }
            first = false;
            match item {
                nautilus_core::SelectItem::Column(column) => {
                    self.push_result_alias(&column_name(column), quote)
                }
                nautilus_core::SelectItem::Computed { alias, .. } => {
                    self.push_result_alias(alias, quote)
                }
            }
        }
        if first {
            self.sql.push('*');
        }
    }

    fn push_returning(&mut self, columns: &[nautilus_core::ColumnMarker], quote: char) {
        if columns.is_empty() {
            return;
        }
        self.sql.push_str(" RETURNING ");
        for (index, column) in columns.iter().enumerate() {
            if index > 0 {
                self.sql.push_str(", ");
            }
            self.push_aliased_column(column, quote);
        }
    }

    fn with_estimate(estimate: crate::render_estimate::RenderEstimate) -> Self {
        Self {
            sql: String::with_capacity(estimate.sql_capacity),
            params: Vec::with_capacity(estimate.params_capacity),
            aliases: OutputAliases::default(),
        }
    }

    fn push_param(&mut self, value: Value) {
        self.params.push(value);
        self.sql.push('$');
        crate::ident::push_usize(&mut self.sql, self.params.len());
    }

    fn take_param(&mut self, value: &mut Value) {
        // An empty array has no element to take a type from, and bound as
        // `text[]` PostgreSQL assigns it to no other array column. The untyped
        // literal takes the type of the column or operand it meets instead.
        if matches!(value, Value::Array(items) if items.is_empty()) {
            self.sql.push_str("'{}'");
            return;
        }
        self.push_param(std::mem::replace(value, Value::Null));
    }
}

fn render_select_body_owned(ctx: &mut RenderContext, select: &mut nautilus_core::Select) {
    render_select_body_core_mut!(ctx, select, '"', render_expr_owned, true, "");
}

fn render_expr_owned(ctx: &mut RenderContext, expr: &mut Expr) {
    render_expr_common_mut!(ctx, expr, '"', render_expr_owned, render_select_body_owned, {
        Expr::CompositeField {
            table,
            column,
            field,
            ..
        } => {
            crate::ident::push_composite_field_reference(&mut ctx.sql, table, column, field, '"');
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
                    _ => crate::expr::binary_op_sql(op),
                });
                ctx.sql.push(' ');
                render_expr_owned(ctx, right.as_mut());
                if matches!(*op, BinaryOp::LikeEscape) {
                    ctx.sql.push_str(" ESCAPE '\\'");
                }
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
            if let [arg] = args.as_mut_slice() {
                let uuid_aggregate = match name.as_str() {
                    nautilus_core::expr::UUID_MIN_FUNCTION => Some("MIN("),
                    nautilus_core::expr::UUID_MAX_FUNCTION => Some("MAX("),
                    _ => None,
                };
                if let Some(open) = uuid_aggregate {
                    // The canonical text of a UUID, compared byte by byte,
                    // sorts as the 16 bytes of the UUID do.
                    ctx.sql.push_str(open);
                    render_expr_owned(ctx, arg);
                    ctx.sql.push_str("::text COLLATE \"C\")::uuid");
                    return;
                }
            }
            ctx.sql.push_str(match name.as_str() {
                nautilus_core::expr::BOOLEAN_MIN_FUNCTION => "BOOL_AND",
                nautilus_core::expr::BOOLEAN_MAX_FUNCTION => "BOOL_OR",
                name => name,
            });
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
    NamedArray(String),
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
                crate::ident::push_quoted_identifier(sql, type_name, '"');
            }
            Self::NamedArray(type_name) => {
                sql.push_str("::");
                crate::ident::push_quoted_identifier(sql, type_name, '"');
                sql.push_str("[]");
            }
        }
    }
}

/// The rendered `::type` suffix a bound parameter needs in an INSERT or UPDATE,
/// or `None` when the parameter binds as its own type already.
fn render_on_conflict(ctx: &mut RenderContext, on_conflict: &mut OnConflict) {
    render_on_conflict_body_mut!(
        ctx,
        on_conflict,
        '"',
        render_expr_owned,
        postgres_assignment_cast
    );
}

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
        Value::Array(items) => homogeneous_text_array_type(items).map(ParamCast::NamedArray),
        Value::Extension { type_name, .. } | Value::Enum { type_name, .. } => {
            Some(ParamCast::Enum(type_name.clone()))
        }
        Value::Composite { type_name, .. } => Some(ParamCast::Composite(type_name.clone())),
        _ => None,
    }
}

/// Enum labels and extension values bind as `text[]`, so a homogeneous list
/// needs a cast to the array of its named type, just as a scalar does.
fn homogeneous_text_array_type(items: &[Value]) -> Option<String> {
    let first = items.first()?;
    let type_name = match first {
        Value::Enum { type_name, .. } | Value::Extension { type_name, .. } => type_name,
        _ => return None,
    };
    items
        .iter()
        .all(|item| match (first, item) {
            (
                Value::Enum { .. },
                Value::Enum {
                    type_name: other, ..
                },
            )
            | (
                Value::Extension { .. },
                Value::Extension {
                    type_name: other, ..
                },
            ) => other == type_name,
            _ => false,
        })
        .then(|| type_name.clone())
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

    #[test]
    fn upsert_renders_on_conflict_do_update_after_the_values() {
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
            .returning(vec![nautilus_core::ColumnMarker::new("users", "id")])
            .build()
            .unwrap();

        let sql = PostgresDialect.render_insert(&insert).unwrap();

        assert_eq!(
            sql.text,
            "INSERT INTO \"users\" (\"email\", \"name\") VALUES ($1, $2) \
             ON CONFLICT (\"email\") DO UPDATE SET \"name\" = $3 \
             RETURNING \"users\".\"id\" AS \"users__id\""
        );
        assert_eq!(
            sql.params,
            vec![
                Value::String("alice@example.com".to_string()),
                Value::String("Alice".to_string()),
                Value::String("Alice II".to_string()),
            ]
        );
    }

    #[test]
    fn an_expression_assignment_reads_the_column_it_writes() {
        let update = nautilus_core::Update::table("posts")
            .set(
                nautilus_core::ColumnMarker::new("posts", "views"),
                Expr::Binary {
                    left: Box::new(Expr::column("views")),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::param(Value::I32(5))),
                },
            )
            .set(
                nautilus_core::ColumnMarker::new("posts", "title"),
                Value::String("kept".to_string()),
            )
            .build()
            .unwrap();

        let sql = PostgresDialect.render_update(&update).unwrap();

        assert_eq!(
            sql.text,
            "UPDATE \"posts\" SET \"views\" = (\"views\" + $1), \"title\" = $2"
        );
        assert_eq!(
            sql.params,
            vec![Value::I32(5), Value::String("kept".to_string())]
        );
    }

    #[test]
    fn upsert_without_assignments_renders_do_nothing() {
        let insert = Insert::into_table("users")
            .columns(vec![nautilus_core::ColumnMarker::new("users", "email")])
            .values(vec![Value::String("alice@example.com".to_string())])
            .on_conflict(nautilus_core::OnConflict::do_nothing(vec![
                nautilus_core::ColumnMarker::new("users", "email"),
            ]))
            .build()
            .unwrap();

        let sql = PostgresDialect.render_insert(&insert).unwrap();

        assert_eq!(
            sql.text,
            "INSERT INTO \"users\" (\"email\") VALUES ($1) ON CONFLICT (\"email\") DO NOTHING"
        );
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
    fn empty_array_params_render_as_an_untyped_literal() {
        let dialect = PostgresDialect;
        let tags = Value::Array(vec![Value::String("gift".to_string())]);

        let insert = Insert::into_table("orders")
            .columns(vec![
                nautilus_core::ColumnMarker::new("orders", "unit_prices"),
                nautilus_core::ColumnMarker::new("orders", "tags"),
            ])
            .values(vec![Value::Array(vec![]), tags.clone()])
            .build()
            .unwrap();
        let sql = dialect.render_insert(&insert).unwrap();
        assert_eq!(
            sql.text, "INSERT INTO \"orders\" (\"unit_prices\", \"tags\") VALUES ('{}', $1)",
            "a bound empty array is text[], which a numeric[] column does not accept"
        );
        assert_eq!(sql.params, vec![tags]);

        let update = Update::table("orders")
            .set(
                nautilus_core::ColumnMarker::new("orders", "unit_prices"),
                Value::Array(vec![]),
            )
            .build()
            .unwrap();
        let sql = dialect.render_update(&update).unwrap();
        assert_eq!(sql.text, "UPDATE \"orders\" SET \"unit_prices\" = '{}'");
        assert!(sql.params.is_empty());

        let select = Select::from_table("orders")
            .filter(Expr::Binary {
                left: Box::new(Expr::column("orders__unit_prices")),
                op: BinaryOp::ArrayContains,
                right: Box::new(Expr::param(Value::Array(vec![]))),
            })
            .build()
            .unwrap();
        assert_eq!(
            dialect.render_select(&select).unwrap().text,
            "SELECT * FROM \"orders\" WHERE (\"orders\".\"unit_prices\" @> '{}')"
        );
    }

    #[test]
    fn enum_array_params_are_cast_to_the_array_of_their_enum() {
        let dialect = PostgresDialect;
        let label = |value: &str| Value::Enum {
            value: value.to_string(),
            type_name: "orderstatus".to_string(),
        };
        let history = Value::Array(vec![label("PENDING"), label("SHIPPED")]);

        let insert = Insert::into_table("orders")
            .columns(vec![nautilus_core::ColumnMarker::new("orders", "history")])
            .values(vec![history.clone()])
            .build()
            .unwrap();
        assert_eq!(
            dialect.render_insert(&insert).unwrap().text,
            "INSERT INTO \"orders\" (\"history\") VALUES ($1::\"orderstatus\"[])",
            "the labels bind as text[], which PostgreSQL assigns to no enum array column"
        );

        let update = Update::table("orders")
            .set(
                nautilus_core::ColumnMarker::new("orders", "history"),
                history.clone(),
            )
            .build()
            .unwrap();
        assert_eq!(
            dialect.render_update(&update).unwrap().text,
            "UPDATE \"orders\" SET \"history\" = $1::\"orderstatus\"[]"
        );

        let select = Select::from_table("orders")
            .filter(Expr::Binary {
                left: Box::new(Expr::column("orders__history")),
                op: BinaryOp::ArrayContains,
                right: Box::new(Expr::param(history)),
            })
            .build()
            .unwrap();
        assert_eq!(
            dialect.render_select(&select).unwrap().text,
            "SELECT * FROM \"orders\" WHERE (\"orders\".\"history\" @> $1::\"orderstatus\"[])"
        );

        let mixed = Value::Array(vec![label("PENDING"), Value::String("SHIPPED".to_string())]);
        assert!(postgres_param_cast(&mixed).is_none());
    }

    #[test]
    fn extension_array_params_are_cast_to_the_array_of_their_type() {
        let dialect = PostgresDialect;
        for type_name in ["ltree", "citext", "custom\"type"] {
            let extension = |value: &str| Value::Extension {
                value: value.to_string(),
                type_name: type_name.to_string(),
            };
            let values = Value::Array(vec![extension("top.c"), extension("")]);
            let quoted_type = type_name.replace('"', "\"\"");
            let cast = format!("::\"{quoted_type}\"[]");
            let insert = Insert::into_table("orders")
                .columns(vec![nautilus_core::ColumnMarker::new("orders", "paths")])
                .values(vec![values.clone()])
                .build()
                .unwrap();
            let sql = dialect.render_insert(&insert).unwrap();
            assert_eq!(
                sql.text,
                format!("INSERT INTO \"orders\" (\"paths\") VALUES ($1{cast})")
            );
            assert_eq!(sql.params, vec![values.clone()]);

            let update = Update::table("orders")
                .set(
                    nautilus_core::ColumnMarker::new("orders", "paths"),
                    values.clone(),
                )
                .build()
                .unwrap();
            let sql = dialect.render_update(&update).unwrap();
            assert_eq!(
                sql.text,
                format!("UPDATE \"orders\" SET \"paths\" = $1{cast}")
            );
            assert_eq!(sql.params, vec![values.clone()]);

            let select = Select::from_table("orders")
                .filter(Expr::Binary {
                    left: Box::new(Expr::column("orders__paths")),
                    op: BinaryOp::ArrayContains,
                    right: Box::new(Expr::param(values.clone())),
                })
                .build()
                .unwrap();
            let sql = dialect.render_select(&select).unwrap();
            assert_eq!(
                sql.text,
                format!("SELECT * FROM \"orders\" WHERE (\"orders\".\"paths\" @> $1{cast})")
            );
            assert_eq!(sql.params, vec![values]);

            for other in [
                Value::Extension {
                    value: "top.d".into(),
                    type_name: "other".into(),
                },
                Value::Enum {
                    value: "top.d".into(),
                    type_name: type_name.into(),
                },
                Value::String("top.d".into()),
                Value::Null,
            ] {
                assert!(
                    postgres_param_cast(&Value::Array(vec![extension("top.c"), other])).is_none()
                );
            }
        }
        assert!(postgres_param_cast(&Value::Array(vec![])).is_none());
    }

    #[test]
    fn extension_scalar_params_are_cast_to_their_type() {
        let dialect = PostgresDialect;
        let citext = Value::Extension {
            value: "slug-near".to_string(),
            type_name: "citext".to_string(),
        };

        let select = Select::from_table("docs")
            .filter(Expr::column("docs__slug").eq(Expr::param(citext.clone())))
            .build()
            .unwrap();

        assert_eq!(
            dialect.render_select(&select).unwrap().text,
            "SELECT * FROM \"docs\" WHERE (\"docs\".\"slug\" = $1::\"citext\")",
            "without the cast PostgreSQL resolves citext = text and compares case sensitively"
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
