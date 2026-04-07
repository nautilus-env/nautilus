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
//! not).  The type instead uses a private `TransactionInner` enum to hold
//! whichever backend's transaction is live, while presenting a uniform public
//! API to all callers.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use nautilus_dialect::Sql;

use crate::error::{ConnectorError as Error, Result};
use crate::row_stream::RowStream;
use crate::{Executor, Row};

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
/// Re-exported from `nautilus-protocol` for convenience; the connector uses
/// the same enum so callers don't need to depend on the protocol crate.
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

/// Per-backend transaction storage.
///
/// This is a private implementation detail — callers always interact with the
/// outer [`TransactionExecutor`] type.
enum TransactionInner {
    Postgres(Arc<Mutex<Option<sqlx::Transaction<'static, sqlx::Postgres>>>>),
    Mysql(Arc<Mutex<Option<sqlx::Transaction<'static, sqlx::MySql>>>>),
    Sqlite(Arc<Mutex<Option<sqlx::Transaction<'static, sqlx::Sqlite>>>>),
}

/// An executor that runs queries inside a live database transaction.
///
/// This single type works with PostgreSQL, MySQL, and SQLite, replacing the
/// previous per-backend `TxPgExecutor` / `TxMysqlExecutor` / `TxSqliteExecutor`
/// trio.  Internally it holds a [`TransactionInner`] enum; callers see one
/// consistent API regardless of the backend in use.
///
/// The underlying sqlx transaction is stored behind an
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
            inner: TransactionInner::Postgres(Arc::new(Mutex::new(Some(tx)))),
        }
    }

    /// Wrap an already-begun MySQL transaction.
    pub fn mysql(tx: sqlx::Transaction<'static, sqlx::MySql>) -> Self {
        Self {
            inner: TransactionInner::Mysql(Arc::new(Mutex::new(Some(tx)))),
        }
    }

    /// Wrap an already-begun SQLite transaction.
    pub fn sqlite(tx: sqlx::Transaction<'static, sqlx::Sqlite>) -> Self {
        Self {
            inner: TransactionInner::Sqlite(Arc::new(Mutex::new(Some(tx)))),
        }
    }

    /// Commit the transaction. After this, further queries will return an error.
    pub async fn commit(&self) -> Result<()> {
        match &self.inner {
            TransactionInner::Postgres(mx) => {
                let tx = mx
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                tx.commit()
                    .await
                    .map_err(|e| Error::database(e, "Commit failed"))
            }
            TransactionInner::Mysql(mx) => {
                let tx = mx
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                tx.commit()
                    .await
                    .map_err(|e| Error::database(e, "Commit failed"))
            }
            TransactionInner::Sqlite(mx) => {
                let tx = mx
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                tx.commit()
                    .await
                    .map_err(|e| Error::database(e, "Commit failed"))
            }
        }
    }

    /// Rollback the transaction. After this, further queries will return an error.
    pub async fn rollback(&self) -> Result<()> {
        match &self.inner {
            TransactionInner::Postgres(mx) => {
                let tx = mx
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                tx.rollback()
                    .await
                    .map_err(|e| Error::database(e, "Rollback failed"))
            }
            TransactionInner::Mysql(mx) => {
                let tx = mx
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                tx.rollback()
                    .await
                    .map_err(|e| Error::database(e, "Rollback failed"))
            }
            TransactionInner::Sqlite(mx) => {
                let tx = mx
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                tx.rollback()
                    .await
                    .map_err(|e| Error::database(e, "Rollback failed"))
            }
        }
    }

    /// Returns `true` if the transaction has not yet been committed or rolled back.
    pub async fn is_open(&self) -> bool {
        match &self.inner {
            TransactionInner::Postgres(mx) => mx.lock().await.is_some(),
            TransactionInner::Mysql(mx) => mx.lock().await.is_some(),
            TransactionInner::Sqlite(mx) => mx.lock().await.is_some(),
        }
    }

    /// Execute a mutation SQL inside this transaction and return the number of
    /// affected rows.
    ///
    /// Used when `return_data = false` so no RETURNING clause is emitted and
    /// the affected-row count comes from the database execution result.
    pub async fn execute_affected(&self, sql: &Sql) -> Result<usize> {
        match &self.inner {
            TransactionInner::Postgres(tx_arc) => {
                let mut guard = tx_arc.lock().await;
                let tx = guard
                    .as_mut()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                let mut query = sqlx::query(&sql.text);
                for param in &sql.params {
                    query = crate::postgres::bind_value(query, param)?;
                }
                use sqlx::Executor as _;
                let result = (&mut **tx)
                    .execute(query)
                    .await
                    .map_err(|e| Error::database(e, "Mutation failed"))?;
                Ok(result.rows_affected() as usize)
            }
            TransactionInner::Mysql(tx_arc) => {
                let mut guard = tx_arc.lock().await;
                let tx = guard
                    .as_mut()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                let mut query = sqlx::query(&sql.text);
                for param in &sql.params {
                    query = crate::mysql::bind_value(query, param)?;
                }
                use sqlx::Executor as _;
                let result = (&mut **tx)
                    .execute(query)
                    .await
                    .map_err(|e| Error::database(e, "Mutation failed"))?;
                Ok(result.rows_affected() as usize)
            }
            TransactionInner::Sqlite(tx_arc) => {
                let mut guard = tx_arc.lock().await;
                let tx = guard
                    .as_mut()
                    .ok_or_else(|| Error::database_msg("Transaction already closed"))?;
                let mut query = sqlx::query(&sql.text);
                for param in &sql.params {
                    query = crate::sqlite::bind_value(query, param)?;
                }
                use sqlx::Executor as _;
                let result = (&mut **tx)
                    .execute(query)
                    .await
                    .map_err(|e| Error::database(e, "Mutation failed"))?;
                Ok(result.rows_affected() as usize)
            }
        }
    }
}

impl Executor for TransactionExecutor {
    type Row<'conn>
        = Row
    where
        Self: 'conn;
    type RowStream<'conn>
        = RowStream
    where
        Self: 'conn;

    fn execute<'conn>(&'conn self, sql: &'conn Sql) -> Self::RowStream<'conn> {
        let sql_text = sql.text.clone();
        let params = sql.params.clone();

        match &self.inner {
            TransactionInner::Postgres(tx_arc) => {
                let tx_arc = Arc::clone(tx_arc);
                let stream = async_stream::stream! {
                    let mut guard = tx_arc.lock().await;
                    let tx = match guard.as_mut() {
                        Some(tx) => tx,
                        None => { yield Err(Error::database_msg("Transaction already closed")); return; }
                    };
                    let mut query = sqlx::query(&sql_text);
                    for param in &params {
                        query = match crate::postgres::bind_value(query, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    use sqlx::Executor as _;
                    let rows = match (&mut **tx).fetch_all(query).await {
                        Ok(rows) => rows,
                        Err(e) => { yield Err(Error::database(e, "Query failed")); return; }
                    };
                    drop(guard);
                    for row in rows {
                        match crate::postgres_stream::decode_row_internal(row) {
                            Ok(r) => yield Ok(r),
                            Err(e) => yield Err(e),
                        }
                    }
                };
                RowStream::new_from_stream(Box::pin(stream))
            }
            TransactionInner::Mysql(tx_arc) => {
                let tx_arc = Arc::clone(tx_arc);
                let stream = async_stream::stream! {
                    let mut guard = tx_arc.lock().await;
                    let tx = match guard.as_mut() {
                        Some(tx) => tx,
                        None => { yield Err(Error::database_msg("Transaction already closed")); return; }
                    };
                    let mut query = sqlx::query(&sql_text);
                    for param in &params {
                        query = match crate::mysql::bind_value(query, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    use sqlx::Executor as _;
                    let rows = match (&mut **tx).fetch_all(query).await {
                        Ok(rows) => rows,
                        Err(e) => { yield Err(Error::database(e, "Query failed")); return; }
                    };
                    drop(guard);
                    for row in rows {
                        match crate::mysql_stream::decode_row_internal(row) {
                            Ok(r) => yield Ok(r),
                            Err(e) => yield Err(e),
                        }
                    }
                };
                RowStream::new_from_stream(Box::pin(stream))
            }
            TransactionInner::Sqlite(tx_arc) => {
                let tx_arc = Arc::clone(tx_arc);
                let stream = async_stream::stream! {
                    let mut guard = tx_arc.lock().await;
                    let tx = match guard.as_mut() {
                        Some(tx) => tx,
                        None => { yield Err(Error::database_msg("Transaction already closed")); return; }
                    };
                    let mut query = sqlx::query(&sql_text);
                    for param in &params {
                        query = match crate::sqlite::bind_value(query, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    use sqlx::Executor as _;
                    let rows = match (&mut **tx).fetch_all(query).await {
                        Ok(rows) => rows,
                        Err(e) => { yield Err(Error::database(e, "Query failed")); return; }
                    };
                    drop(guard);
                    for row in rows {
                        match crate::sqlite_stream::decode_row_internal(row) {
                            Ok(r) => yield Ok(r),
                            Err(e) => yield Err(e),
                        }
                    }
                };
                RowStream::new_from_stream(Box::pin(stream))
            }
        }
    }

    fn execute_and_fetch<'conn>(
        &'conn self,
        mutation: &'conn Sql,
        fetch: &'conn Sql,
    ) -> Self::RowStream<'conn> {
        let mutation_text = mutation.text.clone();
        let mutation_params = mutation.params.clone();
        let fetch_text = fetch.text.clone();
        let fetch_params = fetch.params.clone();

        match &self.inner {
            TransactionInner::Postgres(tx_arc) => {
                let tx_arc = Arc::clone(tx_arc);
                let stream = async_stream::stream! {
                    let mut guard = tx_arc.lock().await;
                    let tx = match guard.as_mut() {
                        Some(tx) => tx,
                        None => { yield Err(Error::database_msg("Transaction already closed")); return; }
                    };
                    let mut mq = sqlx::query(&mutation_text);
                    for param in &mutation_params {
                        mq = match crate::postgres::bind_value(mq, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    use sqlx::Executor as _;
                    if let Err(e) = (&mut **tx).execute(mq).await {
                        yield Err(Error::database(e, "Mutation failed")); return;
                    }
                    let mut fq = sqlx::query(&fetch_text);
                    for param in &fetch_params {
                        fq = match crate::postgres::bind_value(fq, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    let rows = match (&mut **tx).fetch_all(fq).await {
                        Ok(rows) => rows,
                        Err(e) => { yield Err(Error::database(e, "Fetch failed")); return; }
                    };
                    drop(guard);
                    for row in rows {
                        match crate::postgres_stream::decode_row_internal(row) {
                            Ok(r) => yield Ok(r),
                            Err(e) => yield Err(e),
                        }
                    }
                };
                RowStream::new_from_stream(Box::pin(stream))
            }
            TransactionInner::Mysql(tx_arc) => {
                let tx_arc = Arc::clone(tx_arc);
                let stream = async_stream::stream! {
                    let mut guard = tx_arc.lock().await;
                    let tx = match guard.as_mut() {
                        Some(tx) => tx,
                        None => { yield Err(Error::database_msg("Transaction already closed")); return; }
                    };
                    let mut mq = sqlx::query(&mutation_text);
                    for param in &mutation_params {
                        mq = match crate::mysql::bind_value(mq, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    use sqlx::Executor as _;
                    if let Err(e) = (&mut **tx).execute(mq).await {
                        yield Err(Error::database(e, "Mutation failed")); return;
                    }
                    let mut fq = sqlx::query(&fetch_text);
                    for param in &fetch_params {
                        fq = match crate::mysql::bind_value(fq, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    let rows = match (&mut **tx).fetch_all(fq).await {
                        Ok(rows) => rows,
                        Err(e) => { yield Err(Error::database(e, "Fetch failed")); return; }
                    };
                    drop(guard);
                    for row in rows {
                        match crate::mysql_stream::decode_row_internal(row) {
                            Ok(r) => yield Ok(r),
                            Err(e) => yield Err(e),
                        }
                    }
                };
                RowStream::new_from_stream(Box::pin(stream))
            }
            TransactionInner::Sqlite(tx_arc) => {
                let tx_arc = Arc::clone(tx_arc);
                let stream = async_stream::stream! {
                    let mut guard = tx_arc.lock().await;
                    let tx = match guard.as_mut() {
                        Some(tx) => tx,
                        None => { yield Err(Error::database_msg("Transaction already closed")); return; }
                    };
                    let mut mq = sqlx::query(&mutation_text);
                    for param in &mutation_params {
                        mq = match crate::sqlite::bind_value(mq, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    use sqlx::Executor as _;
                    if let Err(e) = (&mut **tx).execute(mq).await {
                        yield Err(Error::database(e, "Mutation failed")); return;
                    }
                    let mut fq = sqlx::query(&fetch_text);
                    for param in &fetch_params {
                        fq = match crate::sqlite::bind_value(fq, param) {
                            Ok(q) => q,
                            Err(e) => { yield Err(e); return; }
                        };
                    }
                    let rows = match (&mut **tx).fetch_all(fq).await {
                        Ok(rows) => rows,
                        Err(e) => { yield Err(Error::database(e, "Fetch failed")); return; }
                    };
                    drop(guard);
                    for row in rows {
                        match crate::sqlite_stream::decode_row_internal(row) {
                            Ok(r) => yield Ok(r),
                            Err(e) => yield Err(e),
                        }
                    }
                };
                RowStream::new_from_stream(Box::pin(stream))
            }
        }
    }
}
