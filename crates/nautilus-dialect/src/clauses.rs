//! Clause fragments every dialect writes the same way and that bind nothing.
//!
//! They write identifiers and keywords only, so they need the SQL buffer and
//! the dialect's quote character, not its render context or expression
//! renderer: that is what lets them be functions rather than macros.

use nautilus_core::{ColumnMarker, OrderBy, OrderDir, PartitionWindow, Select, SelectItem};

use crate::ident::{
    push_column_alias, push_identifier_reference, push_qualified_identifier,
    push_quoted_identifier, push_u32, push_u64, WINDOW_ROW_NUMBER_ALIAS, WINDOW_SUBQUERY_ALIAS,
};

/// Append `table.column AS table__column`, the projection of a plain column.
pub(crate) fn push_aliased_column(sql: &mut String, col: &ColumnMarker, quote: char) {
    push_qualified_identifier(sql, &col.table, &col.name, quote);
    sql.push_str(" AS ");
    push_column_alias(sql, col, quote);
}

/// Append the unqualified names of `columns`, comma-separated.
pub(crate) fn push_column_names(sql: &mut String, columns: &[ColumnMarker], quote: char) {
    for (i, col) in columns.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        push_quoted_identifier(sql, &col.name, quote);
    }
}

/// Append identifier references, comma-separated; each may use the
/// `table__column` shorthand.
pub(crate) fn push_identifier_references(sql: &mut String, names: &[String], quote: char) {
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        push_identifier_reference(sql, name, quote);
    }
}

/// Append `DISTINCT ON (...) ` when the dialect has it, or `DISTINCT `, when
/// the select deduplicates at all.
pub(crate) fn push_distinct(sql: &mut String, distinct: &[String], distinct_on: bool, quote: char) {
    if distinct.is_empty() {
        return;
    }
    if distinct_on {
        sql.push_str("DISTINCT ON (");
        push_identifier_references(sql, distinct, quote);
        sql.push_str(") ");
    } else {
        sql.push_str("DISTINCT ");
    }
}

/// Append ` GROUP BY table.column, ...` when `group_by` is non-empty.
pub(crate) fn push_group_by(sql: &mut String, group_by: &[ColumnMarker], quote: char) {
    if group_by.is_empty() {
        return;
    }
    sql.push_str(" GROUP BY ");
    for (i, col) in group_by.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        push_qualified_identifier(sql, &col.table, &col.name, quote);
    }
}

/// Append ` ASC` or ` DESC`.
pub(crate) fn push_direction(sql: &mut String, dir: OrderDir) {
    match dir {
        OrderDir::Asc => sql.push_str(" ASC"),
        OrderDir::Desc => sql.push_str(" DESC"),
    }
}

/// Append one column entry of an `ORDER BY` list.
pub(crate) fn push_column_order(sql: &mut String, order: &OrderBy, quote: char) {
    push_identifier_reference(sql, &order.column, quote);
    push_direction(sql, order.direction);
}

/// Append ` RETURNING table.column AS table__column, ...` when `returning` is
/// non-empty.
pub(crate) fn push_returning(sql: &mut String, returning: &[ColumnMarker], quote: char) {
    if returning.is_empty() {
        return;
    }
    sql.push_str(" RETURNING ");
    for (i, col) in returning.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        push_aliased_column(sql, col, quote);
    }
}

/// Append the statement's `LIMIT` and `OFFSET`.
///
/// A negative `take` asks for backward pagination, which the caller serves by
/// reversing the ordering, so only its magnitude reaches the SQL. MySQL and
/// SQLite reject a bare `OFFSET`, so a provider that needs one passes the
/// largest limit it accepts as `offset_limit_sentinel`; an empty sentinel
/// writes the `OFFSET` alone.
pub(crate) fn push_limit_offset(
    sql: &mut String,
    take: Option<i32>,
    skip: Option<u32>,
    offset_limit_sentinel: &str,
) {
    if let Some(take) = take {
        sql.push_str(" LIMIT ");
        push_u32(sql, take.unsigned_abs());
    } else if skip.is_some() && !offset_limit_sentinel.is_empty() {
        sql.push_str(" LIMIT ");
        sql.push_str(offset_limit_sentinel);
    }

    if let Some(skip) = skip {
        sql.push_str(" OFFSET ");
        push_u32(sql, skip);
    }
}

/// Render the outer projection of a partition-window subquery: the inner select
/// list referenced by alias, so the row-number column stays internal and the
/// result keeps exactly the columns an unwindowed render would return.
pub(crate) fn push_window_projection(sql: &mut String, select: &Select, quote: char) {
    let items = select
        .items
        .iter()
        .chain(select.joins.iter().flat_map(|join| join.items.iter()));
    let mut first = true;
    for item in items {
        if !first {
            sql.push_str(", ");
        }
        first = false;
        match item {
            SelectItem::Column(col) => push_column_alias(sql, col, quote),
            SelectItem::Computed { alias, .. } => push_quoted_identifier(sql, alias, quote),
        }
    }
    if first {
        sql.push('*');
    }
}

/// Close a partition-window subquery and keep the rows whose number falls in
/// `(skip, skip + take]`, in partition order.
pub(crate) fn push_window_bounds(sql: &mut String, window: &PartitionWindow, quote: char) {
    sql.push_str(") AS ");
    push_quoted_identifier(sql, WINDOW_SUBQUERY_ALIAS, quote);

    let mut first_bound = true;
    if window.skip > 0 {
        sql.push_str(" WHERE ");
        first_bound = false;
        push_quoted_identifier(sql, WINDOW_ROW_NUMBER_ALIAS, quote);
        sql.push_str(" > ");
        push_u32(sql, window.skip);
    }
    if let Some(take) = window.take {
        sql.push_str(if first_bound { " WHERE " } else { " AND " });
        push_quoted_identifier(sql, WINDOW_ROW_NUMBER_ALIAS, quote);
        sql.push_str(" <= ");
        push_u64(sql, u64::from(window.skip) + u64::from(take));
    }

    sql.push_str(" ORDER BY ");
    push_quoted_identifier(sql, WINDOW_ROW_NUMBER_ALIAS, quote);
    sql.push_str(" ASC");
}
