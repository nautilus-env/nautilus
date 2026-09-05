//! The macro for the expression forms every dialect renders the same way.

/// Render the expression forms every dialect spells the same way, leaving
/// `$specific` to add the ones a dialect renders its own way.
macro_rules! render_expr_common_mut {
    (
        $ctx:expr, $expr:expr,
        $quote:expr, $render_expr:ident, $render_select_body:ident,
        { $($specific:tt)* }
    ) => {
        match $expr {
            nautilus_core::Expr::Column(name) => {
                crate::ident::push_identifier_reference(&mut $ctx.sql, name, $quote);
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
                        crate::ident::push_table_name(&mut $ctx.sql, &via.table, $quote);
                        $ctx.sql.push_str(" INNER JOIN ");
                        crate::ident::push_table_name(&mut $ctx.sql, &relation.target_table, $quote);
                        $ctx.sql.push_str(" ON ");
                        crate::ident::push_qualified_identifier(
                            &mut $ctx.sql,
                            relation.target_table.as_str(),
                            &relation.fk_db,
                            $quote,
                        );
                        $ctx.sql.push_str(" = ");
                        crate::ident::push_qualified_identifier(
                            &mut $ctx.sql,
                            via.table.as_str(),
                            &via.child_column,
                            $quote,
                        );
                        $ctx.sql.push_str(" WHERE ");
                        crate::ident::push_qualified_identifier(
                            &mut $ctx.sql,
                            via.table.as_str(),
                            &via.parent_column,
                            $quote,
                        );
                    }
                    None => {
                        crate::ident::push_table_name(&mut $ctx.sql, &relation.target_table, $quote);
                        $ctx.sql.push_str(" WHERE ");
                        crate::ident::push_qualified_identifier(
                            &mut $ctx.sql,
                            relation.target_table.as_str(),
                            &relation.fk_db,
                            $quote,
                        );
                    }
                }
                $ctx.sql.push_str(" = ");
                crate::ident::push_qualified_identifier(
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
                crate::ident::push_sql_string_literal(&mut $ctx.sql, s.as_str());
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

pub(crate) use render_expr_common_mut;
