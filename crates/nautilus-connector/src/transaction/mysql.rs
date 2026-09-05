//! MySQL transaction ownership and isolation setup.
//!
//! MySQL applies `SET TRANSACTION` to the next transaction, so isolation must be
//! set before `BEGIN` on the same connection. A pooled guard owns that connection
//! across both operations: incomplete preparation closes it, while an open
//! transaction uses SQLx's rollback-on-drop handling.

use std::ops::{Deref, DerefMut};

use sqlx::{
    mysql::MySqlTransactionManager, pool::PoolConnection, MySql, MySqlConnection, MySqlPool,
    TransactionManager,
};

use super::IsolationLevel;
use crate::error::{ConnectorError as Error, Result};

/// MySQL transaction storage shared by the executor's query helpers.
///
/// Both variants dereference to the transaction's connection, preserving support
/// for externally opened SQLx transactions alongside the pooled opener.
pub(super) enum MysqlTransaction {
    /// An already-open transaction supplied by the caller.
    Sqlx(sqlx::Transaction<'static, MySql>),
    /// A transaction whose isolation setup and connection are owned here.
    Pooled(PooledMysqlTransaction),
}

/// Determines the cleanup required when a pooled transaction is dropped.
enum State {
    /// Isolation may be pending or `BEGIN` may be awaiting acknowledgement.
    Preparing,
    /// `BEGIN` succeeded; dropping the guard must queue a rollback.
    Open,
    /// Commit or rollback succeeded; the connection can return to the pool.
    Closed,
}

/// Owns a pooled connection from preparation through transaction completion.
///
/// The connection stays inside this guard while SQLx borrows it for `BEGIN`, so
/// cancellation cannot return a pending isolation override to the pool.
pub(super) struct PooledMysqlTransaction {
    connection: PoolConnection<MySql>,
    state: State,
}

impl PooledMysqlTransaction {
    /// Acquire a connection and optionally set isolation for its next transaction.
    ///
    /// The returned guard is still preparing: dropping it before a successful
    /// `BEGIN` discards the connection, including on setup errors or cancellation.
    async fn prepare(pool: &MySqlPool, isolation: Option<IsolationLevel>) -> Result<Self> {
        let mut tx = Self {
            connection: pool
                .acquire()
                .await
                .map_err(|e| Error::connection(e, "Failed to acquire transaction connection"))?,
            state: State::Preparing,
        };
        if let Some(isolation) = isolation {
            sqlx::query(&format!(
                "SET TRANSACTION ISOLATION LEVEL {}",
                isolation.as_sql()
            ))
            .persistent(false)
            .execute(&mut *tx.connection)
            .await
            .map_err(|e| Error::database(e, "SET ISOLATION failed"))?;
        }
        Ok(tx)
    }

    /// Commit and release the connection after SQLx acknowledges completion.
    ///
    /// On error or cancellation, the open guard still queues rollback on drop.
    async fn commit(mut self) -> std::result::Result<(), sqlx::Error> {
        MySqlTransactionManager::commit(&mut self.connection).await?;
        self.state = State::Closed;
        Ok(())
    }

    /// Roll back and release the connection after SQLx acknowledges completion.
    ///
    /// On error or cancellation, the open guard still queues rollback on drop.
    async fn rollback(mut self) -> std::result::Result<(), sqlx::Error> {
        MySqlTransactionManager::rollback(&mut self.connection).await?;
        self.state = State::Closed;
        Ok(())
    }
}

impl Drop for PooledMysqlTransaction {
    /// Discard incomplete preparation or queue rollback for an open transaction.
    fn drop(&mut self) {
        match self.state {
            // SET affects the next transaction, and BEGIN may have reached the
            // server without being acknowledged. Neither state may enter the pool.
            State::Preparing => self.connection.close_on_drop(),
            State::Open => MySqlTransactionManager::start_rollback(&mut self.connection),
            State::Closed => {}
        }
    }
}

impl MysqlTransaction {
    /// Prepare isolation and begin a transaction on the same pooled connection.
    ///
    /// Errors or cancellation before `BEGIN` completes discard the connection.
    /// Successful transactions retain normal pool reuse after commit or rollback.
    pub(super) async fn begin(pool: &MySqlPool, isolation: Option<IsolationLevel>) -> Result<Self> {
        let mut tx = PooledMysqlTransaction::prepare(pool, isolation).await?;
        MySqlTransactionManager::begin(&mut tx.connection, None)
            .await
            .map_err(|e| Error::connection(e, "Failed to begin transaction"))?;
        tx.state = State::Open;
        Ok(Self::Pooled(tx))
    }

    /// Consume the handle and commit through its owning transaction implementation.
    pub(super) async fn commit(self) -> std::result::Result<(), sqlx::Error> {
        match self {
            Self::Sqlx(tx) => tx.commit().await,
            Self::Pooled(tx) => tx.commit().await,
        }
    }

    /// Consume the handle and roll back through its owning transaction implementation.
    pub(super) async fn rollback(self) -> std::result::Result<(), sqlx::Error> {
        match self {
            Self::Sqlx(tx) => tx.rollback().await,
            Self::Pooled(tx) => tx.rollback().await,
        }
    }
}

impl Deref for MysqlTransaction {
    type Target = MySqlConnection;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Sqlx(tx) => tx,
            Self::Pooled(tx) => &tx.connection,
        }
    }
}

impl DerefMut for MysqlTransaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Sqlx(tx) => tx,
            Self::Pooled(tx) => &mut tx.connection,
        }
    }
}

#[cfg(test)]
mod tests;
