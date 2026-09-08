//! Transaction executor for Nautilus.
//!
//! This module provides [`TransactionExecutor`], a single type that wraps a
//! live database transaction for any of the three supported backends
//! (PostgreSQL, MySQL, SQLite).  It replaces the previous per-backend trio
//! `TxPgExecutor` / `TxMysqlExecutor` / `TxSqliteExecutor`, which had
//! identical structure in three copies.
//!
//! ## Architecture note
//!
//! sqlx's `Transaction<'static, Db>` is parameterised by `Db`, making a true
//! Rust generic impossible without fighting GAT lifetime constraints (SQLite's
//! `SqliteArguments<'q>` carries a `'q` lifetime that PG/MySQL arguments do
//! not).  The type instead holds a private enum of whichever backend's
//! transaction is live — `handle` holds that storage and its lifecycle,
//! `execute` runs a statement on it — while presenting a uniform public API to
//! all callers.

mod execute;
mod handle;
mod mysql;

use std::time::Duration;

use futures::future::BoxFuture;

use nautilus_dialect::Sql;

use crate::error::{ConnectorError as Error, Result};
use crate::row_stream::RowStream;
use crate::single_row::SingleRowExpectation;
use crate::{Executor, Row};
use execute::{tx_affected, tx_and_fetch, tx_collect, tx_single};
use handle::{handle, TransactionInner};
use mysql::MysqlTransaction;

/// Options for starting a transaction.
#[derive(Debug, Clone)]
pub struct TransactionOptions {
    /// Maximum duration before the transaction is automatically rolled back.
    pub timeout: Duration,
    /// Optional isolation level override.
    pub isolation_level: Option<IsolationLevel>,
}

impl Default for TransactionOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            isolation_level: None,
        }
    }
}

/// Transaction isolation level.
///
/// Independent of the wire protocol; adapters convert levels at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Read uncommitted — allows dirty reads.
    ReadUncommitted,
    /// Read committed — default for most databases.
    ReadCommitted,
    /// Repeatable read — prevents non-repeatable reads.
    RepeatableRead,
    /// Serializable — strictest isolation level.
    Serializable,
}

impl IsolationLevel {
    /// Returns the SQL representation (e.g., `"READ COMMITTED"`).
    pub fn as_sql(&self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    }
}

/// An executor that runs queries inside a live database transaction.
///
/// This single type works with PostgreSQL, MySQL, and SQLite, replacing the
/// previous per-backend `TxPgExecutor` / `TxMysqlExecutor` / `TxSqliteExecutor`
/// trio.  Callers see one consistent API regardless of the backend in use.
///
/// The underlying transaction is stored behind an
/// `Arc<Mutex<Option<…>>>` so the executor can be shared cheaply through
/// [`crate::client::Client`]'s `Arc<E>` wrapping.
///
/// # Example
///
/// ```no_run
/// # use nautilus_connector::{Client, ConnectorResult};
/// # async fn example() -> ConnectorResult<()> {
/// let client = Client::postgres("postgres://localhost/mydb").await?;
/// let result = client.transaction(Default::default(), |tx| Box::pin(async move {
///     // tx is Client<TransactionExecutor>; all queries run inside the transaction.
///     Ok(42i64)
/// })).await?;
/// # Ok(())
/// # }
/// ```
pub struct TransactionExecutor {
    inner: TransactionInner,
}

impl TransactionExecutor {
    /// Wrap an already-begun PostgreSQL transaction.
    pub fn postgres(tx: sqlx::Transaction<'static, sqlx::Postgres>) -> Self {
        Self {
            inner: TransactionInner::Postgres(handle(tx)),
        }
    }

    /// Begin a PostgreSQL transaction and apply an isolation override to it.
    ///
    /// PostgreSQL takes `SET TRANSACTION` inside the transaction it applies to,
    /// so the statement is issued after `BEGIN`, on the same connection.
    ///
    /// # Errors
    ///
    /// Returns a connection error if `BEGIN` fails, or a database error if the
    /// server rejects the isolation statement.
    pub async fn begin_postgres(
        pool: &sqlx::PgPool,
        isolation_level: Option<IsolationLevel>,
    ) -> Result<Self> {
        let tx = pool
            .begin()
            .await
            .map_err(|e| Error::connection(e, "Failed to begin transaction"))?;
        let executor = Self::postgres(tx);
        executor.set_isolation(isolation_level).await?;
        Ok(executor)
    }

    /// Wrap an already-begun MySQL transaction.
    pub fn mysql(tx: sqlx::Transaction<'static, sqlx::MySql>) -> Self {
        Self {
            inner: TransactionInner::Mysql(handle(MysqlTransaction::Sqlx(tx))),
        }
    }

    /// Begin a MySQL transaction with an optional override for this transaction only.
    ///
    /// Isolation is set before BEGIN on the same connection. Errors or cancellation
    /// during preparation discard that connection; completed transactions reuse it.
    /// With no override, MySQL uses the session's default isolation level.
    ///
    /// # Errors
    ///
    /// Returns a connection error if acquisition or `BEGIN` fails, or a database
    /// error if MySQL rejects the isolation statement.
    pub async fn begin_mysql(
        pool: &sqlx::MySqlPool,
        isolation_level: Option<IsolationLevel>,
    ) -> Result<Self> {
        let tx = MysqlTransaction::begin(pool, isolation_level).await?;
        Ok(Self {
            inner: TransactionInner::Mysql(handle(tx)),
        })
    }

    /// Wrap an already-begun SQLite transaction.
    pub fn sqlite(tx: sqlx::Transaction<'static, sqlx::Sqlite>) -> Self {
        Self {
            inner: TransactionInner::Sqlite(handle(tx)),
        }
    }

    /// Begin a SQLite transaction.
    ///
    /// SQLite has no `SET TRANSACTION ISOLATION LEVEL`, so there is no
    /// isolation override to apply here.
    ///
    /// # Errors
    ///
    /// Returns a connection error if `BEGIN` fails.
    pub async fn begin_sqlite(pool: &sqlx::SqlitePool) -> Result<Self> {
        let tx = pool
            .begin()
            .await
            .map_err(|e| Error::connection(e, "Failed to begin transaction"))?;
        Ok(Self::sqlite(tx))
    }

    /// Apply an isolation override to the transaction that is already open.
    async fn set_isolation(&self, isolation_level: Option<IsolationLevel>) -> Result<()> {
        let Some(isolation_level) = isolation_level else {
            return Ok(());
        };

        let sql = Sql {
            text: format!(
                "SET TRANSACTION ISOLATION LEVEL {}",
                isolation_level.as_sql()
            ),
            params: vec![],
        };
        crate::execute_all(self, &sql).await?;
        Ok(())
    }

    /// Commit the transaction. After this, further queries will return an error.
    pub async fn commit(&self) -> Result<()> {
        self.inner.commit().await
    }

    /// Rollback the transaction. After this, further queries will return an error.
    pub async fn rollback(&self) -> Result<()> {
        self.inner.rollback().await
    }

    /// Returns `true` if the transaction has not yet been committed or rolled back.
    pub async fn is_open(&self) -> bool {
        self.inner.is_open().await
    }

    /// Execute a mutation SQL inside this transaction and return the number of
    /// affected rows.
    ///
    /// Used when `return_data = false` so no RETURNING clause is emitted and
    /// the affected-row count comes from the database execution result.
    pub async fn execute_affected(&self, sql: &Sql) -> Result<usize> {
        tx_affected!(&self.inner, sql.text.clone(), sql.params.clone()).await
    }

    /// Execute a SQL query with sqlx statement persistence disabled.
    ///
    /// Raw/direct query paths use this so they remain compatible with
    /// poolers such as PgBouncer even when they run inside a transaction.
    pub async fn execute_collect_unprepared(&self, sql: &Sql) -> Result<Vec<Row>> {
        tx_collect!(
            &self.inner,
            sql.text.clone(),
            sql.params.clone(),
            false,
            "Query failed"
        )
        .await
    }
}

impl Executor for TransactionExecutor {
    type Row<'conn>
        = Row
    where
        Self: 'conn;
    type RowStream<'conn>
        = RowStream<'conn>
    where
        Self: 'conn;

    fn execute<'conn>(&'conn self, sql: &'conn Sql) -> Self::RowStream<'conn> {
        RowStream::from_rows_future(tx_collect!(
            &self.inner,
            sql.text.clone(),
            sql.params.clone(),
            true,
            "Query failed"
        ))
    }

    /// Streaming inside a transaction is buffered: a live transaction holds the
    /// connection exclusively, so the worker pattern used for pooled executors
    /// does not apply. We reuse the collecting runner (which already returns a
    /// `'static` future via the `Arc<Mutex<...>>` handle) and adapt it to the
    /// shared `RowStream<'static>` shape so codegen `stream_many` paths work
    /// uniformly across pooled and transactional clients.
    fn execute_owned(&self, sql: Sql) -> RowStream<'static> {
        RowStream::from_rows_future(tx_collect!(
            &self.inner,
            sql.text,
            sql.params,
            true,
            "Query failed"
        ))
    }

    fn execute_and_fetch<'conn>(
        &'conn self,
        mutation: &'conn Sql,
        fetch: &'conn Sql,
    ) -> Self::RowStream<'conn> {
        RowStream::from_rows_future(tx_and_fetch!(&self.inner, mutation, fetch))
    }

    fn execute_collect<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Vec<Self::Row<'conn>>>>
    where
        Self: 'conn,
    {
        tx_collect!(
            &self.inner,
            sql.text.clone(),
            sql.params.clone(),
            true,
            "Query failed"
        )
    }

    fn execute_one<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Self::Row<'conn>>>
    where
        Self: 'conn,
    {
        Box::pin(async move {
            let row = tx_single!(
                &self.inner,
                sql.text.clone(),
                sql.params.clone(),
                "Query failed",
                SingleRowExpectation::ExactlyOne
            )
            .await?;

            // `ExactlyOne` already validated row_count == 1, so `row` is always
            // `Some` here; the fallback keeps this a graceful error, never a panic.
            row.ok_or_else(|| Error::database_msg("Expected exactly one row, got 0"))
        })
    }

    fn execute_optional<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Option<Self::Row<'conn>>>>
    where
        Self: 'conn,
    {
        tx_single!(
            &self.inner,
            sql.text.clone(),
            sql.params.clone(),
            "Query failed",
            SingleRowExpectation::ZeroOrOne
        )
    }
}
