use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use nautilus_connector::{
    execute_all, Client, ConnectorPoolOptions, Executor, MysqlExecutor, PgExecutor, Row, RowStream,
    SqliteExecutor, SqlxErrorKind, TransactionExecutor,
};
use nautilus_dialect::{Dialect, MysqlDialect, PostgresDialect, Sql, SqliteDialect};
use nautilus_migrate::DatabaseProvider;
use nautilus_protocol::{EngineMetricsResult, PoolMetrics, ProtocolError};
use nautilus_schema::ir::{ModelIr, SchemaIr};

use crate::filter::RelationMap;
use crate::metadata::ModelMetadata;
use crate::metrics::EngineMetrics;
use crate::observability::StatementTimer;
use crate::plan_cache::PlanCache;
use crate::pool_options::EnginePoolOptions;

const EXPIRED_TRANSACTION_RETENTION: Duration = Duration::from_secs(60);

/// Lifetime granted to a transaction the engine opens on the caller's behalf.
///
/// Long enough that a nested write or a read-back cannot be reaped mid-flight,
/// short enough that a handler which somehow never finishes still releases its
/// connection back to the pool.
const IMPLICIT_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Convert a [`nautilus_connector::ConnectorError`] to the appropriate [`ProtocolError`],
/// mapping specific constraint violation kinds to their dedicated error codes.
pub(crate) fn connector_to_protocol(
    e: nautilus_connector::ConnectorError,
    context: &str,
) -> ProtocolError {
    let msg = format!("{}: {}", context, e);
    match e.sqlx_kind() {
        SqlxErrorKind::UniqueConstraint => ProtocolError::UniqueConstraintViolation(msg),
        SqlxErrorKind::ForeignKeyConstraint => ProtocolError::ForeignKeyConstraintViolation(msg),
        SqlxErrorKind::CheckConstraint => ProtocolError::CheckConstraintViolation(msg),
        SqlxErrorKind::NullConstraint => ProtocolError::NullConstraintViolation(msg),
        SqlxErrorKind::Deadlock => ProtocolError::Deadlock(msg),
        SqlxErrorKind::SerializationFailure => ProtocolError::SerializationFailure(msg),
        SqlxErrorKind::PoolTimedOut | SqlxErrorKind::PoolClosed => {
            ProtocolError::ConnectionFailed(msg)
        }
        _ => ProtocolError::DatabaseExecution(msg),
    }
}

/// Engine state holding parsed schema and database connection.
pub struct EngineState {
    /// Cached per-model metadata reused by the hot query paths.
    model_metadata: HashMap<String, ModelMetadata>,
    /// The full validated schema IR.
    pub schema: SchemaIr,
    /// SQL dialect renderer.
    pub dialect: Arc<dyn Dialect + Send + Sync>,
    /// Database connection (pooled / proxied URL).
    pub client: DatabaseClient,
    /// Optional direct connection that bypasses poolers like PgBouncer.
    /// Used for raw SQL queries when `direct_url` is configured in the schema.
    direct_client: Option<DatabaseClient>,
    /// Active interactive transactions, keyed by transaction ID.
    pub transactions: Arc<Mutex<HashMap<String, ActiveTransaction>>>,
    /// Recently expired interactive transactions, kept briefly so late follow-up
    /// calls still report a timeout instead of an unknown transaction.
    expired_transactions: Arc<Mutex<HashMap<String, Instant>>>,
    /// Cached SQL plans for repeated read shapes (e.g. `findUnique` by id).
    plan_cache: PlanCache,
    /// Upper bound on requests the transport handles concurrently.
    max_concurrent_requests: usize,
    /// Duration past which an executed statement is logged, when configured.
    slow_query_threshold: Option<Duration>,
    /// Backend this state is connected to.
    provider: DatabaseProvider,
    /// Runtime counters served by `engine.metrics`.
    metrics: EngineMetrics,
}

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

/// Enum to hold different client types.
pub enum DatabaseClient {
    /// PostgreSQL client.
    Postgres(Client<PgExecutor>),
    /// MySQL client.
    Mysql(Client<MysqlExecutor>),
    /// SQLite client.
    Sqlite(Client<SqliteExecutor>),
}

/// Dispatch an expression across all [`DatabaseClient`] variants.
macro_rules! with_client {
    ($self:expr, $client:ident => $body:expr) => {
        match $self {
            DatabaseClient::Postgres($client) => $body,
            DatabaseClient::Mysql($client) => $body,
            DatabaseClient::Sqlite($client) => $body,
        }
    };
}

impl DatabaseClient {
    /// Connection-pool counters as reported by the driver.
    pub fn pool_metrics(&self) -> PoolMetrics {
        with_client!(self, client => PoolMetrics {
            size: client.executor().pool().size(),
            idle: client.executor().pool().num_idle(),
        })
    }

    /// Execute a rendered SQL query and return all result rows.
    pub async fn execute_query(&self, sql: &Sql, context: &str) -> Result<Vec<Row>, ProtocolError> {
        with_client!(self, client => {
            execute_all(client.executor(), sql)
                .await
                .map_err(|e| connector_to_protocol(e, context))
        })
    }

    /// Execute a rendered SQL query with sqlx statement persistence disabled.
    ///
    /// This is used only for raw/direct query paths that may run through
    /// PgBouncer-style transaction poolers.
    pub async fn execute_query_unprepared(
        &self,
        sql: &Sql,
        context: &str,
    ) -> Result<Vec<Row>, ProtocolError> {
        match self {
            DatabaseClient::Postgres(client) => client
                .executor()
                .execute_collect_unprepared(sql)
                .await
                .map_err(|e| connector_to_protocol(e, context)),
            DatabaseClient::Mysql(client) => execute_all(client.executor(), sql)
                .await
                .map_err(|e| connector_to_protocol(e, context)),
            DatabaseClient::Sqlite(client) => execute_all(client.executor(), sql)
                .await
                .map_err(|e| connector_to_protocol(e, context)),
        }
    }

    /// Execute a mutation SQL and return the number of affected rows.
    pub async fn execute_affected(&self, sql: &Sql, context: &str) -> Result<usize, ProtocolError> {
        with_client!(self, client => {
            client.executor()
                .execute_affected(sql)
                .await
                .map_err(|e| connector_to_protocol(e, context))
        })
    }

    /// Execute a raw DDL statement (no parameters, no result rows).
    pub async fn execute_raw(&self, stmt: &str) -> Result<(), Box<dyn std::error::Error>> {
        with_client!(self, client => client.executor().execute_raw(stmt).await?);
        Ok(())
    }

    /// Execute a rendered SQL query and return a row-by-row stream that owns
    /// its database connection.
    ///
    /// Unlike [`Self::execute_query`], which materialises the full result set,
    /// this path drives the underlying sqlx stream from a worker task. The
    /// returned [`RowStream`] is `'static` and can be moved between tasks; if
    /// the consumer drops it mid-iteration, the worker drains the remaining
    /// rows so the connection returns to the pool clean.
    pub fn execute_query_stream(&self, sql: Sql) -> RowStream<'static> {
        with_client!(self, client => client.executor().execute_owned(sql))
    }
}

impl EngineState {
    /// Connect to a database and return a `(dialect, client)` pair.
    async fn build_client(
        provider: DatabaseProvider,
        url: &str,
        pool_options: EnginePoolOptions,
    ) -> Result<(Arc<dyn Dialect + Send + Sync>, DatabaseClient), Box<dyn std::error::Error>> {
        let connector_pool_options = pool_options.to_connector_pool_options();
        match provider {
            DatabaseProvider::Postgres => {
                let pg_client = Client::postgres_with_options(url, connector_pool_options).await?;
                let dialect: Arc<dyn Dialect + Send + Sync> = Arc::new(PostgresDialect);
                Ok((dialect, DatabaseClient::Postgres(pg_client)))
            }
            DatabaseProvider::Mysql => {
                let mysql_client = Client::mysql_with_options(url, connector_pool_options).await?;
                let dialect: Arc<dyn Dialect + Send + Sync> = Arc::new(MysqlDialect);
                Ok((dialect, DatabaseClient::Mysql(mysql_client)))
            }
            DatabaseProvider::Sqlite => {
                let sqlite_client =
                    Client::sqlite_with_options(url, connector_pool_options).await?;
                let dialect: Arc<dyn Dialect + Send + Sync> = Arc::new(SqliteDialect);
                Ok((dialect, DatabaseClient::Sqlite(sqlite_client)))
            }
        }
    }

    /// Create a new engine state by connecting to the database.
    ///
    /// `direct_url`, when provided, opens a second connection that bypasses
    /// poolers (e.g. PgBouncer). Raw SQL queries prefer this connection.
    pub async fn new(
        schema: SchemaIr,
        database_url: String,
        direct_url: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_engine_pool_options(
            schema,
            database_url,
            direct_url,
            EnginePoolOptions::default(),
        )
        .await
    }

    /// Create a new engine state by connecting to the database with explicit pool overrides.
    ///
    /// `direct_url`, when provided, opens a second connection that bypasses
    /// poolers (e.g. PgBouncer). Raw SQL queries prefer this connection.
    pub async fn new_with_pool_options(
        schema: SchemaIr,
        database_url: String,
        direct_url: Option<String>,
        pool_options: ConnectorPoolOptions,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_engine_pool_options(
            schema,
            database_url,
            direct_url,
            EnginePoolOptions::from_connector_pool_options(pool_options),
        )
        .await
    }

    /// Create a new engine state with explicit engine-level pool overrides.
    pub async fn new_with_engine_pool_options(
        schema: SchemaIr,
        database_url: String,
        direct_url: Option<String>,
        pool_options: EnginePoolOptions,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let datasource = schema
            .datasource
            .as_ref()
            .ok_or("No datasource found in schema")?;

        let provider = DatabaseProvider::from_schema_provider(&datasource.provider)
            .ok_or_else(|| format!("Unsupported database provider: {}", datasource.provider))?;

        // Only PostgreSQL stores composite types as native (non-JSON) values,
        // which require record-literal decoding on read.
        let native_composites = matches!(provider, DatabaseProvider::Postgres);
        let model_metadata = schema
            .models
            .iter()
            .map(|(name, model)| {
                (
                    name.clone(),
                    ModelMetadata::new(model, &schema.composite_types, native_composites),
                )
            })
            .collect();

        let resolved_url = resolve_database_url(&database_url)?;
        let (dialect, client) = Self::build_client(provider, &resolved_url, pool_options).await?;

        let direct_client = if let Some(raw_direct) = direct_url {
            let resolved_direct = resolve_database_url(&raw_direct)?;
            let (_, dc) = Self::build_client(provider, &resolved_direct, pool_options).await?;
            Some(dc)
        } else {
            None
        };

        Ok(EngineState {
            model_metadata,
            schema,
            dialect,
            client,
            direct_client,
            transactions: Arc::new(Mutex::new(HashMap::new())),
            expired_transactions: Arc::new(Mutex::new(HashMap::new())),
            plan_cache: PlanCache::default(),
            max_concurrent_requests: pool_options.resolved_max_concurrent_requests(),
            slow_query_threshold: crate::observability::slow_query_threshold(),
            provider,
            metrics: EngineMetrics::default(),
        })
    }

    /// Model lookup map (logical name -> IR), borrowed from the schema IR.
    pub fn models(&self) -> &HashMap<String, ModelIr> {
        &self.schema.models
    }

    /// Read-plan cache shared by hot read paths.
    pub(crate) fn plan_cache(&self) -> &PlanCache {
        &self.plan_cache
    }

    /// Backend this state is connected to.
    pub fn provider(&self) -> DatabaseProvider {
        self.provider
    }

    /// Record one dispatched request against the per-method counters.
    pub(crate) fn record_request(&self, method: &str, elapsed: Duration, failed: bool) {
        self.metrics.record(method, elapsed, failed);
    }

    /// Snapshot every runtime counter, optionally zeroing the cumulative ones.
    pub(crate) async fn metrics_snapshot(&self, reset: bool) -> EngineMetricsResult {
        let snapshot = EngineMetricsResult {
            uptime_seconds: self.metrics.uptime(),
            plan_cache: self.plan_cache.metrics(),
            pool: self.client.pool_metrics(),
            active_transactions: self.transactions.lock().await.len(),
            methods: self.metrics.method_snapshot(),
        };
        if reset {
            self.metrics.reset();
            self.plan_cache.reset_metrics();
        }
        snapshot
    }

    /// Upper bound on requests the transport handles concurrently.
    pub fn max_concurrent_requests(&self) -> usize {
        self.max_concurrent_requests
    }

    /// Whether the active backend stores composite types as native PostgreSQL
    /// composite types (and therefore needs `Value::Composite` binding) rather
    /// than as JSON. Only PostgreSQL supports user-defined composite types.
    pub(crate) fn uses_native_composite_types(&self) -> bool {
        matches!(self.client, DatabaseClient::Postgres(_))
    }

    /// Return cached metadata for a validated model.
    pub(crate) fn model_metadata(&self, model: &ModelIr) -> &ModelMetadata {
        self.model_metadata
            .get(&model.logical_name)
            .expect("engine metadata missing for validated model")
    }

    /// Return the lazily cached relation map for a validated model.
    pub(crate) fn relation_map_for_model(
        &self,
        model: &ModelIr,
    ) -> Result<&RelationMap, ProtocolError> {
        self.model_metadata(model)
            .relation_map(model, &self.schema.models)
    }

    /// Look up a related model together with its cached metadata.
    pub(crate) fn related_model(&self, model_name: &str) -> Option<(&ModelIr, &ModelMetadata)> {
        Some((
            self.schema.models.get(model_name)?,
            self.model_metadata.get(model_name)?,
        ))
    }

    /// Execute raw DDL SQL statements against the database.
    ///
    /// Used for running migrations (CREATE TABLE, etc.).
    pub async fn execute_ddl_sql(
        &self,
        statements: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for stmt in &statements {
            if stmt.trim().is_empty() {
                continue;
            }
            self.client.execute_raw(stmt).await?;
        }
        Ok(())
    }

    /// Start timing a statement for the slow-statement log.
    ///
    /// Only the paths that materialise their result set are timed: the
    /// streaming path hands the connection to the consumer, so its duration
    /// belongs to the caller draining the rows rather than to the statement.
    fn time_statement<'a>(&self, sql: &'a Sql, context: &'a str) -> StatementTimer<'a> {
        StatementTimer::start(self.slow_query_threshold, context, &sql.text)
    }

    /// Execute a SQL query, optionally inside a transaction.
    ///
    /// If `tx_id` is `Some`, the query runs on the transaction's connection;
    /// otherwise it runs on the pool-backed default connection.
    pub async fn execute_query_on(
        &self,
        sql: &Sql,
        context: &str,
        tx_id: Option<&str>,
    ) -> Result<Vec<Row>, ProtocolError> {
        let timer = self.time_statement(sql, context);
        let rows = match tx_id {
            None => self.client.execute_query(sql, context).await,
            Some(id) => {
                let tx_client = self.transaction_client_for_request(id).await?;
                execute_all(tx_client.executor(), sql)
                    .await
                    .map_err(|e| connector_to_protocol(e, context))
            }
        };
        timer.finish();
        rows
    }

    /// Execute a SQL query and return a row-by-row stream, optionally inside a
    /// transaction.
    ///
    /// Unlike [`Self::execute_query_on`], which buffers the full result set,
    /// this path keeps memory bounded for large reads by streaming each row
    /// through a worker-owned connection. The returned stream is `'static`,
    /// so the caller can move it between tasks; dropping the stream
    /// mid-iteration releases the connection cleanly.
    ///
    /// Used by the chunked `findMany` IPC path so partial responses can be
    /// emitted as rows arrive from the database, without first materialising
    /// the whole `Vec<Row>`.
    pub async fn execute_query_stream_on(
        &self,
        sql: Sql,
        tx_id: Option<&str>,
    ) -> Result<RowStream<'static>, ProtocolError> {
        match tx_id {
            None => Ok(self.client.execute_query_stream(sql)),
            Some(id) => {
                let tx_client = self.transaction_client_for_request(id).await?;
                Ok(tx_client.executor().execute_owned(sql))
            }
        }
    }

    /// Execute a SQL query using the direct connection when available, otherwise the pooled one.
    ///
    /// Raw SQL queries should use this so they bypass connection poolers
    /// (e.g. PgBouncer) when possible and disable sqlx statement persistence.
    /// If a `tx_id` is provided the query always runs on the transaction's
    /// connection regardless.
    pub async fn execute_direct_query_on(
        &self,
        sql: &Sql,
        context: &str,
        tx_id: Option<&str>,
    ) -> Result<Vec<Row>, ProtocolError> {
        let timer = self.time_statement(sql, context);
        let rows = match tx_id {
            Some(tx_id) => {
                let tx_client = self.transaction_client_for_request(tx_id).await?;
                tx_client
                    .executor()
                    .execute_collect_unprepared(sql)
                    .await
                    .map_err(|e| connector_to_protocol(e, context))
            }
            None => match &self.direct_client {
                Some(direct) => direct.execute_query_unprepared(sql, context).await,
                None => self.client.execute_query_unprepared(sql, context).await,
            },
        };
        timer.finish();
        rows
    }

    /// Execute a mutation SQL and return the affected-row count, optionally
    /// inside a transaction.
    ///
    /// Use this when `return_data = false` so no RETURNING clause is emitted.
    pub async fn execute_affected_on(
        &self,
        sql: &Sql,
        context: &str,
        tx_id: Option<&str>,
    ) -> Result<usize, ProtocolError> {
        let timer = self.time_statement(sql, context);
        let affected = match tx_id {
            None => self.client.execute_affected(sql, context).await,
            Some(id) => {
                let tx_client = self.transaction_client_for_request(id).await?;
                tx_client
                    .executor()
                    .execute_affected(sql)
                    .await
                    .map_err(|e| connector_to_protocol(e, context))
            }
        };
        timer.finish();
        affected
    }

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
        let tx_client = match &self.client {
            DatabaseClient::Postgres(c) => {
                let sqlx_tx = c.executor().pool().begin().await.map_err(|e| {
                    ProtocolError::TransactionFailed(format!("BEGIN failed: {}", e))
                })?;
                let tx_exec = TransactionExecutor::postgres(sqlx_tx);
                if let Some(iso) = isolation_level {
                    let iso_sql = format!("SET TRANSACTION ISOLATION LEVEL {}", iso.as_sql());
                    let sql = Sql {
                        text: iso_sql,
                        params: vec![],
                    };
                    execute_all(&tx_exec, &sql).await.map_err(|e| {
                        ProtocolError::TransactionFailed(format!("SET ISOLATION failed: {}", e))
                    })?;
                }
                Client::new(PostgresDialect, tx_exec)
            }
            DatabaseClient::Mysql(c) => {
                let sqlx_tx = c.executor().pool().begin().await.map_err(|e| {
                    ProtocolError::TransactionFailed(format!("BEGIN failed: {}", e))
                })?;
                let tx_exec = TransactionExecutor::mysql(sqlx_tx);
                if let Some(iso) = isolation_level {
                    let iso_sql = format!("SET TRANSACTION ISOLATION LEVEL {}", iso.as_sql());
                    let sql = Sql {
                        text: iso_sql,
                        params: vec![],
                    };
                    execute_all(&tx_exec, &sql).await.map_err(|e| {
                        ProtocolError::TransactionFailed(format!("SET ISOLATION failed: {}", e))
                    })?;
                }
                Client::new(MysqlDialect, tx_exec)
            }
            DatabaseClient::Sqlite(c) => {
                let sqlx_tx = c.executor().pool().begin().await.map_err(|e| {
                    ProtocolError::TransactionFailed(format!("BEGIN failed: {}", e))
                })?;
                let tx_exec = TransactionExecutor::sqlite(sqlx_tx);
                // SQLite doesn't support SET TRANSACTION ISOLATION LEVEL
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

    async fn transaction_client_for_request(
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

/// Resolve database URL, handling env() references.
fn resolve_database_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    nautilus_schema::resolve_env_url(url).map_err(|msg| msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;

    use nautilus_core::Value;
    use nautilus_migrate::{DatabaseProvider, DdlGenerator};
    use nautilus_schema::validate_schema_source;
    use tempfile::TempDir;

    fn parse_ir(source: &str) -> SchemaIr {
        validate_schema_source(source)
            .expect("validation failed")
            .ir
    }

    fn test_db_url() -> (String, TempDir) {
        let dir = tempfile::Builder::new()
            .prefix("transaction-timeout-state-tests")
            .tempdir()
            .expect("failed to create sqlite test directory");

        let path = dir.path().join("test.db");
        fs::File::create(&path).expect("failed to create sqlite test file");
        let url = format!("sqlite:///{}", path.to_string_lossy().replace('\\', "/"));
        (url, dir)
    }

    async fn sqlite_state(schema_source: &str) -> (EngineState, TempDir) {
        let schema = parse_ir(schema_source);
        let (database_url, temp_dir) = test_db_url();
        let state = EngineState::new(schema.clone(), database_url, None)
            .await
            .expect("failed to create engine state");

        let ddl = DdlGenerator::new(DatabaseProvider::Sqlite)
            .generate_create_tables(&schema)
            .expect("failed to build ddl");
        state
            .execute_ddl_sql(ddl)
            .await
            .expect("failed to apply ddl");

        (state, temp_dir)
    }

    fn schema_source() -> &'static str {
        r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id   Int    @id @default(autoincrement())
  name String
}
"#
    }

    fn insert_user_sql(name: &str) -> Sql {
        Sql {
            text: r#"INSERT INTO "User" ("name") VALUES (?)"#.to_string(),
            params: vec![Value::String(name.to_string())],
        }
    }

    fn long_running_sql(iterations: usize) -> Sql {
        Sql {
            text: format!(
                "WITH RECURSIVE cnt(x) AS (SELECT 0 UNION ALL SELECT x + 1 FROM cnt WHERE x < {iterations}) SELECT MAX(x) AS value FROM cnt"
            ),
            params: vec![],
        }
    }

    async fn count_users(state: &EngineState) -> usize {
        let sql = Sql {
            text: r#"SELECT "id" FROM "User""#.to_string(),
            params: vec![],
        };
        state
            .execute_query_on(&sql, "count users", None)
            .await
            .expect("count query should succeed")
            .len()
    }

    #[tokio::test]
    async fn commit_after_timeout_returns_timeout_and_rolls_back() {
        let (state, temp_dir) = sqlite_state(schema_source()).await;
        let tx_id = "commit-timeout".to_string();

        state
            .begin_transaction(tx_id.clone(), Duration::from_millis(10), None)
            .await
            .expect("transaction should start");
        state
            .execute_affected_on(&insert_user_sql("Alice"), "insert user", Some(&tx_id))
            .await
            .expect("insert inside tx should succeed");

        tokio::time::sleep(Duration::from_millis(30)).await;

        let err = state
            .commit_transaction(&tx_id)
            .await
            .expect_err("commit should time out");
        assert!(matches!(err, ProtocolError::TransactionTimeout(_)));
        assert_eq!(count_users(&state).await, 0);

        let lookup_err = state
            .commit_transaction(&tx_id)
            .await
            .expect_err("late commit should keep surfacing timeout");
        assert!(matches!(lookup_err, ProtocolError::TransactionTimeout(_)));

        drop(state);
        drop(temp_dir);
    }

    #[tokio::test]
    async fn rollback_after_timeout_returns_timeout() {
        let (state, temp_dir) = sqlite_state(schema_source()).await;
        let tx_id = "rollback-timeout".to_string();

        state
            .begin_transaction(tx_id.clone(), Duration::from_millis(10), None)
            .await
            .expect("transaction should start");

        tokio::time::sleep(Duration::from_millis(30)).await;

        let err = state
            .rollback_transaction(&tx_id)
            .await
            .expect_err("rollback should time out");
        assert!(matches!(err, ProtocolError::TransactionTimeout(_)));

        let lookup_err = state
            .rollback_transaction(&tx_id)
            .await
            .expect_err("late rollback should keep surfacing timeout");
        assert!(matches!(lookup_err, ProtocolError::TransactionTimeout(_)));

        drop(state);
        drop(temp_dir);
    }

    #[tokio::test]
    async fn reaping_idle_transactions_rolls_back_uncommitted_changes() {
        let (state, temp_dir) = sqlite_state(schema_source()).await;
        let tx_id = "idle-timeout".to_string();

        state
            .begin_transaction(tx_id.clone(), Duration::from_millis(10), None)
            .await
            .expect("transaction should start");
        state
            .execute_affected_on(&insert_user_sql("Bob"), "insert user", Some(&tx_id))
            .await
            .expect("insert inside tx should succeed");

        tokio::time::sleep(Duration::from_millis(30)).await;
        state.reap_expired_transactions().await;

        assert_eq!(count_users(&state).await, 0);
        let err = state
            .execute_affected_on(&insert_user_sql("Carol"), "insert user", Some(&tx_id))
            .await
            .expect_err("expired tx should now reject further work");
        assert!(matches!(err, ProtocolError::TransactionTimeout(_)));

        drop(state);
        drop(temp_dir);
    }

    #[tokio::test]
    async fn registered_external_transaction_exposes_uncommitted_rows_to_engine_queries() {
        let (state, temp_dir) = sqlite_state(schema_source()).await;

        let tx_client = match &state.client {
            DatabaseClient::Sqlite(client) => {
                let sqlx_tx = client
                    .executor()
                    .pool()
                    .begin()
                    .await
                    .expect("sqlite transaction should start");
                let tx_exec = TransactionExecutor::sqlite(sqlx_tx);
                Client::new(SqliteDialect, tx_exec)
            }
            _ => panic!("expected sqlite engine state"),
        };

        let tx_id = "external-tx".to_string();
        state
            .register_external_transaction(tx_id.clone(), tx_client.clone(), Duration::from_secs(5))
            .await;

        state
            .execute_affected_on(&insert_user_sql("Dora"), "insert user", Some(&tx_id))
            .await
            .expect("engine should execute writes on registered external transaction");

        let rows = state
            .execute_query_on(
                &Sql {
                    text: r#"SELECT "name" FROM "User""#.to_string(),
                    params: vec![],
                },
                "select users in tx",
                Some(&tx_id),
            )
            .await
            .expect("engine should read uncommitted rows on registered external transaction");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("name"),
            Some(&Value::String("Dora".to_string())),
        );
        assert_eq!(count_users(&state).await, 0);

        state.unregister_external_transaction(&tx_id).await;
        tx_client
            .executor()
            .rollback()
            .await
            .expect("rollback should succeed");

        drop(state);
        drop(temp_dir);
    }

    #[tokio::test]
    async fn long_running_transaction_query_does_not_block_new_transactions() {
        let (state, temp_dir) = sqlite_state(schema_source()).await;
        let state = Arc::new(state);
        let tx_id = "long-query".to_string();

        state
            .begin_transaction(tx_id.clone(), Duration::from_secs(30), None)
            .await
            .expect("transaction should start");

        let query_state = Arc::clone(&state);
        let query_tx_id = tx_id.clone();
        let long_query = tokio::spawn(async move {
            query_state
                .execute_query_on(
                    &long_running_sql(5_000_000),
                    "long-running transaction query",
                    Some(&query_tx_id),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;

        let second_tx_id = "independent-tx".to_string();
        tokio::time::timeout(
            Duration::from_millis(100),
            state.begin_transaction(second_tx_id.clone(), Duration::from_secs(5), None),
        )
        .await
        .expect("independent transaction start should not wait on another transaction query")
        .expect("second transaction should start successfully");

        state
            .rollback_transaction(&second_tx_id)
            .await
            .expect("second transaction rollback should succeed");

        let rows = long_query
            .await
            .expect("long query task should join")
            .expect("long query should succeed");
        assert_eq!(rows.len(), 1);

        state
            .rollback_transaction(&tx_id)
            .await
            .expect("long-query transaction rollback should succeed");

        drop(state);
        drop(temp_dir);
    }

    #[tokio::test]
    async fn find_unique_typed_caches_simple_eq_plans_per_shape() {
        use nautilus_core::{Expr, FindUniqueArgs};

        let (state, temp_dir) = sqlite_state(schema_source()).await;
        for name in ["Alice", "Bob"] {
            state
                .execute_affected_on(&insert_user_sql(name), "insert user", None)
                .await
                .expect("seed insert should succeed");
        }

        assert_eq!(state.plan_cache().find_unique_len(), 0);

        let by_id_args =
            FindUniqueArgs::new(Expr::column("User__id").eq(Expr::param(Value::I64(1))));
        let rows = crate::handlers::handle_find_unique_typed(&state, "User", &by_id_args, None)
            .await
            .expect("first findUnique should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(state.plan_cache().find_unique_len(), 1);

        let by_id_other =
            FindUniqueArgs::new(Expr::column("User__id").eq(Expr::param(Value::I64(2))));
        let rows = crate::handlers::handle_find_unique_typed(&state, "User", &by_id_other, None)
            .await
            .expect("second findUnique should reuse the cached plan");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            state.plan_cache().find_unique_len(),
            1,
            "identical filter shape should not grow the cache",
        );

        let by_name_args = FindUniqueArgs::new(
            Expr::column("User__name").eq(Expr::param(Value::String("Alice".to_string()))),
        );
        let rows = crate::handlers::handle_find_unique_typed(&state, "User", &by_name_args, None)
            .await
            .expect("differently shaped findUnique should also succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(state.plan_cache().find_unique_len(), 2);

        let by_id_gt = FindUniqueArgs::new(Expr::column("User__id").gt(Expr::param(Value::I64(0))));
        let _ = crate::handlers::handle_find_unique_typed(&state, "User", &by_id_gt, None)
            .await
            .expect("non-cacheable filter should still execute");
        assert_eq!(state.plan_cache().find_unique_len(), 2);

        drop(state);
        drop(temp_dir);
    }

    #[tokio::test]
    async fn find_many_typed_caches_parametric_plans_per_shape() {
        use nautilus_core::{Expr, FindManyArgs};

        let (state, temp_dir) = sqlite_state(schema_source()).await;
        for name in ["Alice", "Bob", "Cara"] {
            state
                .execute_affected_on(&insert_user_sql(name), "insert user", None)
                .await
                .expect("seed insert should succeed");
        }

        assert_eq!(state.plan_cache().find_many_len(), 0);

        let first = FindManyArgs {
            where_: Some(Expr::column("User__id").gt(Expr::param(Value::I64(0)))),
            take: Some(2),
            ..Default::default()
        };
        let rows = crate::handlers::handle_find_many_typed(&state, "User", &first, None)
            .await
            .expect("first findMany should succeed");
        assert_eq!(rows.len(), 2);
        assert_eq!(state.plan_cache().find_many_len(), 1);

        let other_value = FindManyArgs {
            where_: Some(Expr::column("User__id").gt(Expr::param(Value::I64(2)))),
            take: Some(2),
            ..Default::default()
        };
        let rows = crate::handlers::handle_find_many_typed(&state, "User", &other_value, None)
            .await
            .expect("second findMany should reuse the cached plan");
        assert_eq!(
            rows.len(),
            1,
            "replayed plan must bind the fresh parameter value"
        );
        assert_eq!(state.plan_cache().find_many_len(), 1);

        let other_take = FindManyArgs {
            where_: Some(Expr::column("User__id").gt(Expr::param(Value::I64(0)))),
            take: Some(1),
            ..Default::default()
        };
        let rows = crate::handlers::handle_find_many_typed(&state, "User", &other_take, None)
            .await
            .expect("different take should also succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(state.plan_cache().find_many_len(), 2);

        let in_list = FindManyArgs {
            where_: Some(
                Expr::column("User__id")
                    .in_list(vec![Expr::param(Value::I64(1)), Expr::param(Value::I64(2))]),
            ),
            ..Default::default()
        };
        let rows = crate::handlers::handle_find_many_typed(&state, "User", &in_list, None)
            .await
            .expect("non-cacheable filter should still execute");
        assert_eq!(rows.len(), 2);
        assert_eq!(state.plan_cache().find_many_len(), 2);

        drop(state);
        drop(temp_dir);
    }
}
