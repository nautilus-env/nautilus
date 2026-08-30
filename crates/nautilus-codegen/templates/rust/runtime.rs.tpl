use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::ConnectorPoolOptions;
use nautilus_connector::{
    Client as ConnectorClient, Executor, MysqlExecutor, PgExecutor, SqliteExecutor,
    TransactionExecutor, TransactionOptions,
};
use nautilus_core::{Error, FindManyArgs, Value};
use nautilus_dialect::Dialect;
use nautilus_engine::{handlers, EngineState};
use nautilus_protocol::{
    AggregateParams, CountParams, CreateManyParams, CreateParams, DeleteManyParams, ExplainResult,
    GroupByParams, ProtocolError, UpdateManyParams, UpdateParams, UpsertParams, PROTOCOL_VERSION,
};
use nautilus_schema::validate_schema_source;
use serde_json::Value as JsonValue;
use tokio::sync::OnceCell;

static GENERATED_SCHEMA_IR: OnceLock<Arc<nautilus_schema::ir::SchemaIr>> = OnceLock::new();

/// Controls when the generated Rust client routes queries through the embedded engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    /// Use the direct connector path for simple CRUD and reserve the engine for
    /// includes and aggregate-style operations that still need engine semantics.
    Auto,
    /// Always route supported operations through the embedded engine.
    Always,
    /// Never initialize or use the embedded engine.
    Never,
}

impl EngineMode {
    fn allows_engine(self) -> bool {
        !matches!(self, Self::Never)
    }

    fn uses_engine_for_simple_crud(self) -> bool {
        matches!(self, Self::Always)
    }
}

struct EmbeddedTransactionContext {
    client: ConnectorClient<TransactionExecutor>,
    timeout: Duration,
    registration: OnceCell<()>,
}

impl EmbeddedTransactionContext {
    fn new(client: ConnectorClient<TransactionExecutor>, timeout: Duration) -> Self {
        Self {
            client,
            timeout,
            registration: OnceCell::new(),
        }
    }

    async fn ensure_registered(
        &self,
        state: &EngineState,
        transaction_id: &str,
    ) -> nautilus_core::Result<()> {
        let client = self.client.clone();
        let timeout = self.timeout;
        let transaction_id = transaction_id.to_string();

        self.registration
            .get_or_try_init(|| async move {
                state
                    .register_external_transaction(transaction_id, client, timeout)
                    .await;
                Ok::<(), Error>(())
            })
            .await?;

        Ok(())
    }
}

pub struct Client<E: Executor> {
    inner: ConnectorClient<E>,
    database_url: Arc<String>,
    engine_state: Arc<OnceCell<Arc<EngineState>>>,
    pool_options: ConnectorPoolOptions,
    engine_mode: EngineMode,
    transaction_id: Option<String>,
    embedded_transaction: Option<Arc<EmbeddedTransactionContext>>,
    events: crate::EventRegistry,
}

impl<E> Clone for Client<E>
where
    E: Executor,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            database_url: Arc::clone(&self.database_url),
            engine_state: Arc::clone(&self.engine_state),
            pool_options: self.pool_options,
            engine_mode: self.engine_mode,
            transaction_id: self.transaction_id.clone(),
            embedded_transaction: self.embedded_transaction.clone(),
            events: self.events.clone(),
        }
    }
}

impl<E> Client<E>
where
    E: Executor,
{
    pub fn new<D>(dialect: D, executor: E) -> Self
    where
        D: Dialect + Send + Sync + 'static,
    {
        Self {
            inner: ConnectorClient::new(dialect, executor),
            database_url: Arc::new(String::new()),
            engine_state: Arc::new(OnceCell::new()),
            pool_options: ConnectorPoolOptions::default(),
            engine_mode: EngineMode::Never,
            transaction_id: None,
            embedded_transaction: None,
            events: crate::EventRegistry::default(),
        }
    }

    fn from_connector(
        inner: ConnectorClient<E>,
        database_url: Arc<String>,
        engine_state: Arc<OnceCell<Arc<EngineState>>>,
        pool_options: ConnectorPoolOptions,
        engine_mode: EngineMode,
        transaction_id: Option<String>,
        embedded_transaction: Option<Arc<EmbeddedTransactionContext>>,
        events: crate::EventRegistry,
    ) -> Self {
        Self {
            inner,
            database_url,
            engine_state,
            pool_options,
            engine_mode,
            transaction_id,
            embedded_transaction,
            events,
        }
    }

    pub fn dialect(&self) -> &(dyn Dialect + Send + Sync) {
        self.inner.dialect()
    }

    pub fn executor(&self) -> &E {
        self.inner.executor()
    }

    pub fn events(&self) -> &crate::EventRegistry {
        &self.events
    }

    /// Return the current embedded-engine routing policy.
    pub fn engine_mode(&self) -> EngineMode {
        self.engine_mode
    }

    /// Update the embedded-engine routing policy in place.
    pub fn set_engine_mode(&mut self, engine_mode: EngineMode) {
        self.engine_mode = engine_mode;
    }

    /// Return a clone of this client with a different embedded-engine routing policy.
    pub fn with_engine_mode(mut self, engine_mode: EngineMode) -> Self {
        self.engine_mode = engine_mode;
        self
    }

    async fn engine_state(&self) -> nautilus_core::Result<Option<Arc<EngineState>>> {
        if !self.engine_mode.allows_engine() || self.database_url.is_empty() {
            return Ok(None);
        }

        let database_url = Arc::clone(&self.database_url);
        let pool_options = self.pool_options;
        let state = self
            .engine_state
            .get_or_try_init(|| async move {
                let schema = generated_schema_ir()?;
                EngineState::new_with_pool_options(
                    schema.as_ref().clone(),
                    (*database_url).clone(),
                    None,
                    pool_options,
                )
                    .await
                    .map(Arc::new)
                    .map_err(|e| {
                        Error::Other(format!("failed to initialize embedded engine: {}", e))
                    })
            })
            .await?;

        if let (Some(transaction_id), Some(embedded_transaction)) = (
            self.transaction_id.as_deref(),
            self.embedded_transaction.as_ref(),
        ) {
            embedded_transaction
                .ensure_registered(state.as_ref(), transaction_id)
                .await?;
        }

        Ok(Some(Arc::clone(state)))
    }

    pub(crate) fn transaction_id(&self) -> Option<String> {
        self.transaction_id.clone()
    }

    fn should_try_engine_for_find_many(&self, args: &FindManyArgs) -> bool {
        match self.engine_mode {
            EngineMode::Always => true,
            EngineMode::Auto => !args.include.is_empty(),
            EngineMode::Never => false,
        }
    }

    fn should_try_engine_for_find_unique(&self, args: &nautilus_core::FindUniqueArgs) -> bool {
        match self.engine_mode {
            EngineMode::Always => true,
            EngineMode::Auto => !args.include.is_empty(),
            EngineMode::Never => false,
        }
    }

    /// Snapshot the embedded engine's runtime counters.
    ///
    /// Covers the read-plan cache (entries, hits, misses and evictions per
    /// section), the connection pool, the number of open interactive
    /// transactions, and per-method call, error and latency totals. `reset`
    /// zeroes the cumulative counters after reading them, so successive
    /// samples measure the interval between calls rather than the whole
    /// uptime.
    ///
    /// Returns `None` when this client has no engine to ask — the engine mode
    /// is `Never`, or the schema needed to build one is not available.
    pub async fn metrics(
        &self,
        reset: bool,
    ) -> nautilus_core::Result<Option<nautilus_protocol::EngineMetricsResult>> {
        if !self.engine_mode.allows_engine() {
            return Ok(None);
        }

        let Some(state) = self.engine_state().await? else {
            return Ok(None);
        };

        Ok(Some(handlers::engine_metrics_typed(state.as_ref(), reset).await))
    }

    /// Gate for `create`, `update` and `delete`.
    ///
    /// A dialect without `RETURNING` (MySQL) cannot answer these from the
    /// direct connector path at all: the statement reports no rows, so the
    /// caller would get an error on a create and an empty result on an update
    /// or a delete. The engine reads the written rows back on the connection
    /// that wrote them, so it is the only path that can serve them there.
    fn should_try_engine_for_mutation(&self) -> bool {
        self.engine_mode.uses_engine_for_simple_crud() || !self.dialect().supports_returning()
    }

    /// Gate for the operations the direct connector path cannot serve at all —
    /// `count`, `groupBy`, `aggregate`, `updateMany`, `deleteMany`, `explain`.
    /// They only ask whether an engine may be built, not whether this mode
    /// prefers the engine for simple CRUD.
    fn should_try_engine_for_aggregate(&self) -> bool {
        self.engine_mode.allows_engine()
    }
}

impl Client<PgExecutor> {
    pub async fn postgres(url: &str) -> nautilus_connector::ConnectorResult<Self> {
        Self::postgres_with_options(url, ConnectorPoolOptions::default()).await
    }

    pub async fn postgres_with_options(
        url: &str,
        pool_options: ConnectorPoolOptions,
    ) -> nautilus_connector::ConnectorResult<Self> {
        let inner = ConnectorClient::postgres_with_options(url, pool_options).await?;
        Ok(Self::from_connector(
            inner,
            Arc::new(url.to_string()),
            Arc::new(OnceCell::new()),
            pool_options,
            EngineMode::Auto,
            None,
            None,
            crate::EventRegistry::default(),
        ))
    }

    pub async fn transaction<F, Fut, T>(
        &self,
        opts: TransactionOptions,
        f: F,
    ) -> nautilus_connector::ConnectorResult<T>
    where
        F: FnOnce(Client<TransactionExecutor>) -> Fut + Send,
        Fut: Future<Output = nautilus_connector::ConnectorResult<T>> + Send,
        T: Send + 'static,
    {
        let database_url = Arc::clone(&self.database_url);
        let engine_state = Arc::clone(&self.engine_state);
        let pool_options = self.pool_options;
        let engine_mode = self.engine_mode;
        let events = self.events.clone();
        let tx_id = engine_mode
            .allows_engine()
            .then(|| uuid::Uuid::new_v4().to_string());
        let timeout = opts.timeout;
        let tx_id_for_cleanup = tx_id.clone();

        let result = self
            .inner
            .transaction(opts, move |tx| {
                let database_url = Arc::clone(&database_url);
                let engine_state = Arc::clone(&engine_state);
                let events = events.clone();
                let tx_id = tx_id.clone();
                let embedded_transaction = tx_id.as_ref().map(|_| {
                    Arc::new(EmbeddedTransactionContext::new(tx.clone(), timeout))
                });
                async move {
                    let wrapped = Client::from_connector(
                        tx,
                        database_url,
                        engine_state,
                        pool_options,
                        engine_mode,
                        tx_id,
                        embedded_transaction,
                        events,
                    );
                    f(wrapped).await
                }
            })
            .await;

        if let Some(id) = tx_id_for_cleanup.as_deref() {
            if let Some(state) = self.engine_state.get() {
                state.unregister_external_transaction(id).await;
            }
        }

        result
    }
}

impl Client<MysqlExecutor> {
    pub async fn mysql(url: &str) -> nautilus_connector::ConnectorResult<Self> {
        Self::mysql_with_options(url, ConnectorPoolOptions::default()).await
    }

    pub async fn mysql_with_options(
        url: &str,
        pool_options: ConnectorPoolOptions,
    ) -> nautilus_connector::ConnectorResult<Self> {
        let inner = ConnectorClient::mysql_with_options(url, pool_options).await?;
        Ok(Self::from_connector(
            inner,
            Arc::new(url.to_string()),
            Arc::new(OnceCell::new()),
            pool_options,
            EngineMode::Auto,
            None,
            None,
            crate::EventRegistry::default(),
        ))
    }

    pub async fn transaction<F, Fut, T>(
        &self,
        opts: TransactionOptions,
        f: F,
    ) -> nautilus_connector::ConnectorResult<T>
    where
        F: FnOnce(Client<TransactionExecutor>) -> Fut + Send,
        Fut: Future<Output = nautilus_connector::ConnectorResult<T>> + Send,
        T: Send + 'static,
    {
        let database_url = Arc::clone(&self.database_url);
        let engine_state = Arc::clone(&self.engine_state);
        let pool_options = self.pool_options;
        let engine_mode = self.engine_mode;
        let events = self.events.clone();
        let tx_id = engine_mode
            .allows_engine()
            .then(|| uuid::Uuid::new_v4().to_string());
        let timeout = opts.timeout;
        let tx_id_for_cleanup = tx_id.clone();

        let result = self
            .inner
            .transaction(opts, move |tx| {
                let database_url = Arc::clone(&database_url);
                let engine_state = Arc::clone(&engine_state);
                let events = events.clone();
                let tx_id = tx_id.clone();
                let embedded_transaction = tx_id.as_ref().map(|_| {
                    Arc::new(EmbeddedTransactionContext::new(tx.clone(), timeout))
                });
                async move {
                    let wrapped = Client::from_connector(
                        tx,
                        database_url,
                        engine_state,
                        pool_options,
                        engine_mode,
                        tx_id,
                        embedded_transaction,
                        events,
                    );
                    f(wrapped).await
                }
            })
            .await;

        if let Some(id) = tx_id_for_cleanup.as_deref() {
            if let Some(state) = self.engine_state.get() {
                state.unregister_external_transaction(id).await;
            }
        }

        result
    }
}

impl Client<SqliteExecutor> {
    pub async fn sqlite(url: &str) -> nautilus_connector::ConnectorResult<Self> {
        Self::sqlite_with_options(url, ConnectorPoolOptions::default()).await
    }

    pub async fn sqlite_with_options(
        url: &str,
        pool_options: ConnectorPoolOptions,
    ) -> nautilus_connector::ConnectorResult<Self> {
        let inner = ConnectorClient::sqlite_with_options(url, pool_options).await?;
        Ok(Self::from_connector(
            inner,
            Arc::new(url.to_string()),
            Arc::new(OnceCell::new()),
            pool_options,
            EngineMode::Auto,
            None,
            None,
            crate::EventRegistry::default(),
        ))
    }

    pub async fn transaction<F, Fut, T>(
        &self,
        opts: TransactionOptions,
        f: F,
    ) -> nautilus_connector::ConnectorResult<T>
    where
        F: FnOnce(Client<TransactionExecutor>) -> Fut + Send,
        Fut: Future<Output = nautilus_connector::ConnectorResult<T>> + Send,
        T: Send + 'static,
    {
        let database_url = Arc::clone(&self.database_url);
        let engine_state = Arc::clone(&self.engine_state);
        let pool_options = self.pool_options;
        let engine_mode = self.engine_mode;
        let events = self.events.clone();
        let tx_id = engine_mode
            .allows_engine()
            .then(|| uuid::Uuid::new_v4().to_string());
        let timeout = opts.timeout;
        let tx_id_for_cleanup = tx_id.clone();

        let result = self
            .inner
            .transaction(opts, move |tx| {
                let database_url = Arc::clone(&database_url);
                let engine_state = Arc::clone(&engine_state);
                let events = events.clone();
                let tx_id = tx_id.clone();
                let embedded_transaction = tx_id.as_ref().map(|_| {
                    Arc::new(EmbeddedTransactionContext::new(tx.clone(), timeout))
                });
                async move {
                    let wrapped = Client::from_connector(
                        tx,
                        database_url,
                        engine_state,
                        pool_options,
                        engine_mode,
                        tx_id,
                        embedded_transaction,
                        events,
                    );
                    f(wrapped).await
                }
            })
            .await;

        if let Some(id) = tx_id_for_cleanup.as_deref() {
            if let Some(state) = self.engine_state.get() {
                state.unregister_external_transaction(id).await;
            }
        }

        result
    }
}

pub(crate) async fn try_find_many_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    args: &FindManyArgs,
    mut decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<Vec<M>>>
where
    E: Executor,
{
    if !client.should_try_engine_for_find_many(args) {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let transaction_id = client.transaction_id();
    let rows = handlers::handle_find_many_typed(
        state.as_ref(),
        model,
        args,
        transaction_id.as_deref(),
    )
    .await
    .map_err(map_engine_protocol_error)?;

    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(decode_row(row)?);
    }

    Ok(Some(decoded))
}

pub(crate) async fn try_find_unique_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    args: &nautilus_core::FindUniqueArgs,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<M>>
where
    E: Executor,
{
    if !client.should_try_engine_for_find_unique(args) {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let transaction_id = client.transaction_id();
    let rows = handlers::handle_find_unique_typed(
        state.as_ref(),
        model,
        args,
        transaction_id.as_deref(),
    )
    .await
    .map_err(map_engine_protocol_error)?;
    let decoded = decode_engine_rows(rows, decode_row)?;

    Ok(decoded.into_iter().next())
}

pub(crate) async fn try_count_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: Option<JsonValue>,
) -> nautilus_core::Result<Option<i64>>
where
    E: Executor,
{
    if !client.should_try_engine_for_aggregate() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = CountParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        args,
        transaction_id: client.transaction_id(),
    };

    let count = handlers::handle_count_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(Some(count))
}

pub(crate) async fn try_group_by_rows_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: JsonValue,
) -> nautilus_core::Result<Option<Vec<crate::Row>>>
where
    E: Executor,
{
    if !client.should_try_engine_for_aggregate() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = GroupByParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        args: Some(args),
        transaction_id: client.transaction_id(),
    };

    let rows = handlers::handle_group_by_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(Some(rows))
}

pub(crate) async fn try_aggregate_row_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: JsonValue,
) -> nautilus_core::Result<Option<Option<crate::Row>>>
where
    E: Executor,
{
    if !client.should_try_engine_for_aggregate() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = AggregateParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        args: Some(args),
        transaction_id: client.transaction_id(),
    };

    let rows = handlers::handle_aggregate_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(Some(rows.into_iter().next()))
}

pub(crate) async fn try_explain_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: &FindManyArgs,
    analyze: bool,
) -> nautilus_core::Result<Option<ExplainResult>>
where
    E: Executor,
{
    if !client.should_try_engine_for_aggregate() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let transaction_id = client.transaction_id();
    let result = handlers::handle_explain_typed(
        state.as_ref(),
        model,
        args,
        analyze,
        transaction_id.as_deref(),
    )
    .await
    .map_err(map_engine_protocol_error)?;

    Ok(Some(result))
}

pub(crate) async fn try_update_many_via_engine<E>(
    client: &Client<E>,
    model: &str,
    filter: JsonValue,
    data: JsonValue,
) -> nautilus_core::Result<Option<u64>>
where
    E: Executor,
{
    if !client.should_try_engine_for_aggregate() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = UpdateManyParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        filter,
        data,
        transaction_id: client.transaction_id(),
    };

    let count = handlers::handle_update_many_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(Some(count as u64))
}

pub(crate) async fn try_delete_many_via_engine<E>(
    client: &Client<E>,
    model: &str,
    filter: JsonValue,
) -> nautilus_core::Result<Option<u64>>
where
    E: Executor,
{
    if !client.should_try_engine_for_aggregate() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = DeleteManyParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        filter,
        transaction_id: client.transaction_id(),
    };

    let count = handlers::handle_delete_many_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(Some(count as u64))
}

pub(crate) async fn try_create_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    data: JsonValue,
    require_engine: bool,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<M>>
where
    E: Executor,
{
    if !require_engine && !client.should_try_engine_for_mutation() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        if require_engine {
            return Err(nested::writes_need_engine(model));
        }
        return Ok(None);
    };

    let params = CreateParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        data,
        transaction_id: client.transaction_id(),
        return_data: true,
    };

    let rows = handlers::handle_create_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;
    let decoded = decode_engine_rows(rows, decode_row)?;

    Ok(decoded.into_iter().next())
}

pub(crate) async fn try_create_many_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    data: Vec<JsonValue>,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<Vec<M>>>
where
    E: Executor,
{
    if !client.should_try_engine_for_mutation() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = CreateManyParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        data,
        transaction_id: client.transaction_id(),
        return_data: true,
    };

    let rows = handlers::handle_create_many_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    decode_engine_rows(rows, decode_row).map(Some)
}

pub(crate) async fn try_update_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    filter: JsonValue,
    data: JsonValue,
    require_engine: bool,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<Vec<M>>>
where
    E: Executor,
{
    if !require_engine && !client.should_try_engine_for_mutation() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        if require_engine {
            return Err(nested::writes_need_engine(model));
        }
        return Ok(None);
    };

    let params = UpdateParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        filter,
        data,
        transaction_id: client.transaction_id(),
        return_data: true,
    };

    let rows = handlers::handle_update_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    decode_engine_rows(rows, decode_row).map(Some)
}

pub(crate) async fn try_upsert_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    filter: JsonValue,
    create: JsonValue,
    update: JsonValue,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<Vec<M>>>
where
    E: Executor,
{
    if !client.should_try_engine_for_mutation() {
        return Ok(None);
    }

    let Some(state) = client.engine_state().await? else {
        return Ok(None);
    };

    let params = UpsertParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        filter,
        create,
        update,
        transaction_id: client.transaction_id(),
        return_data: true,
    };

    let rows = handlers::handle_upsert_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    decode_engine_rows(rows, decode_row).map(Some)
}

/// Nested-write plumbing shared by the generated create and update inputs.
///
/// Which of these a client reaches for depends on the relations its schema
/// declares — a schema naming only the side that holds a foreign key leaves the
/// list helpers unused — so the module as a whole opts out of the dead-code
/// lint instead of every item carrying its own attribute.
#[allow(dead_code)]
pub(crate) mod nested {
    use nautilus_core::Error;
    use serde_json::Value as JsonValue;

    /// A generated create or update input, as the nested-write helpers read it.
    pub trait NestedInput {
        /// The payload the engine receives for this input.
        fn to_nested_json(&self) -> nautilus_core::Result<JsonValue>;
    }

    /// One entry of a nested-write operation pairing a filter with a payload.
    pub trait NestedEntry {
        /// The entry as the engine receives it.
        fn to_nested_json(&self) -> nautilus_core::Result<JsonValue>;
    }

    /// Connect the record `where_` matches, creating one from `create` when the
    /// filter matches none.
    #[derive(Debug, Clone)]
    pub struct ConnectOrCreate<C> {
        /// Filter identifying the record to connect.
        pub where_: nautilus_core::Expr,
        /// Input used when the filter matches no record.
        pub create: C,
    }

    impl<C: NestedInput> NestedEntry for ConnectOrCreate<C> {
        fn to_nested_json(&self) -> nautilus_core::Result<JsonValue> {
            Ok(serde_json::json!({
                "where": filter_json(&self.where_)?,
                "create": self.create.to_nested_json()?,
            }))
        }
    }

    /// One nested `update` or `updateMany`: `where_` narrows the records reached
    /// through the relation, `data` is applied to them.
    #[derive(Debug, Clone, Default)]
    pub struct NestedUpdate<U> {
        /// Filter narrowing the connected records, or `None` for all of them.
        pub where_: Option<nautilus_core::Expr>,
        /// Assignments applied to the records the filter keeps.
        pub data: U,
    }

    impl<U: NestedInput> NestedEntry for NestedUpdate<U> {
        fn to_nested_json(&self) -> nautilus_core::Result<JsonValue> {
            let mut entry = serde_json::Map::new();
            if let Some(filter) = &self.where_ {
                entry.insert("where".to_string(), filter_json(filter)?);
            }
            entry.insert("data".to_string(), self.data.to_nested_json()?);
            Ok(JsonValue::Object(entry))
        }
    }

    pub fn filter_json(filter: &nautilus_core::Expr) -> nautilus_core::Result<JsonValue> {
        nautilus_core::where_expr_to_protocol_json(filter)
    }

    pub fn filter_array(filters: &[nautilus_core::Expr]) -> nautilus_core::Result<JsonValue> {
        let mut items = Vec::with_capacity(filters.len());
        for filter in filters {
            items.push(filter_json(filter)?);
        }
        Ok(JsonValue::Array(items))
    }

    pub fn input_json<C: NestedInput>(input: &C) -> nautilus_core::Result<JsonValue> {
        input.to_nested_json()
    }

    pub fn data_array<C: NestedInput>(inputs: &[C]) -> nautilus_core::Result<JsonValue> {
        let mut items = Vec::with_capacity(inputs.len());
        for input in inputs {
            items.push(input.to_nested_json()?);
        }
        Ok(JsonValue::Array(items))
    }

    pub fn entry_json<E: NestedEntry>(entry: &E) -> nautilus_core::Result<JsonValue> {
        entry.to_nested_json()
    }

    pub fn entry_array<E: NestedEntry>(entries: &[E]) -> nautilus_core::Result<JsonValue> {
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            items.push(entry.to_nested_json()?);
        }
        Ok(JsonValue::Array(items))
    }

    /// The error a nested write raises when no embedded engine can serve it.
    ///
    /// Relation operations span several statements in one transaction, which the
    /// direct connector path has no plan for; only the engine can run them.
    pub fn writes_need_engine(model: &str) -> Error {
        Error::InvalidQuery(format!(
            "nested writes on '{model}' need the embedded engine, which this client is not configured to use"
        ))
    }
}

fn decode_engine_rows<M>(
    rows: Vec<crate::Row>,
    mut decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Vec<M>> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(decode_row(row)?);
    }

    Ok(decoded)
}

fn map_engine_protocol_error(error: ProtocolError) -> Error {
    match error {
        ProtocolError::RecordNotFound(message) => Error::NotFound(message),
        other => Error::Other(other.to_string()),
    }
}

fn parse_generated_schema() -> nautilus_core::Result<nautilus_schema::ir::SchemaIr> {
    validate_schema_source(crate::SCHEMA_SOURCE)
        .map(|validated| validated.ir)
        .map_err(|e| Error::Other(format!("failed to validate embedded schema: {}", e)))
}

fn generated_schema_ir() -> nautilus_core::Result<Arc<nautilus_schema::ir::SchemaIr>> {
    if let Some(schema) = GENERATED_SCHEMA_IR.get() {
        return Ok(Arc::clone(schema));
    }

    let schema = Arc::new(parse_generated_schema()?);

    match GENERATED_SCHEMA_IR.set(Arc::clone(&schema)) {
        Ok(()) => Ok(schema),
        Err(schema) => Ok(GENERATED_SCHEMA_IR
            .get()
            .map(Arc::clone)
            .unwrap_or(schema)),
    }
}

pub(crate) fn wire_value_to_core_value(name: &str, value: &JsonValue) -> Value {
    if name.ends_with("_json") {
        return Value::Json(value.clone());
    }

    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(v) => Value::Bool(*v),
        JsonValue::Number(v) => {
            if let Some(i) = v.as_i64() {
                Value::I64(i)
            } else if let Some(f) = v.as_f64() {
                Value::F64(f)
            } else {
                Value::Null
            }
        }
        JsonValue::String(v) => Value::String(v.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => Value::Json(value.clone()),
    }
}
