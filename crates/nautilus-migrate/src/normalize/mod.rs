//! How a database and a schema spell the same thing.
//!
//! Three forms meet here and are deliberately kept apart:
//!
//! - **Provider SQL** is what introspection reads back. Every database rewrites
//!   what it was given — quoting identifiers its own way, adding casts, wrapping
//!   literals in parentheses, spelling a type differently — and the `*_pg_*`,
//!   `*_mysql_*` and `*_sqlite_*` rules turn each dialect's answer into the one
//!   form [`crate::live::LiveSchema`] carries.
//! - **Schema expression** is what a `.nautilus` file says, and what a live
//!   schema must be reducible to for `db pull` to produce a file that pushes
//!   back unchanged.
//! - **Comparison form** is what the diff reduces both sides to before asking
//!   whether they differ. It is lossier than the other two and is never written
//!   anywhere: `DEFAULT 'Draft'` and `DEFAULT 'draft'` compare equal, but
//!   neither is rewritten into the other.
//!
//! [`sql_text`] holds the rewrites the first and third layer genuinely share.
//! Rules that merely look alike are not merged: `strip_mysql_backtick_quotes`
//! and `strip_identifier_quotes` both remove quoting, but the first only knows
//! backticks and the second must leave the contents of a string literal alone,
//! so each keeps its own.

pub(crate) mod defaults;
pub(crate) mod predicates;
pub(crate) mod sql_text;
pub(crate) mod types;
