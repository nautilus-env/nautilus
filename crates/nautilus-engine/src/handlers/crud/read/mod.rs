//! Read operations: `findMany`, `findFirst`, `findUnique`, `count` and
//! `explain`.
//!
//! Planning is separate from consumption. [`plan`] turns parsed
//! [`QueryArgs`](crate::filter::QueryArgs) into a `FindManyPlan` — rendered
//! SQL, decoding hints and whatever the dialect could not express — and
//! [`ordering`] resolves the `orderBy` targets that go into it. The entry
//! points then pick how to consume that plan: buffered rows plus include
//! hydration in [`find_many`], row-by-row chunks in [`stream`], the database's
//! own `EXPLAIN` in [`explain`]. [`count`] and [`find_unique`] render their
//! own narrower statements, and [`find_unique`] also owns the row lookups the
//! write paths use for read-back and target resolution.

mod count;
mod explain;
mod find_many;
mod find_unique;
mod ordering;
mod plan;
mod stream;

pub(in crate::handlers) use count::{handle_count, handle_count_embedded, handle_count_typed};
pub(in crate::handlers) use explain::{execute_explain_typed, handle_explain};
pub(in crate::handlers) use find_many::{
    execute_find_many_typed, handle_find_first, handle_find_first_or_throw, handle_find_many,
    handle_find_many_embedded,
};
pub(in crate::handlers) use find_unique::{
    execute_find_unique_typed, handle_find_unique, handle_find_unique_or_throw,
};

pub(in crate::handlers::crud) use find_many::execute_find_many_rows;
pub(in crate::handlers::crud) use find_unique::{
    build_find_unique_sql, find_all_rows_by_filter, find_one_row, find_rows_by_expr,
    find_rows_by_filter,
};
