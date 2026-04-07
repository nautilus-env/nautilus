//! SQL dialect renderers for Nautilus ORM.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

// These macros accept identifier parameters (`$quote`, `$render_expr`) so that
// each dialect module supplies only the logic that differs between dialects.
// Free identifiers in macro bodies (types, constants) are resolved at the
// *definition site* (here in lib.rs), so the required types must be imported
// below.  Identifier parameters (`$quote:ident`, `$render_expr:ident`) are
// substituted textually at the call site, which is the intended behaviour.

/// Append `RETURNING col1 AS alias1, ...` when `$returning` is non-empty.
macro_rules! render_returning {
    ($ctx:expr, $returning:expr, $quote:ident) => {{
        if !$returning.is_empty() {
            $ctx.sql.push_str(" RETURNING ");
            for (i, col) in $returning.iter().enumerate() {
                if i > 0 {
                    $ctx.sql.push_str(", ");
                }
                $ctx.sql.push_str(&$quote(&col.table));
                $ctx.sql.push('.');
                $ctx.sql.push_str(&$quote(&col.name));
                $ctx.sql.push_str(" AS ");
                $ctx.sql.push_str(&$quote(&col.alias()));
            }
        }
    }};
}

/// Render the full body of an INSERT statement into `$ctx`.
///
/// `$supports_returning`: when `false` the RETURNING clause is omitted (MySQL).
macro_rules! render_insert_body {
    ($ctx:expr, $insert:expr, $quote:ident, $supports_returning:expr, $supports_enum_cast:expr) => {{
        $ctx.sql.push_str("INSERT INTO ");
        $ctx.sql.push_str(&$quote(&$insert.table));

        $ctx.sql.push_str(" (");
        for (i, col) in $insert.columns.iter().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            $ctx.sql.push_str(&$quote(&col.name));
        }
        $ctx.sql.push(')');

        $ctx.sql.push_str(" VALUES ");
        for (row_idx, row) in $insert.values.iter().enumerate() {
            if row_idx > 0 {
                $ctx.sql.push_str(", ");
            }
            $ctx.sql.push('(');
            for (val_idx, value) in row.iter().enumerate() {
                if val_idx > 0 {
                    $ctx.sql.push_str(", ");
                }
                if matches!(value, nautilus_core::Value::Null) {
                    $ctx.sql.push_str("NULL");
                } else {
                    let placeholder = $ctx.push_param(value.clone());
                    $ctx.sql.push_str(&placeholder);
                    if $supports_enum_cast {
                        if let nautilus_core::Value::Enum { type_name, .. } = value {
                            $ctx.sql.push_str("::");
                            $ctx.sql.push_str(type_name);
                        }
                    }
                }
            }
            $ctx.sql.push(')');
        }

        if $supports_returning {
            render_returning!($ctx, $insert.returning, $quote);
        }
    }};
}

/// Render the full body of an UPDATE statement into `$ctx`.
///
/// `$render_expr`: the dialect-local expression renderer.
/// `$supports_returning`: when `false` the RETURNING clause is omitted (MySQL).
macro_rules! render_update_body {
    ($ctx:expr, $update:expr, $quote:ident, $render_expr:ident, $supports_returning:expr, $supports_enum_cast:expr) => {{
        $ctx.sql.push_str("UPDATE ");
        $ctx.sql.push_str(&$quote(&$update.table));

        $ctx.sql.push_str(" SET ");
        for (i, (col, value)) in $update.assignments.iter().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            $ctx.sql.push_str(&$quote(&col.name));
            $ctx.sql.push_str(" = ");
            if matches!(value, nautilus_core::Value::Null) {
                $ctx.sql.push_str("NULL");
            } else {
                let placeholder = $ctx.push_param(value.clone());
                $ctx.sql.push_str(&placeholder);
                if $supports_enum_cast {
                    if let nautilus_core::Value::Enum { type_name, .. } = value {
                        $ctx.sql.push_str("::");
                        $ctx.sql.push_str(type_name);
                    }
                }
            }
        }

        if let Some(ref filter) = $update.filter {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if $supports_returning {
            render_returning!($ctx, $update.returning, $quote);
        }
    }};
}

/// Render the full body of a DELETE statement into `$ctx`.
///
/// `$render_expr`: the dialect-local expression renderer.
/// `$supports_returning`: when `false` the RETURNING clause is omitted (MySQL).
macro_rules! render_delete_body {
    ($ctx:expr, $delete:expr, $quote:ident, $render_expr:ident, $supports_returning:expr) => {{
        $ctx.sql.push_str("DELETE FROM ");
        $ctx.sql.push_str(&$quote(&$delete.table));

        if let Some(ref filter) = $delete.filter {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if $supports_returning {
            render_returning!($ctx, $delete.returning, $quote);
        }
    }};
}

/// Render the full body of a SELECT statement into `$ctx`.
///
/// - `$distinct_on`: `true` for PostgreSQL-style `DISTINCT ON (cols)`;
///   `false` emits plain `SELECT DISTINCT`.
/// - `$mysql_limit_hack`: `true` inserts a synthetic `LIMIT 18446744073709551615`
///   when only OFFSET is present (required by MySQL).
/// - `$render_expr`: the dialect-local expression renderer.
macro_rules! render_select_body_core {
    (
        $ctx:expr, $select:expr,
        $quote:ident, $render_expr:ident,
        $distinct_on:expr, $mysql_limit_hack:expr
    ) => {{
        $ctx.sql.push_str("SELECT ");

        // DISTINCT handling: Postgres supports DISTINCT ON (cols);
        // other dialects support only full-row SELECT DISTINCT.
        if !$select.distinct.is_empty() {
            if $distinct_on {
                $ctx.sql.push_str("DISTINCT ON (");
                for (i, col) in $select.distinct.iter().enumerate() {
                    if i > 0 {
                        $ctx.sql.push_str(", ");
                    }
                    crate::push_identifier_reference(&mut $ctx.sql, col, $quote);
                }
                $ctx.sql.push_str(") ");
            } else {
                $ctx.sql.push_str("DISTINCT ");
            }
        }

        let join_items: Vec<&nautilus_core::SelectItem> =
            $select.joins.iter().flat_map(|j| j.items.iter()).collect();
        let has_items = !$select.items.is_empty() || !join_items.is_empty();

        if !has_items {
            $ctx.sql.push('*');
        } else {
            let mut first = true;
            for item in $select.items.iter().chain(join_items.iter().copied()) {
                if !first {
                    $ctx.sql.push_str(", ");
                }
                first = false;
                match item {
                    nautilus_core::SelectItem::Column(col) => {
                        $ctx.sql.push_str(&$quote(&col.table));
                        $ctx.sql.push('.');
                        $ctx.sql.push_str(&$quote(&col.name));
                        $ctx.sql.push_str(" AS ");
                        $ctx.sql.push_str(&$quote(&col.alias()));
                    }
                    nautilus_core::SelectItem::Computed { expr, alias } => {
                        $ctx.sql.push('(');
                        $render_expr($ctx, expr);
                        $ctx.sql.push(')');
                        $ctx.sql.push_str(" AS ");
                        $ctx.sql.push_str(&$quote(alias));
                    }
                }
            }
        }

        $ctx.sql.push_str(" FROM ");
        $ctx.sql.push_str(&$quote(&$select.table));

        for join in &$select.joins {
            match join.join_type {
                nautilus_core::JoinType::Inner => $ctx.sql.push_str(" INNER JOIN "),
                nautilus_core::JoinType::Left => $ctx.sql.push_str(" LEFT JOIN "),
            }
            $ctx.sql.push_str(&$quote(&join.table));
            $ctx.sql.push_str(" ON ");
            $render_expr($ctx, &join.on);
        }

        if let Some(ref filter) = $select.filter {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if !$select.group_by.is_empty() {
            $ctx.sql.push_str(" GROUP BY ");
            for (i, col) in $select.group_by.iter().enumerate() {
                if i > 0 {
                    $ctx.sql.push_str(", ");
                }
                $ctx.sql.push_str(&$quote(&col.table));
                $ctx.sql.push('.');
                $ctx.sql.push_str(&$quote(&col.name));
            }
        }

        if let Some(ref having) = $select.having {
            $ctx.sql.push_str(" HAVING ");
            $render_expr($ctx, having);
        }

        let has_order_items = !$select.order_by_items.is_empty();
        let has_col_order = !$select.order_by.is_empty();
        let has_expr_order = !$select.order_by_exprs.is_empty();
        if has_order_items || has_col_order || has_expr_order {
            $ctx.sql.push_str(" ORDER BY ");
            let mut first = true;
            if has_order_items {
                for item in &$select.order_by_items {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    match item {
                        nautilus_core::OrderByItem::Column(order) => {
                            crate::push_identifier_reference(&mut $ctx.sql, &order.column, $quote);
                            match order.direction {
                                nautilus_core::OrderDir::Asc => $ctx.sql.push_str(" ASC"),
                                nautilus_core::OrderDir::Desc => $ctx.sql.push_str(" DESC"),
                            }
                        }
                        nautilus_core::OrderByItem::Expr(expr, dir) => {
                            $render_expr($ctx, expr);
                            match dir {
                                nautilus_core::OrderDir::Asc => $ctx.sql.push_str(" ASC"),
                                nautilus_core::OrderDir::Desc => $ctx.sql.push_str(" DESC"),
                            }
                        }
                    }
                }
            } else {
                for order in &$select.order_by {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    crate::push_identifier_reference(&mut $ctx.sql, &order.column, $quote);
                    match order.direction {
                        nautilus_core::OrderDir::Asc => $ctx.sql.push_str(" ASC"),
                        nautilus_core::OrderDir::Desc => $ctx.sql.push_str(" DESC"),
                    }
                }
                for (expr, dir) in &$select.order_by_exprs {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    $render_expr($ctx, expr);
                    match dir {
                        nautilus_core::OrderDir::Asc => $ctx.sql.push_str(" ASC"),
                        nautilus_core::OrderDir::Desc => $ctx.sql.push_str(" DESC"),
                    }
                }
            }
        }

        // MySQL requires LIMIT whenever OFFSET is present; emit a synthetic max value.
        if let Some(take) = $select.take {
            $ctx.sql.push_str(" LIMIT ");
            $ctx.sql.push_str(&take.unsigned_abs().to_string());
        } else if $mysql_limit_hack && $select.skip.is_some() {
            $ctx.sql.push_str(" LIMIT 18446744073709551615");
        }

        if let Some(skip) = $select.skip {
            $ctx.sql.push_str(" OFFSET ");
            $ctx.sql.push_str(&skip.to_string());
        }
    }};
}

/// Render the `Expr` variants that are **identical** across all SQL dialect renderers.
///
/// Eight variants (`Column`, `Not`, `Exists`, `NotExists`, `ScalarSubquery`,
/// `IsNull`, `IsNotNull`, `Literal`) have the same rendering logic in every
/// dialect — the only structural difference is which function is called to
/// quote identifiers and which function recurses for sub-expressions.
///
/// The four dialect-specific variants (`Param`, `Binary`, `FunctionCall`,
/// `Filter`) are provided by the caller as a block of match arms in
/// `{ $($specific:tt)* }` and are appended after the shared arms.
///
/// Parameters:
/// - `$ctx`: `&mut RenderContext` — mutable render context
/// - `$expr`: `&Expr` — the expression to render
/// - `$quote`: local identifier-quoting function
/// - `$render_expr`: dialect-local recursive expression renderer
/// - `$render_select_body`: dialect-local subquery renderer
/// - `{ $($specific:tt)* }`: match arms for dialect-specific variants
macro_rules! render_expr_common {
    (
        $ctx:expr, $expr:expr,
        $quote:ident, $render_expr:ident, $render_select_body:ident,
        { $($specific:tt)* }
    ) => {
        match $expr {
            // Split "table__column" into a qualified identifier pair; otherwise
            // render as a single unqualified identifier.
            nautilus_core::Expr::Column(name) => {
                crate::push_identifier_reference(&mut $ctx.sql, name, $quote);
            }
            nautilus_core::Expr::Not(inner) => {
                $ctx.sql.push_str("NOT (");
                $render_expr($ctx, inner);
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::Exists(subquery) => {
                $ctx.sql.push_str("EXISTS (");
                $render_select_body($ctx, subquery);
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::NotExists(subquery) => {
                $ctx.sql.push_str("NOT EXISTS (");
                $render_select_body($ctx, subquery);
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::Relation { op, relation } => {
                let is_exists = matches!(op, nautilus_core::expr::RelationFilterOp::Some);
                if is_exists {
                    $ctx.sql.push_str("EXISTS (SELECT * FROM ");
                } else {
                    $ctx.sql.push_str("NOT EXISTS (SELECT * FROM ");
                }
                $ctx.sql.push_str(&$quote(&relation.target_table));
                $ctx.sql.push_str(" WHERE ");
                crate::push_identifier_reference(
                    &mut $ctx.sql,
                    &format!("{}__{}", relation.target_table, relation.fk_db),
                    $quote,
                );
                $ctx.sql.push_str(" = ");
                crate::push_identifier_reference(
                    &mut $ctx.sql,
                    &format!("{}__{}", relation.parent_table, relation.pk_db),
                    $quote,
                );
                $ctx.sql.push_str(" AND ");
                if matches!(op, nautilus_core::expr::RelationFilterOp::Every) {
                    $ctx.sql.push_str("NOT (");
                    $render_expr($ctx, &relation.filter);
                    $ctx.sql.push(')');
                } else {
                    $render_expr($ctx, &relation.filter);
                }
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::ScalarSubquery(subquery) => {
                $ctx.sql.push('(');
                $render_select_body($ctx, subquery);
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::IsNull(inner) => {
                $ctx.sql.push('(');
                $render_expr($ctx, inner);
                $ctx.sql.push_str(" IS NULL)");
            }
            nautilus_core::Expr::IsNotNull(inner) => {
                $ctx.sql.push('(');
                $render_expr($ctx, inner);
                $ctx.sql.push_str(" IS NOT NULL)");
            }
            // Emit as a single-quoted SQL string literal with internal
            // single-quotes escaped by doubling.
            // Must only be called with trusted, static strings.
            nautilus_core::Expr::Literal(s) => {
                $ctx.sql.push('\'');
                $ctx.sql.push_str(&s.replace('\'', "''"));
                $ctx.sql.push('\'');
            }
            nautilus_core::Expr::List(exprs) => {
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 { $ctx.sql.push_str(", "); }
                    $render_expr($ctx, e);
                }
            }
            nautilus_core::Expr::CaseWhen { condition, then } => {
                $ctx.sql.push_str("CASE WHEN ");
                $render_expr($ctx, condition);
                $ctx.sql.push_str(" THEN ");
                $render_expr($ctx, then);
                $ctx.sql.push_str(" ELSE NULL END");
            }
            nautilus_core::Expr::Star => {
                $ctx.sql.push('*');
            }
            $($specific)*
        }
    };
}

mod mysql;
mod postgres;
mod sqlite;

pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use sqlite::SqliteDialect;

use nautilus_core::{Delete, Insert, Result, Select, Update, Value};

/// SQL query with bound parameters.
///
/// Separates the SQL text from parameter values for use with prepared statements.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Sql {
    /// The SQL query text with parameter placeholders.
    pub text: String,
    /// The parameter values to bind to the query.
    pub params: Vec<Value>,
}

/// Trait for SQL dialect renderers.
///
/// Allows rendering AST queries into dialect-specific SQL strings.
pub trait Dialect {
    /// Whether this dialect natively supports the RETURNING clause
    /// on INSERT, UPDATE, and DELETE statements.
    ///
    /// Dialects that return `false` (e.g. MySQL) will have RETURNING
    /// emulated at the connector layer via separate queries.
    fn supports_returning(&self) -> bool {
        true
    }

    /// Render a SELECT query into SQL.
    fn render_select(&self, select: &Select) -> Result<Sql>;

    /// Render an INSERT query into SQL.
    fn render_insert(&self, insert: &Insert) -> Result<Sql>;

    /// Render an UPDATE query into SQL.
    fn render_update(&self, update: &Update) -> Result<Sql>;

    /// Render a DELETE query into SQL.
    fn render_delete(&self, delete: &Delete) -> Result<Sql>;
}

/// Quote a SQL identifier with double quotes (ANSI standard; used by PostgreSQL and SQLite).
///
/// Internal double quotes are escaped by doubling them.
pub(crate) fn double_quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quote a SQL identifier with backticks (MySQL style).
///
/// Internal backticks are escaped by doubling them.
pub(crate) fn backtick_quote_identifier(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Render an identifier reference that may use the `table__column` shorthand.
///
/// The split happens only on the first `__`, so mapped column names like
/// `users__profile__slug` still render as `users.profile__slug`.
pub(crate) fn push_identifier_reference<F>(sql: &mut String, name: &str, quote: F)
where
    F: Fn(&str) -> String,
{
    if let Some((table, column)) = name.split_once("__") {
        sql.push_str(&quote(table));
        sql.push('.');
        sql.push_str(&quote(column));
    } else {
        sql.push_str(&quote(name));
    }
}

/// Return the SQL operator keyword for a standard scalar binary operation.
///
/// Call only for the nine scalar operators (Eq through Like).  Composite cases
/// (IN/NOT IN, array operators) must be handled separately by each dialect before
/// delegating to this helper.
#[inline]
pub(crate) fn binary_op_sql(op: &nautilus_core::BinaryOp) -> &'static str {
    match op {
        nautilus_core::BinaryOp::Eq => "=",
        nautilus_core::BinaryOp::Ne => "!=",
        nautilus_core::BinaryOp::Lt => "<",
        nautilus_core::BinaryOp::Le => "<=",
        nautilus_core::BinaryOp::Gt => ">",
        nautilus_core::BinaryOp::Ge => ">=",
        nautilus_core::BinaryOp::And => "AND",
        nautilus_core::BinaryOp::Or => "OR",
        nautilus_core::BinaryOp::Like => "LIKE",
        nautilus_core::BinaryOp::ArrayContains
        | nautilus_core::BinaryOp::ArrayContainedBy
        | nautilus_core::BinaryOp::ArrayOverlaps
        | nautilus_core::BinaryOp::In
        | nautilus_core::BinaryOp::NotIn => {
            unreachable!(
                "binary_op_sql: operator {:?} must be handled by dialect-specific code",
                op
            )
        }
    }
}
