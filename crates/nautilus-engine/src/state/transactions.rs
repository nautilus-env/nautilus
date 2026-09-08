//! Interactive transactions: their registry, lifetime and the implicit
//! transaction the engine opens around a multi-statement write.

use std::time::{Duration, Instant};

use nautilus_connector::{Client, TransactionExecutor};
use nautilus_dialect::{MysqlDialect, PostgresDialect, SqliteDialect};
use nautilus_protocol::ProtocolError;

use crate::state::{DatabaseClient, EngineState};

const EXPIRED_TRANSACTION_RETENTION: Duration = Duration::from_secs(60);

/// Lifetime granted to a transaction the engine opens on the caller's behalf.
///
/// Long enough that a nested write or a read-back cannot be reaped mid-flight,
/// short enough that a handler which somehow never finishes still releases its
/// connection back to the pool.
const IMPLICIT_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// An active interactive transaction managed by the engine.
#[derive(Clone)]
pub struct ActiveTransaction {
    /// The transaction-scoped database client.
    pub client: TransactionClient,
    /// When this transaction was started.
    pub created_at: Instant,
    /// Maximum lifetime before auto-rollback.
    pub timeout: Duration,
}

/// A transaction-scoped database client shared across all backends.
pub type TransactionClient = Client<TransactionExecutor>;

impl EngineState {
    /// Run `operation` on one connection, opening a transaction for it when the
    /// caller did not supply one.
    ///
    /// An implicit transaction is committed when `operation` returns `Ok` and
    /// rolled back otherwise; a caller-supplied one is left open, since its
    /// lifetime belongs to whoever started it. Statements that have to observe
    /// each other — an insert and the read-back of the key it generated, a
    /// parent row and the children hung from it — must share a connection, and
    /// the pool hands out a different one per statement.
    pub async fn in_transaction<T, F, Fut>(
        &self,
        transaction_id: Option<&str>,
        operation: F,
    ) -> Result<T, ProtocolError>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, ProtocolError>>,
    {
        if let Some(id) = transaction_id {
            return operation(id.to_string()).await;
        }

        let id = uuid::Uuid::new_v4().to_string();
        self.begin_transaction(id.clone(), IMPLICIT_TRANSACTION_TIMEOUT, None)
            .await?;

        match operation(id.clone()).await {
            Ok(value) => {
                self.commit_transaction(&id).await?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.rollback_transaction(&id).await;
                Err(error)
            }
        }
    }

    /// Begin a new interactive transaction.
    pub async fn begin_transaction(
        &self,
        id: String,
        timeout: Duration,
        isolation_level: Option<nautilus_protocol::IsolationLevel>,
    ) -> Result<(), ProtocolError> {
        let isolation = isolation_level.map(connector_isolation_level);
        let tx_client = match &self.client {
            DatabaseClient::Postgres(c) => {
                let tx_exec = TransactionExecutor::begin_postgres(c.executor().pool(), isolation)
                    .await
                    .map_err(|e| ProtocolError::TransactionFailed(e.to_string()))?;
                Client::new(PostgresDialect, tx_exec)
            }
            DatabaseClient::Mysql(c) => {
                let tx_exec = TransactionExecutor::begin_mysql(c.executor().pool(), isolation)
                    .await
                    .map_err(|e| ProtocolError::TransactionFailed(e.to_string()))?;
                Client::new(MysqlDialect, tx_exec)
            }
            DatabaseClient::Sqlite(c) => {
                // SQLite has no SET TRANSACTION ISOLATION LEVEL to apply.
                let tx_exec = TransactionExecutor::begin_sqlite(c.executor().pool())
                    .await
                    .map_err(|e| ProtocolError::TransactionFailed(e.to_string()))?;
                Client::new(SqliteDialect, tx_exec)
            }
        };

        let active = ActiveTransaction {
            client: tx_client,
            created_at: Instant::now(),
            timeout,
        };

        self.expired_transactions.lock().await.remove(&id);
        self.transactions.lock().await.insert(id, active);
        Ok(())
    }

    /// Register an already-open transaction client so engine requests can reuse it.
    ///
    /// This is used by embedded generated clients that manage the database
    /// transaction outside the engine but still want all query semantics to flow
    /// through the engine handlers.
    pub async fn register_external_transaction(
        &self,
        id: String,
        client: TransactionClient,
        timeout: Duration,
    ) {
        let active = ActiveTransaction {
            client,
            created_at: Instant::now(),
            timeout,
        };

        self.expired_transactions.lock().await.remove(&id);
        self.transactions.lock().await.insert(id, active);
    }

    /// Remove a previously registered external transaction without committing it.
    ///
    /// The caller remains responsible for committing or rolling back the actual
    /// database transaction.
    pub async fn unregister_external_transaction(&self, id: &str) {
        self.transactions.lock().await.remove(id);
        self.expired_transactions.lock().await.remove(id);
    }

    /// Commit a transaction by ID and remove it from the map.
    pub async fn commit_transaction(&self, id: &str) -> Result<(), ProtocolError> {
        let active = self.take_transaction(id).await?;
        if active.created_at.elapsed() > active.timeout {
            self.expire_active_transaction(id, active).await;
            return Err(Self::transaction_timeout_error(id));
        }
        active
            .client
            .executor()
            .commit()
            .await
            .map_err(|e| ProtocolError::TransactionFailed(format!("Commit failed: {}", e)))
    }

    /// Rollback a transaction by ID and remove it from the map.
    pub async fn rollback_transaction(&self, id: &str) -> Result<(), ProtocolError> {
        let active = self.take_transaction(id).await?;
        if active.created_at.elapsed() > active.timeout {
            self.expire_active_transaction(id, active).await;
            return Err(Self::transaction_timeout_error(id));
        }
        active
            .client
            .executor()
            .rollback()
            .await
            .map_err(|e| ProtocolError::TransactionFailed(format!("Rollback failed: {}", e)))
    }

    /// Expire (rollback + remove) a timed-out transaction.
    async fn expire_transaction(&self, id: &str) {
        if let Some(active) = self.transactions.lock().await.remove(id) {
            self.expire_active_transaction(id, active).await;
        }
    }

    /// Reap all timed-out transactions. Called periodically by the engine.
    pub async fn reap_expired_transactions(&self) {
        let expired: Vec<(String, ActiveTransaction)> = {
            let mut txs = self.transactions.lock().await;
            let expired_ids: Vec<String> = txs
                .iter()
                .filter(|(_, tx)| tx.created_at.elapsed() > tx.timeout)
                .map(|(id, _)| id.clone())
                .collect();
            expired_ids
                .into_iter()
                .filter_map(|id| txs.remove(&id).map(|active| (id, active)))
                .collect()
        };
        for (id, active) in expired {
            tracing::warn!(transaction_id = %id, "reaping expired transaction");
            self.expire_active_transaction(&id, active).await;
        }
    }

    fn transaction_timeout_error(id: &str) -> ProtocolError {
        ProtocolError::TransactionTimeout(format!("Transaction '{}' timed out", id))
    }

    fn transaction_not_found_error(id: &str) -> ProtocolError {
        ProtocolError::TransactionNotFound(format!("Transaction '{}' not found", id))
    }

    async fn transaction_lookup_error(&self, id: &str) -> ProtocolError {
        let mut expired = self.expired_transactions.lock().await;
        expired.retain(|_, expired_at| expired_at.elapsed() <= EXPIRED_TRANSACTION_RETENTION);
        if expired.contains_key(id) {
            Self::transaction_timeout_error(id)
        } else {
            Self::transaction_not_found_error(id)
        }
    }

    pub(super) async fn transaction_client_for_request(
        &self,
        id: &str,
    ) -> Result<TransactionClient, ProtocolError> {
        enum TransactionLookup {
            Ready(TransactionClient),
            TimedOut,
            Missing,
        }

        let lookup = {
            let txs = self.transactions.lock().await;
            match txs.get(id) {
                Some(active) if active.created_at.elapsed() > active.timeout => {
                    TransactionLookup::TimedOut
                }
                Some(active) => TransactionLookup::Ready(active.client.clone()),
                None => TransactionLookup::Missing,
            }
        };

        match lookup {
            TransactionLookup::Ready(client) => Ok(client),
            TransactionLookup::TimedOut => {
                self.expire_transaction(id).await;
                Err(Self::transaction_timeout_error(id))
            }
            TransactionLookup::Missing => Err(self.transaction_lookup_error(id).await),
        }
    }

    async fn take_transaction(&self, id: &str) -> Result<ActiveTransaction, ProtocolError> {
        match self.transactions.lock().await.remove(id) {
            Some(active) => Ok(active),
            None => Err(self.transaction_lookup_error(id).await),
        }
    }

    async fn expire_active_transaction(&self, id: &str, active: ActiveTransaction) {
        {
            let mut expired = self.expired_transactions.lock().await;
            expired.retain(|_, expired_at| expired_at.elapsed() <= EXPIRED_TRANSACTION_RETENTION);
            expired.insert(id.to_string(), Instant::now());
        }
        let _ = active.client.executor().rollback().await;
    }
}

/// Map the protocol's isolation level onto the connector's.
fn connector_isolation_level(
    level: nautilus_protocol::IsolationLevel,
) -> nautilus_connector::IsolationLevel {
    match level {
        nautilus_protocol::IsolationLevel::ReadUncommitted => {
            nautilus_connector::IsolationLevel::ReadUncommitted
        }
        nautilus_protocol::IsolationLevel::ReadCommitted => {
            nautilus_connector::IsolationLevel::ReadCommitted
        }
        nautilus_protocol::IsolationLevel::RepeatableRead => {
            nautilus_connector::IsolationLevel::RepeatableRead
        }
        nautilus_protocol::IsolationLevel::Serializable => {
            nautilus_connector::IsolationLevel::Serializable
        }
    }
}
