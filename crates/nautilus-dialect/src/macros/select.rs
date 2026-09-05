//! Macros for SELECT: the statement body, its ORDER BY clause, and the
//! projection of a partition-window subquery.

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
                            crate::ident::push_identifier_reference(
                                &mut $ctx.sql,
                                &order.column,
                                $quote,
                            );
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
                    crate::ident::push_identifier_reference(&mut $ctx.sql, &order.column, $quote);
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
                    crate::ident::push_column_alias(&mut $ctx.sql, col, $quote);
                }
                nautilus_core::SelectItem::Computed { alias, .. } => {
                    crate::ident::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
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
                        crate::ident::push_column_alias(&mut $ctx.sql, col, $quote);
                    }
                    nautilus_core::SelectItem::Computed { alias, .. } => {
                        crate::ident::push_quoted_identifier(&mut $ctx.sql, alias, $quote);
                    }
                }
            }
        }
        if first {
            $ctx.sql.push('*');
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
        let partition_window = $select.partition_window.take();

        if partition_window.is_some() {
            $ctx.sql.push_str("SELECT ");
            $crate::macros::select::render_window_projection_mut!($ctx, $select, $quote);
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
                    crate::ident::push_identifier_reference(&mut $ctx.sql, col, $quote);
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
                        crate::ident::push_qualified_identifier(
                            &mut $ctx.sql,
                            &col.table,
                            &col.name,
                            $quote,
                        );
                        $ctx.sql.push_str(" AS ");
                        crate::ident::push_column_alias(&mut $ctx.sql, col, $quote);
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
            for join in $select.joins.iter_mut() {
                for item in join.items.iter_mut() {
                    if !first {
                        $ctx.sql.push_str(", ");
                    }
                    first = false;
                    match item {
                        nautilus_core::SelectItem::Column(col) => {
                            crate::ident::push_qualified_identifier(
                                &mut $ctx.sql,
                                &col.table,
                                &col.name,
                                $quote,
                            );
                            $ctx.sql.push_str(" AS ");
                            crate::ident::push_column_alias(&mut $ctx.sql, col, $quote);
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
                    crate::ident::push_identifier_reference(&mut $ctx.sql, col, $quote);
                }
                window_clause_prefix = " ";
            }
            $crate::macros::select::render_order_by_clause_mut!(
                $ctx,
                $select,
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
        crate::ident::push_table_name(&mut $ctx.sql, &$select.table, $quote);

        for join in $select.joins.iter_mut() {
            match join.join_type {
                nautilus_core::JoinType::Inner => $ctx.sql.push_str(" INNER JOIN "),
                nautilus_core::JoinType::Left => $ctx.sql.push_str(" LEFT JOIN "),
            }
            crate::ident::push_table_name(&mut $ctx.sql, &join.table, $quote);
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
                crate::ident::push_qualified_identifier(
                    &mut $ctx.sql,
                    &col.table,
                    &col.name,
                    $quote,
                );
            }
        }

        if let Some(having) = $select.having.as_mut() {
            $ctx.sql.push_str(" HAVING ");
            $render_expr($ctx, having);
        }

        if let Some(window) = partition_window.as_ref() {
            $ctx.sql.push_str(") AS ");
            crate::ident::push_quoted_identifier(
                &mut $ctx.sql,
                crate::ident::WINDOW_SUBQUERY_ALIAS,
                $quote,
            );

            let mut first_bound = true;
            if window.skip > 0 {
                $ctx.sql.push_str(" WHERE ");
                first_bound = false;
                crate::ident::push_quoted_identifier(
                    &mut $ctx.sql,
                    crate::ident::WINDOW_ROW_NUMBER_ALIAS,
                    $quote,
                );
                $ctx.sql.push_str(" > ");
                crate::ident::push_u32(&mut $ctx.sql, window.skip);
            }
            if let Some(take) = window.take {
                if first_bound {
                    $ctx.sql.push_str(" WHERE ");
                } else {
                    $ctx.sql.push_str(" AND ");
                }
                crate::ident::push_quoted_identifier(
                    &mut $ctx.sql,
                    crate::ident::WINDOW_ROW_NUMBER_ALIAS,
                    $quote,
                );
                $ctx.sql.push_str(" <= ");
                crate::ident::push_u64(&mut $ctx.sql, u64::from(window.skip) + u64::from(take));
            }

            $ctx.sql.push_str(" ORDER BY ");
            crate::ident::push_quoted_identifier(
                &mut $ctx.sql,
                crate::ident::WINDOW_ROW_NUMBER_ALIAS,
                $quote,
            );
            $ctx.sql.push_str(" ASC");
        } else {
            $crate::macros::select::render_order_by_clause_mut!(
                $ctx,
                $select,
                $quote,
                $render_expr,
                " "
            );

            if let Some(take) = $select.take {
                $ctx.sql.push_str(" LIMIT ");
                crate::ident::push_u32(&mut $ctx.sql, take.unsigned_abs());
            } else if $select.skip.is_some() && !$offset_limit_sentinel.is_empty() {
                // MySQL and SQLite both reject a bare OFFSET, so a provider
                // that needs one supplies the largest limit it accepts.
                $ctx.sql.push_str(" LIMIT ");
                $ctx.sql.push_str($offset_limit_sentinel);
            }

            if let Some(skip) = $select.skip {
                $ctx.sql.push_str(" OFFSET ");
                crate::ident::push_u32(&mut $ctx.sql, skip);
            }
        }
    }};
}

pub(crate) use render_order_by_clause_mut;
pub(crate) use render_select_body_core_mut;
pub(crate) use render_window_projection_mut;
