//! The live transaction itself: which backend holds it, and its lifecycle.
//!
//! The transaction is stored behind an `Arc<Mutex<Option<…>>>` so the executor
//! stays cheap to clone through [`crate::client::Client`]'s `Arc<E>`, and so
//! commit and rollback can take it out of the handle exactly once. A handle
//! whose slot is empty has already been completed, and every later query on it
//! is refused.

use std::sync::Arc;

use tokio::sync::Mutex;

use super::mysql::MysqlTransaction;
use crate::error::{ConnectorError as Error, Result};

/// A live transaction shared by the executor's query helpers.
pub(super) type TxHandle<T> = Arc<Mutex<Option<T>>>;

/// Per-backend transaction storage.
///
/// This is a private implementation detail — callers always interact with the
/// outer [`super::TransactionExecutor`] type.
pub(super) enum TransactionInner {
    Postgres(TxHandle<sqlx::Transaction<'static, sqlx::Postgres>>),
    Mysql(TxHandle<MysqlTransaction>),
    Sqlite(TxHandle<sqlx::Transaction<'static, sqlx::Sqlite>>),
}

/// Put a freshly begun transaction into a shareable handle.
pub(super) fn handle<T>(transaction: T) -> TxHandle<T> {
    Arc::new(Mutex::new(Some(transaction)))
}

impl TransactionInner {
    /// Take the transaction out of its handle and commit it.
    pub(super) async fn commit(&self) -> Result<()> {
        match self {
            Self::Postgres(handle) => take(handle)
                .await?
                .commit()
                .await
                .map_err(|e| Error::database(e, "Commit failed")),
            Self::Mysql(handle) => take(handle)
                .await?
                .commit()
                .await
                .map_err(|e| Error::database(e, "Commit failed")),
            Self::Sqlite(handle) => take(handle)
                .await?
                .commit()
                .await
                .map_err(|e| Error::database(e, "Commit failed")),
        }
    }

    /// Take the transaction out of its handle and roll it back.
    pub(super) async fn rollback(&self) -> Result<()> {
        match self {
            Self::Postgres(handle) => take(handle)
                .await?
                .rollback()
                .await
                .map_err(|e| Error::database(e, "Rollback failed")),
            Self::Mysql(handle) => take(handle)
                .await?
                .rollback()
                .await
                .map_err(|e| Error::database(e, "Rollback failed")),
            Self::Sqlite(handle) => take(handle)
                .await?
                .rollback()
                .await
                .map_err(|e| Error::database(e, "Rollback failed")),
        }
    }

    /// Whether the transaction has not yet been committed or rolled back.
    pub(super) async fn is_open(&self) -> bool {
        match self {
            Self::Postgres(handle) => is_open(handle).await,
            Self::Mysql(handle) => is_open(handle).await,
            Self::Sqlite(handle) => is_open(handle).await,
        }
    }
}

async fn take<T>(handle: &TxHandle<T>) -> Result<T> {
    handle
        .lock()
        .await
        .take()
        .ok_or_else(|| Error::database_msg("Transaction already closed"))
}

async fn is_open<T>(handle: &TxHandle<T>) -> bool {
    handle.lock().await.is_some()
}
