#![forbid(unsafe_code)]
//! Database executors and connection management for Nautilus ORM.
//!
//! [`Executor`] is the interface the rest of the workspace runs SQL through;
//! `sqlx` pools back the three implementations, one per supported provider.
//! A [`Row`] reads by position or by name.
//!
//! ## Example
//!
//! ```no_run
//! use futures::StreamExt;
//! use nautilus_connector::{execute_all, ConnectorResult, Executor, PgExecutor};
//! use nautilus_core::{ColumnMarker, RowAccess, Select};
//! use nautilus_dialect::{Dialect, PostgresDialect};
//!
//! # async fn example() -> ConnectorResult<()> {
//! let executor = PgExecutor::new("postgres://localhost/mydb").await?;
//! let select = Select::from_table("users")
//!     .item(ColumnMarker::new("users", "id").into())
//!     .item(ColumnMarker::new("users", "name").into())
//!     .build()?;
//! let sql = PostgresDialect.render_select(&select)?;
//!
//! let mut stream = executor.execute(&sql);
//! while let Some(row) = stream.next().await {
//!     println!("{:?}", row?.get("name"));
//! }
//!
//! for row in execute_all(&executor, &sql).await? {
//!     println!("{:?}", row.get("id"));
//! }
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

/// Generate `execute_affected` for a pool-backed executor.
///
/// Each backend module defines its own `bind_value` function and exposes a
/// `self.pool` field.  The method body is identical across all three backends
/// (Postgres, MySQL, SQLite), so this macro produces it from a single source.
macro_rules! impl_execute_affected {
    () => {
        /// Execute a mutation SQL and return the number of affected rows.
        ///
        /// Used when `return_data = false` — no `RETURNING` clause is emitted
        /// so the affected-row count must come from the database result.
        pub async fn execute_affected(
            &self,
            sql: &nautilus_dialect::Sql,
        ) -> $crate::error::Result<usize> {
            let mut query = sqlx::query(&sql.text);
            for param in &sql.params {
                query = bind_value(query, param)?;
            }
            let result = query
                .execute(&self.pool)
                .await
                .map_err(|e| $crate::error::ConnectorError::database(e, "Mutation failed"))?;
            Ok(result.rows_affected() as usize)
        }
    };
}

mod client;
pub mod error;
mod executor;
mod from_row;
mod mysql;
mod mysql_stream;
mod pool_options;
mod postgres;
mod row;
mod row_stream;
mod single_row;
mod sqlite;
mod sqlite_stream;
mod streaming;
pub mod transaction;
mod utils;
mod value_hint;

pub use client::Client;
pub use error::{ConnectorError, Result as ConnectorResult, SqlxErrorKind};
pub use executor::{execute_all, execute_one, execute_optional, Executor};
pub use from_row::FromRow;
pub use mysql::MysqlExecutor;
pub use mysql_stream::MysqlRowStream;
pub use pool_options::ConnectorPoolOptions;
pub use postgres::{PgExecutor, PgRowStream};
pub use row::Row;
pub use row_stream::RowStream;
pub use sqlite::SqliteExecutor;
pub use sqlite_stream::SqliteRowStream;
pub use transaction::{IsolationLevel, TransactionExecutor, TransactionOptions};
pub use value_hint::{
    decode_row_with_hints, hint_name, normalize_row_with_hints, normalize_rows_with_hints,
    normalize_scalar, normalize_value_with_hint, HintMismatch, ValueHint,
};

pub use nautilus_core::RowAccess;

pub use nautilus_core::Column;
pub use nautilus_core::FromValue;

/// Internal decode entry points re-exported for the criterion benches only.
/// Not part of the public connector API.
#[doc(hidden)]
pub mod bench {
    /// Batch-decode sqlite rows through the same path the executors use.
    pub fn decode_sqlite_rows(
        rows: &[sqlx::sqlite::SqliteRow],
    ) -> crate::error::Result<Vec<crate::Row>> {
        crate::sqlite_stream::decode_rows(rows)
    }
}
