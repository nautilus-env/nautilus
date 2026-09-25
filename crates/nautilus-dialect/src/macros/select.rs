//! Macros for SELECT: the statement body and its ORDER BY clause.

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
                            crate::clauses::push_column_order(&mut $ctx.sql, order, $quote);
                        }
                        nautilus_core::OrderByItem::Expr(expr, dir) => {
                            $render_expr($ctx, expr);
                            crate::clauses::push_direction(&mut $ctx.sql, *dir);
                        }
                    }
                }
            } else {
                for order in $select.order_by.iter() {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    crate::clauses::push_column_order(&mut $ctx.sql, order, $quote);
                }
                for (expr, dir) in $select.order_by_exprs.iter_mut() {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    $render_expr($ctx, expr);
                    crate::clauses::push_direction(&mut $ctx.sql, *dir);
                }
            }
        }
    }};
}

/// Render the body of a SELECT: projection, source, joins, filters, grouping,
/// ordering and paging, including the partition-window rewrite.
macro_rules! render_select_body_core_mut {
    (
        $ctx:expr, $select:expr,
        $quote:expr, $render_expr:ident,
        $distinct_on:expr, $offset_limit_sentinel:expr
    ) => {{
        let select: &mut nautilus_core::Select = $select;
        let partition_window = select.partition_window.take();

        if partition_window.is_some() {
            $ctx.sql.push_str("SELECT ");
            crate::clauses::push_window_projection(&mut $ctx.sql, select, $quote);
            $ctx.sql.push_str(" FROM (");
        }

        $ctx.sql.push_str("SELECT ");
        crate::clauses::push_distinct(&mut $ctx.sql, &select.distinct, $distinct_on, $quote);

        let items = select.items.iter_mut().chain(
            select
                .joins
                .iter_mut()
                .flat_map(|join| join.items.iter_mut()),
        );
        let mut first = true;
        for item in items {
            if !first {
                $ctx.sql.push_str(", ");
            }
            first = false;
            match item {
                nautilus_core::SelectItem::Column(col) => {
                    crate::clauses::push_aliased_column(&mut $ctx.sql, col, $quote);
                }
                nautilus_core::SelectItem::Computed { expr, alias } => {
                    $ctx.sql.push('(');
                    $render_expr($ctx, expr);
                    $ctx.sql.push(')');
                    $ctx.sql.push_str(" AS ");
                    crate::ident::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
                }
            }
        }
        if first {
            $ctx.sql.push('*');
        }

        if let Some(window) = partition_window.as_ref() {
            $ctx.sql.push_str(", ROW_NUMBER() OVER (");
            let mut window_clause_prefix = "";
            if !window.partition_by.is_empty() {
                $ctx.sql.push_str("PARTITION BY ");
                crate::clauses::push_identifier_references(
                    &mut $ctx.sql,
                    &window.partition_by,
                    $quote,
                );
                window_clause_prefix = " ";
            }
            $crate::macros::select::render_order_by_clause_mut!(
                $ctx,
                select,
                $quote,
                $render_expr,
                window_clause_prefix
            );
            $ctx.sql.push_str(") AS ");
            crate::ident::push_quoted_identifier(
                &mut $ctx.sql,
                crate::ident::WINDOW_ROW_NUMBER_ALIAS,
                $quote,
            );
        }

        $ctx.sql.push_str(" FROM ");
        crate::ident::push_table_name(&mut $ctx.sql, &select.table, $quote);

        for join in select.joins.iter_mut() {
            match join.join_type {
                nautilus_core::JoinType::Inner => $ctx.sql.push_str(" INNER JOIN "),
                nautilus_core::JoinType::Left => $ctx.sql.push_str(" LEFT JOIN "),
            }
            crate::ident::push_table_name(&mut $ctx.sql, &join.table, $quote);
            $ctx.sql.push_str(" ON ");
            $render_expr($ctx, &mut join.on);
        }

        if let Some(filter) = select.filter.as_mut() {
            $ctx.sql.push_str(" WHERE ");
            $render_expr($ctx, filter);
        }

        crate::clauses::push_group_by(&mut $ctx.sql, &select.group_by, $quote);

        if let Some(having) = select.having.as_mut() {
            $ctx.sql.push_str(" HAVING ");
            $render_expr($ctx, having);
        }

        if let Some(window) = partition_window.as_ref() {
            crate::clauses::push_window_bounds(&mut $ctx.sql, window, $quote);
        } else {
            $crate::macros::select::render_order_by_clause_mut!(
                $ctx,
                select,
                $quote,
                $render_expr,
                " "
            );
            crate::clauses::push_limit_offset(
                &mut $ctx.sql,
                select.take,
                select.skip,
                $offset_limit_sentinel,
            );
        }
    }};
}

pub(crate) use render_order_by_clause_mut;
pub(crate) use render_select_body_core_mut;
