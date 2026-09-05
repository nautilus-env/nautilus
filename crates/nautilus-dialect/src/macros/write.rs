//! Macros for the statements that change rows: INSERT, UPDATE and DELETE, with
//! the RETURNING, assignment and ON CONFLICT clauses they share.

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
                crate::ident::push_qualified_identifier(
                    &mut $ctx.sql,
                    &col.table,
                    &col.name,
                    $quote,
                );
                $ctx.sql.push_str(" AS ");
                crate::ident::push_column_alias(&mut $ctx.sql, col, $quote);
            }
        }
    }};
}

/// Render the body of an INSERT, including its conflict clause and RETURNING.
macro_rules! render_insert_body_mut {
    ($ctx:expr, $insert:expr, $quote:expr, $supports_returning:expr, $param_cast:expr, $conflict_clause:ident) => {{
        $ctx.sql.push_str("INSERT INTO ");
        crate::ident::push_table_name(&mut $ctx.sql, &$insert.table, $quote);

        $ctx.sql.push_str(" (");
        for (i, col) in $insert.columns.iter().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            crate::ident::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
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
            $crate::macros::write::render_returning_mut!($ctx, $insert.returning, $quote);
        }
    }};
}

/// Render the right-hand side of one `SET` entry.
///
/// A bound value keeps the NULL-literal and cast handling every dialect needs;
/// an expression goes through the dialect's own expression renderer, which is
/// what lets `views = (views + $1)` reference the row being updated.
macro_rules! render_assignment_mut {
    ($ctx:expr, $assignment:expr, $render_expr:ident, $param_cast:expr) => {{
        match $assignment {
            nautilus_core::Assignment::Value(value) => {
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
            nautilus_core::Assignment::Expr(expr) => $render_expr($ctx, expr),
        }
    }};
}

/// Append the SQL-standard `ON CONFLICT (...) DO UPDATE SET ... ` / `DO NOTHING`
/// clause shared by PostgreSQL and SQLite.
///
/// MySQL spells the same idea `ON DUPLICATE KEY UPDATE` and has no conflict
/// target, so it renders its own clause instead of calling this.
macro_rules! render_on_conflict_body_mut {
    ($ctx:expr, $on_conflict:expr, $quote:expr, $render_expr:ident, $param_cast:expr) => {{
        $ctx.sql.push_str(" ON CONFLICT (");
        for (i, col) in $on_conflict.target.iter().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            crate::ident::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
        }
        $ctx.sql.push(')');

        if $on_conflict.update.is_empty() {
            $ctx.sql.push_str(" DO NOTHING");
        } else {
            $ctx.sql.push_str(" DO UPDATE SET ");
            for (i, (col, assignment)) in $on_conflict.update.iter_mut().enumerate() {
                if i > 0 {
                    $ctx.sql.push_str(", ");
                }
                crate::ident::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
                $ctx.sql.push_str(" = ");
                $crate::macros::write::render_assignment_mut!(
                    $ctx,
                    assignment,
                    $render_expr,
                    $param_cast
                );
            }
        }
    }};
}

/// Render the body of an UPDATE: its assignments, filter and RETURNING.
macro_rules! render_update_body_mut {
    ($ctx:expr, $update:expr, $quote:expr, $render_expr:ident, $supports_returning:expr, $param_cast:expr) => {{
        $ctx.sql.push_str("UPDATE ");
        crate::ident::push_table_name(&mut $ctx.sql, &$update.table, $quote);

        $ctx.sql.push_str(" SET ");
        for (i, (col, assignment)) in $update.assignments.iter_mut().enumerate() {
            if i > 0 {
                $ctx.sql.push_str(", ");
            }
            crate::ident::push_quoted_identifier(&mut $ctx.sql, &col.name, $quote);
            $ctx.sql.push_str(" = ");
            $crate::macros::write::render_assignment_mut!(
                $ctx,
                assignment,
                $render_expr,
                $param_cast
            );
        }

        if let Some(filter) = $update.filter.as_mut() {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if $supports_returning {
            $crate::macros::write::render_returning_mut!($ctx, $update.returning, $quote);
        }
    }};
}

/// Render the body of a DELETE: its filter and RETURNING.
macro_rules! render_delete_body_mut {
    ($ctx:expr, $delete:expr, $quote:expr, $render_expr:ident, $supports_returning:expr) => {{
        $ctx.sql.push_str("DELETE FROM ");
        crate::ident::push_table_name(&mut $ctx.sql, &$delete.table, $quote);

        if let Some(filter) = $delete.filter.as_mut() {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        if $supports_returning {
            $crate::macros::write::render_returning_mut!($ctx, $delete.returning, $quote);
        }
    }};
}

pub(crate) use render_assignment_mut;
pub(crate) use render_delete_body_mut;
pub(crate) use render_insert_body_mut;
pub(crate) use render_on_conflict_body_mut;
pub(crate) use render_returning_mut;
pub(crate) use render_update_body_mut;
