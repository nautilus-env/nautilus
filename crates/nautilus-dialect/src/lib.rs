//! SQL dialect renderers for Nautilus ORM.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

// These macros accept identifier parameters (`$quote`, `$render_expr`) so that
// each dialect module supplies only the logic that differs between dialects.
// Free identifiers in macro bodies (types, constants) are resolved at the
// *definition site* (here in lib.rs), so the required types must be imported
// below.  Identifier parameters (`$quote:expr`, `$render_expr:ident`) are
// substituted textually at the call site, which is the intended behaviour.

/// Append `RETURNING col1 AS alias1, ...` when `$returning` is non-empty.
///
/// All render paths consume the AST by value (moving bound values out instead of
/// cloning them), so the rendering macros take `&mut` and are the single source of
/// truth for SQL generation. The borrowed `Dialect::render_*` entry points simply
/// clone the AST once and delegate here.
macro_rules! render_returning_mut {
    ($ctx:expr, $returning:expr, $quote:expr) => {{
        if !$returning.is_empty() {
            $ctx.sql.push_str(" RETURNING ");
            for (i, col) in $returning.iter().enumerate() {
                if i > 0 {
                    $ctx.sql.push_str(", ");
                }
                crate::push_qualified_identifier(&mut $ctx.sql, &col.table, &col.name, $quote);
                $ctx.sql.push_str(" AS ");
                crate::push_column_alias(&mut $ctx.sql, col, $quote);
            }
        }
    }};
}

/// Mutable/owned variant of [`render_insert_body!`] used by `render_*_owned`.
macro_rules! render_insert_body_mut {
    ($ctx:expr, $insert:expr, $quote:expr, $supports_returning:expr, $param_cast:expr, $conflict_clause:ident) => {{
        $ctx.sql.push_str("INSERT INTO ");
        crate::push_table_name(&mut $ctx.sql, &$insert.table, $quote);

        $ctx.sql.push_str(" (");
        for (i, col) in $insert.columns.iter().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            crate::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
        }
        $ctx.sql.push(')');

        $ctx.sql.push_str(" VALUES ");
        for (row_idx, row) in $insert.values.iter_mut().enumerate() {
            if row_idx > 0 {
                $ctx.sql.push_str(", ");
            }
            $ctx.sql.push('(');
            for (val_idx, value) in row.iter_mut().enumerate() {
                if val_idx > 0 {
                    $ctx.sql.push_str(", ");
                }
                if matches!(value, nautilus_core::Value::Null) {
                    $ctx.sql.push_str("NULL");
                } else {
                    let cast = $param_cast(&*value);
                    $ctx.take_param(value);
                    if let Some(cast) = cast.as_deref() {
                        $ctx.sql.push_str(cast);
                    }
                }
            }
            $ctx.sql.push(')');
        }

        if let Some(on_conflict) = $insert.on_conflict.as_mut() {
            $conflict_clause($ctx, on_conflict);
        }

        if $supports_returning {
            render_returning_mut!($ctx, $insert.returning, $quote);
        }
    }};
}

/// Append the SQL-standard `ON CONFLICT (...) DO UPDATE SET ... ` / `DO NOTHING`
/// clause shared by PostgreSQL and SQLite.
///
/// MySQL spells the same idea `ON DUPLICATE KEY UPDATE` and has no conflict
/// target, so it renders its own clause instead of calling this.
macro_rules! render_on_conflict_body_mut {
    ($ctx:expr, $on_conflict:expr, $quote:expr, $param_cast:expr) => {{
        $ctx.sql.push_str(" ON CONFLICT (");
        for (i, col) in $on_conflict.target.iter().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            crate::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
        }
        $ctx.sql.push(')');

        if $on_conflict.update.is_empty() {
            $ctx.sql.push_str(" DO NOTHING");
        } else {
            $ctx.sql.push_str(" DO UPDATE SET ");
            for (i, (col, value)) in $on_conflict.update.iter_mut().enumerate() {
                if i > 0 {
                    $ctx.sql.push_str(", ");
                }
                crate::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
                $ctx.sql.push_str(" = ");
                if matches!(value, nautilus_core::Value::Null) {
                    $ctx.sql.push_str("NULL");
                } else {
                    let cast = $param_cast(&*value);
                    $ctx.take_param(value);
                    if let Some(cast) = cast.as_deref() {
                        $ctx.sql.push_str(cast);
                    }
                }
            }
        }
    }};
}

/// Mutable/owned variant of [`render_update_body!`] used by `render_*_owned`.
macro_rules! render_update_body_mut {
    ($ctx:expr, $update:expr, $quote:expr, $render_expr:ident, $supports_returning:expr, $param_cast:expr) => {{
        $ctx.sql.push_str("UPDATE ");
        crate::push_table_name(&mut $ctx.sql, &$update.table, $quote);

        $ctx.sql.push_str(" SET ");
        for (i, (col, value)) in $update.assignments.iter_mut().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            crate::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
            $ctx.sql.push_str(" = ");
            if matches!(value, nautilus_core::Value::Null) {
                $ctx.sql.push_str("NULL");
            } else {
                let cast = $param_cast(&*value);
                $ctx.take_param(value);
                if let Some(cast) = cast.as_deref() {
                    $ctx.sql.push_str(cast);
                }
            }
        }

        if let Some(filter) = $update.filter.as_mut() {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if $supports_returning {
            render_returning_mut!($ctx, $update.returning, $quote);
        }
    }};
}

/// Mutable/owned variant of [`render_delete_body!`] used by `render_*_owned`.
macro_rules! render_delete_body_mut {
    ($ctx:expr, $delete:expr, $quote:expr, $render_expr:ident, $supports_returning:expr) => {{
        $ctx.sql.push_str("DELETE FROM ");
        crate::push_table_name(&mut $ctx.sql, &$delete.table, $quote);

        if let Some(filter) = $delete.filter.as_mut() {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if $supports_returning {
            render_returning_mut!($ctx, $delete.returning, $quote);
        }
    }};
}

/// Render the `ORDER BY` clause of a SELECT, prefixed by `$prefix`.
///
/// Shared by the statement-level clause and by the `OVER (...)` clause of a
/// partition window, which consumes the same ordering.
macro_rules! render_order_by_clause_mut {
    ($ctx:expr, $select:expr, $quote:expr, $render_expr:ident, $prefix:expr) => {{
        let has_order_items = !$select.order_by_items.is_empty();
        let has_col_order = !$select.order_by.is_empty();
        let has_expr_order = !$select.order_by_exprs.is_empty();
        if has_order_items || has_col_order || has_expr_order {
            $ctx.sql.push_str($prefix);
            $ctx.sql.push_str("ORDER BY ");
            let mut first = true;
            if has_order_items {
                for item in $select.order_by_items.iter_mut() {
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
                            match *dir {
                                nautilus_core::OrderDir::Asc => $ctx.sql.push_str(" ASC"),
                                nautilus_core::OrderDir::Desc => $ctx.sql.push_str(" DESC"),
                            }
                        }
                    }
                }
            } else {
                for order in $select.order_by.iter() {
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
                for (expr, dir) in $select.order_by_exprs.iter_mut() {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    $render_expr($ctx, expr);
                    match *dir {
                        nautilus_core::OrderDir::Asc => $ctx.sql.push_str(" ASC"),
                        nautilus_core::OrderDir::Desc => $ctx.sql.push_str(" DESC"),
                    }
                }
            }
        }
    }};
}

/// Render the outer projection of a partition-window subquery: the inner select
/// list referenced by alias, so the row-number column stays internal and the
/// result keeps exactly the columns an unwindowed render would return.
macro_rules! render_window_projection_mut {
    ($ctx:expr, $select:expr, $quote:expr) => {{
        let mut first = true;
        for item in $select.items.iter() {
            if !first {
                $ctx.sql.push_str(", ");
            }
            first = false;
            match item {
                nautilus_core::SelectItem::Column(col) => {
                    crate::push_column_alias(&mut $ctx.sql, col, $quote);
                }
                nautilus_core::SelectItem::Computed { alias, .. } => {
                    crate::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
                }
            }
        }
        for join in $select.joins.iter() {
            for item in join.items.iter() {
                if !first {
                    $ctx.sql.push_str(", ");
                }
                first = false;
                match item {
                    nautilus_core::SelectItem::Column(col) => {
                        crate::push_column_alias(&mut $ctx.sql, col, $quote);
                    }
                    nautilus_core::SelectItem::Computed { alias, .. } => {
                        crate::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
                    }
                }
            }
        }
        if first {
            $ctx.sql.push('*');
        }
    }};
}

/// Mutable/owned variant of [`render_select_body_core!`] used by `render_*_owned`.
macro_rules! render_select_body_core_mut {
    (
        $ctx:expr, $select:expr,
        $quote:expr, $render_expr:ident,
        $distinct_on:expr, $offset_limit_sentinel:expr
    ) => {{
        let partition_window = $select.partition_window.take();

        if partition_window.is_some() {
            $ctx.sql.push_str("SELECT ");
            render_window_projection_mut!($ctx, $select, $quote);
            $ctx.sql.push_str(" FROM (");
        }

        $ctx.sql.push_str("SELECT ");

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

        let has_items =
            !$select.items.is_empty() || $select.joins.iter().any(|join| !join.items.is_empty());

        if !has_items {
            $ctx.sql.push('*');
        } else {
            let mut first = true;
            for item in $select.items.iter_mut() {
                if !first {
                    $ctx.sql.push_str(", ");
                }
                first = false;
                match item {
                    nautilus_core::SelectItem::Column(col) => {
                        crate::push_qualified_identifier(
                            &mut $ctx.sql,
                            &col.table,
                            &col.name,
                            $quote,
                        );
                        $ctx.sql.push_str(" AS ");
                        crate::push_column_alias(&mut $ctx.sql, col, $quote);
                    }
                    nautilus_core::SelectItem::Computed { expr, alias } => {
                        $ctx.sql.push('(');
                        $render_expr($ctx, expr);
                        $ctx.sql.push(')');
                        $ctx.sql.push_str(" AS ");
                        crate::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
                    }
                }
            }
            for join in $select.joins.iter_mut() {
                for item in join.items.iter_mut() {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    match item {
                        nautilus_core::SelectItem::Column(col) => {
                            crate::push_qualified_identifier(
                                &mut $ctx.sql,
                                &col.table,
                                &col.name,
                                $quote,
                            );
                            $ctx.sql.push_str(" AS ");
                            crate::push_column_alias(&mut $ctx.sql, col, $quote);
                        }
                        nautilus_core::SelectItem::Computed { expr, alias } => {
                            $ctx.sql.push('(');
                            $render_expr($ctx, expr);
                            $ctx.sql.push(')');
                            $ctx.sql.push_str(" AS ");
                            crate::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
                        }
                    }
                }
            }
        }

        if let Some(window) = partition_window.as_ref() {
            $ctx.sql.push_str(", ROW_NUMBER() OVER (");
            let mut window_clause_prefix = "";
            if !window.partition_by.is_empty() {
                $ctx.sql.push_str("PARTITION BY ");
                for (i, col) in window.partition_by.iter().enumerate() {
                    if i > 0 {
                        $ctx.sql.push_str(", ");
                    }
                    crate::push_identifier_reference(&mut $ctx.sql, col, $quote);
                }
                window_clause_prefix = " ";
            }
            render_order_by_clause_mut!($ctx, $select, $quote, $render_expr, window_clause_prefix);
            $ctx.sql.push_str(") AS ");
            crate::push_quoted_identifier(&mut $ctx.sql, crate::WINDOW_ROW_NUMBER_ALIAS, $quote);
        }

        $ctx.sql.push_str(" FROM ");
        crate::push_table_name(&mut $ctx.sql, &$select.table, $quote);

        for join in $select.joins.iter_mut() {
            match join.join_type {
                nautilus_core::JoinType::Inner => $ctx.sql.push_str(" INNER JOIN "),
                nautilus_core::JoinType::Left => $ctx.sql.push_str(" LEFT JOIN "),
            }
            crate::push_table_name(&mut $ctx.sql, &join.table, $quote);
            $ctx.sql.push_str(" ON ");
            $render_expr($ctx, &mut join.on);
        }

        if let Some(filter) = $select.filter.as_mut() {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if !$select.group_by.is_empty() {
            $ctx.sql.push_str(" GROUP BY ");
            for (i, col) in $select.group_by.iter().enumerate() {
                if i > 0 {
                    $ctx.sql.push_str(", ");
                }
                crate::push_qualified_identifier(&mut $ctx.sql, &col.table, &col.name, $quote);
            }
        }

        if let Some(having) = $select.having.as_mut() {
            $ctx.sql.push_str(" HAVING ");
            $render_expr($ctx, having);
        }

        if let Some(window) = partition_window.as_ref() {
            $ctx.sql.push_str(") AS ");
            crate::push_quoted_identifier(&mut $ctx.sql, crate::WINDOW_SUBQUERY_ALIAS, $quote);

            let mut first_bound = true;
            if window.skip > 0 {
                $ctx.sql.push_str(" WHERE ");
                first_bound = false;
                crate::push_quoted_identifier(
                    &mut $ctx.sql,
                    crate::WINDOW_ROW_NUMBER_ALIAS,
                    $quote,
                );
                $ctx.sql.push_str(" > ");
                crate::push_u32(&mut $ctx.sql, window.skip);
            }
            if let Some(take) = window.take {
                if first_bound {
                    $ctx.sql.push_str(" WHERE ");
                } else {
                    $ctx.sql.push_str(" AND ");
                }
                crate::push_quoted_identifier(
                    &mut $ctx.sql,
                    crate::WINDOW_ROW_NUMBER_ALIAS,
                    $quote,
                );
                $ctx.sql.push_str(" <= ");
                crate::push_u64(&mut $ctx.sql, u64::from(window.skip) + u64::from(take));
            }

            $ctx.sql.push_str(" ORDER BY ");
            crate::push_quoted_identifier(&mut $ctx.sql, crate::WINDOW_ROW_NUMBER_ALIAS, $quote);
            $ctx.sql.push_str(" ASC");
        } else {
            render_order_by_clause_mut!($ctx, $select, $quote, $render_expr, " ");

            if let Some(take) = $select.take {
                $ctx.sql.push_str(" LIMIT ");
                crate::push_u32(&mut $ctx.sql, take.unsigned_abs());
            } else if $select.skip.is_some() && !$offset_limit_sentinel.is_empty() {
                // MySQL and SQLite both reject a bare OFFSET, so a provider
                // that needs one supplies the largest limit it accepts.
                $ctx.sql.push_str(" LIMIT ");
                $ctx.sql.push_str($offset_limit_sentinel);
            }

            if let Some(skip) = $select.skip {
                $ctx.sql.push_str(" OFFSET ");
                crate::push_u32(&mut $ctx.sql, skip);
            }
        }
    }};
}

/// Mutable/owned variant of [`render_expr_common!`] used by `render_*_owned`.
macro_rules! render_expr_common_mut {
    (
        $ctx:expr, $expr:expr,
        $quote:expr, $render_expr:ident, $render_select_body:ident,
        { $($specific:tt)* }
    ) => {
        match $expr {
            nautilus_core::Expr::Column(name) => {
                crate::push_identifier_reference(&mut $ctx.sql, name, $quote);
            }
            nautilus_core::Expr::Not(inner) => {
                $ctx.sql.push_str("NOT (");
                $render_expr($ctx, inner.as_mut());
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::Exists(subquery) => {
                $ctx.sql.push_str("EXISTS (");
                $render_select_body($ctx, subquery.as_mut());
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::NotExists(subquery) => {
                $ctx.sql.push_str("NOT EXISTS (");
                $render_select_body($ctx, subquery.as_mut());
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::Relation { op, relation } => {
                let is_exists = matches!(*op, nautilus_core::expr::RelationFilterOp::Some);
                if is_exists {
                    $ctx.sql.push_str("EXISTS (SELECT * FROM ");
                } else {
                    $ctx.sql.push_str("NOT EXISTS (SELECT * FROM ");
                }
                // An implicit many-to-many keeps its links in a table of its
                // own, so the subquery starts there and joins the children in;
                // every other relation reads the parent key off the child row.
                match relation.via.as_ref() {
                    Some(via) => {
                        crate::push_table_name(&mut $ctx.sql, &via.table, $quote);
                        $ctx.sql.push_str(" INNER JOIN ");
                        crate::push_table_name(&mut $ctx.sql, &relation.target_table, $quote);
                        $ctx.sql.push_str(" ON ");
                        crate::push_qualified_identifier(
                            &mut $ctx.sql,
                            relation.target_table.as_str(),
                            &relation.fk_db,
                            $quote,
                        );
                        $ctx.sql.push_str(" = ");
                        crate::push_qualified_identifier(
                            &mut $ctx.sql,
                            via.table.as_str(),
                            &via.child_column,
                            $quote,
                        );
                        $ctx.sql.push_str(" WHERE ");
                        crate::push_qualified_identifier(
                            &mut $ctx.sql,
                            via.table.as_str(),
                            &via.parent_column,
                            $quote,
                        );
                    }
                    None => {
                        crate::push_table_name(&mut $ctx.sql, &relation.target_table, $quote);
                        $ctx.sql.push_str(" WHERE ");
                        crate::push_qualified_identifier(
                            &mut $ctx.sql,
                            relation.target_table.as_str(),
                            &relation.fk_db,
                            $quote,
                        );
                    }
                }
                $ctx.sql.push_str(" = ");
                crate::push_qualified_identifier(
                    &mut $ctx.sql,
                    &relation.parent_table,
                    &relation.pk_db,
                    $quote,
                );
                $ctx.sql.push_str(" AND ");
                if matches!(*op, nautilus_core::expr::RelationFilterOp::Every) {
                    $ctx.sql.push_str("NOT (");
                    $render_expr($ctx, relation.filter.as_mut());
                    $ctx.sql.push(')');
                } else {
                    $render_expr($ctx, relation.filter.as_mut());
                }
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::ScalarSubquery(subquery) => {
                $ctx.sql.push('(');
                $render_select_body($ctx, subquery.as_mut());
                $ctx.sql.push(')');
            }
            nautilus_core::Expr::IsNull(inner) => {
                $ctx.sql.push('(');
                $render_expr($ctx, inner.as_mut());
                $ctx.sql.push_str(" IS NULL)");
            }
            nautilus_core::Expr::IsNotNull(inner) => {
                $ctx.sql.push('(');
                $render_expr($ctx, inner.as_mut());
                $ctx.sql.push_str(" IS NOT NULL)");
            }
            nautilus_core::Expr::Literal(s) => {
                crate::push_sql_string_literal(&mut $ctx.sql, s.as_str());
            }
            nautilus_core::Expr::List(exprs) => {
                for (i, e) in exprs.iter_mut().enumerate() {
                    if i > 0 {
                        $ctx.sql.push_str(", ");
                    }
                    $render_expr($ctx, e);
                }
            }
            nautilus_core::Expr::CaseWhen { condition, then } => {
                $ctx.sql.push_str("CASE WHEN ");
                $render_expr($ctx, condition.as_mut());
                $ctx.sql.push_str(" THEN ");
                $render_expr($ctx, then.as_mut());
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
mod render_estimate;
mod sqlite;

pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use sqlite::SqliteDialect;

use nautilus_core::{Delete, Insert, Result, Select, Update, Value};
pub(crate) use render_estimate::{
    estimate_delete_render, estimate_insert_render, estimate_select_render, estimate_update_render,
    RenderEstimate,
};

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

    /// Whether this dialect can restrict a result set to one row per
    /// distinct-column combination on its own (PostgreSQL's `DISTINCT ON`).
    ///
    /// A dialect that returns `false` renders `SELECT DISTINCT`, which
    /// deduplicates whole rows and therefore cannot honour `distinct` by
    /// itself; the engine deduplicates the decoded rows instead.
    fn supports_distinct_on(&self) -> bool {
        false
    }

    /// Render an owned SELECT query into SQL, moving bound values out of the AST
    /// instead of cloning them. This is the primary rendering entry point used by
    /// the engine's hot paths; dialects implement this.
    fn render_select_owned(&self, select: Select) -> Result<Sql>;

    /// Render a borrowed SELECT query into SQL.
    ///
    /// Clones the AST once and delegates to [`Self::render_select_owned`]. Used by
    /// previews, tests, and other non-hot paths that only hold a `&Select`.
    fn render_select(&self, select: &Select) -> Result<Sql> {
        self.render_select_owned(select.clone())
    }

    /// Render an owned INSERT query into SQL, moving bound values out of the AST
    /// instead of cloning them.
    fn render_insert_owned(&self, insert: Insert) -> Result<Sql>;

    /// Render a borrowed INSERT query into SQL by cloning and delegating to
    /// [`Self::render_insert_owned`].
    fn render_insert(&self, insert: &Insert) -> Result<Sql> {
        self.render_insert_owned(insert.clone())
    }

    /// Render an owned UPDATE query into SQL, moving bound values out of the AST
    /// instead of cloning them.
    fn render_update_owned(&self, update: Update) -> Result<Sql>;

    /// Render a borrowed UPDATE query into SQL by cloning and delegating to
    /// [`Self::render_update_owned`].
    fn render_update(&self, update: &Update) -> Result<Sql> {
        self.render_update_owned(update.clone())
    }

    /// Render an owned DELETE query into SQL, moving bound values out of the AST
    /// instead of cloning them.
    fn render_delete_owned(&self, delete: Delete) -> Result<Sql>;

    /// Render a borrowed DELETE query into SQL by cloning and delegating to
    /// [`Self::render_delete_owned`].
    fn render_delete(&self, delete: &Delete) -> Result<Sql> {
        self.render_delete_owned(delete.clone())
    }
}

/// Alias of the row-number column a [`nautilus_core::PartitionWindow`] adds to
/// the inner select. Never projected by the outer query, so callers see the same
/// columns they would without a window.
pub(crate) const WINDOW_ROW_NUMBER_ALIAS: &str = "__nautilus_rn";

/// Alias of the subquery a [`nautilus_core::PartitionWindow`] wraps the select in.
pub(crate) const WINDOW_SUBQUERY_ALIAS: &str = "__nautilus_win";

fn push_escaped_identifier(sql: &mut String, name: &str, quote: char) {
    for ch in name.chars() {
        if ch == quote {
            sql.push(quote);
        }
        sql.push(ch);
    }
}

/// Quote a SQL identifier directly into the SQL buffer.
/// The `$param_cast` hook for dialects that never cast a bound parameter.
///
/// PostgreSQL is the only dialect that needs one: it binds several values as
/// text (pgvector, PostGIS, JSON) and the server refuses to assign text to a
/// column of the real type without an explicit cast.
pub(crate) fn no_param_cast(_value: &nautilus_core::Value) -> Option<String> {
    None
}

pub(crate) fn push_quoted_identifier(sql: &mut String, name: &str, quote: char) {
    sql.push(quote);
    push_escaped_identifier(sql, name, quote);
    sql.push(quote);
}

/// Quote multiple identifier segments as a single identifier directly into the SQL buffer.
pub(crate) fn push_quoted_identifier_segments(sql: &mut String, segments: &[&str], quote: char) {
    sql.push(quote);
    for segment in segments {
        push_escaped_identifier(sql, segment, quote);
    }
    sql.push(quote);
}

/// Render `table.column` directly into the SQL buffer.
/// Render a table in the statement's table position, qualifying it with its
/// schema when it has one.
///
/// Column references keep using the bare table name: every supported provider
/// gives `schema.table` the bare `table` as its implicit alias.
pub(crate) fn push_table_name(sql: &mut String, table: &nautilus_core::TableName, quote: char) {
    if let Some(schema) = table.schema() {
        push_quoted_identifier(sql, schema, quote);
        sql.push('.');
    }
    push_quoted_identifier(sql, &table.name, quote);
}

pub(crate) fn push_qualified_identifier(sql: &mut String, table: &str, column: &str, quote: char) {
    push_quoted_identifier(sql, table, quote);
    sql.push('.');
    push_quoted_identifier(sql, column, quote);
}

/// Render a join-safe `table__column` alias directly into the SQL buffer.
pub(crate) fn push_column_alias(
    sql: &mut String,
    column: &nautilus_core::ColumnMarker,
    quote: char,
) {
    push_quoted_identifier_segments(
        sql,
        &[column.table.as_ref(), "__", column.name.as_ref()],
        quote,
    );
}

/// Render an identifier reference that may use the `table__column` shorthand.
///
/// The split happens only on the first `__`, so mapped column names like
/// `users__profile__slug` still render as `users.profile__slug`.
pub(crate) fn push_identifier_reference(sql: &mut String, name: &str, quote: char) {
    if let Some((table, column)) = name.split_once("__") {
        push_qualified_identifier(sql, table, column, quote);
    } else {
        push_quoted_identifier(sql, name, quote);
    }
}

/// Render a PostgreSQL native composite field reference.
pub(crate) fn push_composite_field_reference(
    sql: &mut String,
    table: &str,
    column: &str,
    field: &str,
    quote: char,
) {
    sql.push('(');
    push_qualified_identifier(sql, table, column, quote);
    sql.push(')');
    sql.push('.');
    push_quoted_identifier(sql, field, quote);
}

fn push_json_path_key(sql: &mut String, key: &str) {
    sql.push_str("$.\"");
    for ch in key.chars() {
        match ch {
            '"' | '\\' => {
                sql.push('\\');
                sql.push(ch);
            }
            other => sql.push(other),
        }
    }
    sql.push('"');
}

/// Render a single-quoted JSON object path literal for a schema-known key.
pub(crate) fn push_json_object_path_literal(sql: &mut String, key: &str) {
    let mut path = String::with_capacity(key.len() + 4);
    push_json_path_key(&mut path, key);
    push_sql_string_literal(sql, &path);
}

/// Render a single-quoted SQL string literal directly into the SQL buffer.
pub(crate) fn push_sql_string_literal(sql: &mut String, value: &str) {
    sql.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            sql.push('\'');
        }
        sql.push(ch);
    }
    sql.push('\'');
}

/// Append a `u64` value directly into the SQL buffer.
pub(crate) fn push_u64(sql: &mut String, mut value: u64) {
    let mut digits = [0_u8; 20];
    let mut idx = digits.len();

    loop {
        idx -= 1;
        digits[idx] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }

    for digit in &digits[idx..] {
        sql.push(char::from(*digit));
    }
}

/// Append a `u32` value directly into the SQL buffer.
pub(crate) fn push_u32(sql: &mut String, value: u32) {
    push_u64(sql, u64::from(value));
}

/// Append a `usize` value directly into the SQL buffer.
pub(crate) fn push_usize(sql: &mut String, value: usize) {
    push_u64(sql, value as u64);
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
        nautilus_core::BinaryOp::Like | nautilus_core::BinaryOp::LikeEscape => "LIKE",
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
