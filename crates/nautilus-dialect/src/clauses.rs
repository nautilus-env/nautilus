//! Clause fragments every dialect writes the same way and that bind nothing.
//!
//! They write identifiers and keywords only, so they need the SQL buffer and
//! the dialect's quote character, not its render context or expression
//! renderer: that is what lets them be functions rather than macros.

use nautilus_core::{ColumnMarker, Select, SelectItem};

use crate::ident::{push_column_alias, push_qualified_identifier, push_quoted_identifier};

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
        push_qualified_identifier(sql, &col.table, &col.name, quote);
        sql.push_str(" AS ");
        push_column_alias(sql, col, quote);
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
