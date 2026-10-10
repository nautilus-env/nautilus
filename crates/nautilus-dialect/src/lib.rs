//! SQL dialect renderers for Nautilus ORM.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![warn(unreachable_pub)]

mod clauses;
mod dialect;
mod expr;
mod ident;
mod macros;
mod mysql;
mod output_aliases;
mod postgres;
mod render_estimate;
mod sqlite;

pub use dialect::{Dialect, Sql, SqlOutput};
pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use sqlite::SqliteDialect;
