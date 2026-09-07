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

/// One field's value in an update, or arithmetic the database applies to it.
///
/// `Set` writes the operand as given. The other four leave the arithmetic to
/// the database — `views = (views + $1)` — so the new value is derived from
/// whatever the row holds when the statement runs and two concurrent updates
/// both land, where a read-modify-write in the client would lose one.
///
/// `T` is the field's own type, so a nullable column can still be set to NULL;
/// `N` is the operand the arithmetic takes, which is never null.
#[derive(Debug, Clone, PartialEq)]
pub enum NumericUpdate<T, N = T> {
    /// Write this value.
    Set(T),
    /// Add to the row's current value.
    Increment(N),
    /// Subtract from the row's current value.
    Decrement(N),
    /// Multiply the row's current value.
    Multiply(N),
    /// Divide the row's current value.
    Divide(N),
}

impl<T, N> From<T> for NumericUpdate<T, N> {
    fn from(value: T) -> Self {
        Self::Set(value)
    }
}

impl<T, N> NumericUpdate<T, N>
where
    T: Clone + Into<nautilus_core::Value>,
    N: Clone + Into<nautilus_core::Value>,
{
    /// The operator name the engine matches, and its operand.
    fn parts(&self) -> (&'static str, nautilus_core::Value) {
        match self {
            Self::Set(value) => ("set", value.clone().into()),
            Self::Increment(value) => ("increment", value.clone().into()),
            Self::Decrement(value) => ("decrement", value.clone().into()),
            Self::Multiply(value) => ("multiply", value.clone().into()),
            Self::Divide(value) => ("divide", value.clone().into()),
        }
    }

    /// The JSON the engine reads: a bare value for `Set`, an operator object
    /// otherwise.
    pub fn to_engine_json(&self) -> JsonValue {
        let (operator, operand) = self.parts();
        if operator == "set" {
            return operand.to_json_plain();
        }
        let mut object = serde_json::Map::with_capacity(1);
        object.insert(operator.to_string(), operand.to_json_plain());
        JsonValue::Object(object)
    }

    /// The same operation as a `SET` right-hand side, for the direct-connector
    /// path that builds its own statement instead of calling the engine.
    pub fn to_assignment(&self, column: &str) -> nautilus_core::Assignment {
        let (operator, operand) = self.parts();
        let op = match operator {
            "increment" => nautilus_core::BinaryOp::Add,
            "decrement" => nautilus_core::BinaryOp::Sub,
            "multiply" => nautilus_core::BinaryOp::Mul,
            "divide" => nautilus_core::BinaryOp::Div,
            _ => return nautilus_core::Assignment::Value(operand),
        };
        nautilus_core::Assignment::Expr(nautilus_core::Expr::Binary {
            left: Box::new(nautilus_core::Expr::column(column)),
            op,
            right: Box::new(nautilus_core::Expr::param(operand)),
        })
    }
}

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

/// One generated operation, named by the paths that can serve it.
///
/// Every call into the embedded engine names one of these, so
/// [`Client::engine_route`] is the single place that chooses between the
/// engine and the direct connector path. A new operation joins an arm here
/// instead of restating the rule at its call site.
///
/// The entry points below spell the same distinction: a `try_…_via_engine`
/// answers `None` when the direct path serves the call, while one named
/// without `try_` has no other path and reports what it needs instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    /// `find_many`, `find_first`, `find_unique`. Both paths read plain rows;
    /// only the engine loads `include` relations, which is why `Auto` sends
    /// those queries there and keeps the rest on the direct path.
    Read { has_include: bool },
    /// `create`, `create_many`, `update`, `upsert` carrying scalar data.
    /// Both paths serve them, so `Auto` keeps them on the direct one.
    Write,
    /// The same writes carrying nested inputs: only the engine plans the
    /// statements a nested write expands into.
    NestedWrite,
    /// An operation the direct connector path does not implement at all.
    EngineOnly(EngineOnly),
}

/// The operations no direct connector path serves, each naming itself in the
/// error a client without an engine returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineOnly {
    /// The `include` argument of a read.
    Include,
    Count,
    Aggregate,
    GroupBy,
    Explain,
    UpdateMany,
    DeleteMany,
}

impl EngineOnly {
    fn requirement(self) -> &'static str {
        match self {
            Self::Include => {
                "include queries require the embedded engine path in the generated Rust client"
            }
            Self::Count => {
                "count queries require the embedded engine path in the generated Rust client"
            }
            Self::Aggregate => {
                "aggregate queries require the embedded engine path in the generated Rust client"
            }
            Self::GroupBy => {
                "groupBy queries require the embedded engine path in the generated Rust client"
            }
            Self::Explain => {
                "explain requires the embedded engine path in the generated Rust client"
            }
            Self::UpdateMany => {
                "updateMany requires the embedded engine path in the generated Rust client"
            }
            Self::DeleteMany => {
                "deleteMany requires the embedded engine path in the generated Rust client"
            }
        }
    }
}

/// The error an engine-only operation returns when this client has no engine.
pub(crate) fn engine_required(operation: EngineOnly) -> Error {
    Error::InvalidQuery(operation.requirement().to_string())
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

    /// Whether `operation` prefers the embedded engine on this client.
    ///
    /// The mode is the caller's standing choice and the operation says what it
    /// can do without an engine; `Auto` leaves on the direct connector path
    /// everything that path serves.
    fn prefers_engine(&self, operation: Operation) -> bool {
        match operation {
            Operation::Read { has_include } => match self.engine_mode {
                EngineMode::Always => true,
                EngineMode::Auto => has_include,
                EngineMode::Never => false,
            },
            // A dialect without `RETURNING` (MySQL) cannot answer a
            // row-returning write from the direct path at all: the statement
            // reports no rows, so the caller would get an error on a create
            // and an empty result on an update. The engine reads the written
            // rows back on the connection that wrote them, so it is the only
            // path that can serve them there.
            Operation::Write => {
                self.engine_mode.uses_engine_for_simple_crud()
                    || !self.dialect().supports_returning()
            }
            Operation::NestedWrite | Operation::EngineOnly(_) => {
                self.engine_mode.allows_engine()
            }
        }
    }

    /// The path `operation` takes: `Some(state)` to run it on the embedded
    /// engine, `None` to run it on the direct connector path.
    async fn engine_route(
        &self,
        operation: Operation,
    ) -> nautilus_core::Result<Option<Arc<EngineState>>> {
        if !self.prefers_engine(operation) {
            return Ok(None);
        }

        self.engine_state().await
    }

    /// The engine an operation the direct path cannot serve has to run on.
    async fn required_engine(
        &self,
        operation: EngineOnly,
    ) -> nautilus_core::Result<Arc<EngineState>> {
        self.engine_route(Operation::EngineOnly(operation))
            .await?
            .ok_or_else(|| engine_required(operation))
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
    let Some(state) = client
        .engine_route(Operation::Read {
            has_include: !args.include.is_empty(),
        })
        .await?
    else {
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
    let Some(state) = client
        .engine_route(Operation::Read {
            has_include: !args.include.is_empty(),
        })
        .await?
    else {
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

pub(crate) async fn count_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: Option<JsonValue>,
) -> nautilus_core::Result<i64>
where
    E: Executor,
{
    let state = client.required_engine(EngineOnly::Count).await?;

    let params = CountParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        args,
        transaction_id: client.transaction_id(),
    };

    handlers::handle_count_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)
}

pub(crate) async fn group_by_rows_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: JsonValue,
) -> nautilus_core::Result<Vec<crate::Row>>
where
    E: Executor,
{
    let state = client.required_engine(EngineOnly::GroupBy).await?;

    let params = GroupByParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        args: Some(args),
        transaction_id: client.transaction_id(),
    };

    handlers::handle_group_by_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)
}

pub(crate) async fn aggregate_row_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: JsonValue,
) -> nautilus_core::Result<Option<crate::Row>>
where
    E: Executor,
{
    let state = client.required_engine(EngineOnly::Aggregate).await?;

    let params = AggregateParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        args: Some(args),
        transaction_id: client.transaction_id(),
    };

    let rows = handlers::handle_aggregate_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(rows.into_iter().next())
}

pub(crate) async fn explain_via_engine<E>(
    client: &Client<E>,
    model: &str,
    args: &FindManyArgs,
    analyze: bool,
) -> nautilus_core::Result<ExplainResult>
where
    E: Executor,
{
    let state = client.required_engine(EngineOnly::Explain).await?;

    let transaction_id = client.transaction_id();
    handlers::handle_explain_typed(
        state.as_ref(),
        model,
        args,
        analyze,
        transaction_id.as_deref(),
    )
    .await
    .map_err(map_engine_protocol_error)
}

pub(crate) async fn update_many_via_engine<E>(
    client: &Client<E>,
    model: &str,
    filter: JsonValue,
    data: JsonValue,
) -> nautilus_core::Result<u64>
where
    E: Executor,
{
    let state = client.required_engine(EngineOnly::UpdateMany).await?;

    let params = UpdateManyParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        filter,
        data,
        transaction_id: client.transaction_id(),
        return_data: false,
    };

    let count = handlers::handle_update_many_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(count as u64)
}

pub(crate) async fn delete_many_via_engine<E>(
    client: &Client<E>,
    model: &str,
    filter: JsonValue,
) -> nautilus_core::Result<u64>
where
    E: Executor,
{
    let state = client.required_engine(EngineOnly::DeleteMany).await?;

    let params = DeleteManyParams {
        protocol_version: PROTOCOL_VERSION,
        model: model.to_string(),
        filter,
        transaction_id: client.transaction_id(),
        return_data: false,
    };

    let count = handlers::handle_delete_many_typed(state.as_ref(), params)
        .await
        .map_err(map_engine_protocol_error)?;

    Ok(count as u64)
}

/// The engine a row-returning write runs on, or `None` to run it on the
/// direct connector path.
///
/// A write carrying nested inputs has no direct path, so a client that cannot
/// reach an engine gets the error naming the model instead of a fallback.
async fn write_engine<E>(
    client: &Client<E>,
    model: &str,
    has_nested_writes: bool,
) -> nautilus_core::Result<Option<Arc<EngineState>>>
where
    E: Executor,
{
    if !has_nested_writes {
        return client.engine_route(Operation::Write).await;
    }

    client
        .engine_route(Operation::NestedWrite)
        .await?
        .ok_or_else(|| nested::writes_need_engine(model))
        .map(Some)
}

pub(crate) async fn try_create_via_engine<E, M>(
    client: &Client<E>,
    model: &str,
    data: JsonValue,
    has_nested_writes: bool,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<M>>
where
    E: Executor,
{
    let Some(state) = write_engine(client, model, has_nested_writes).await? else {
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
    let Some(state) = client.engine_route(Operation::Write).await? else {
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
    has_nested_writes: bool,
    decode_row: impl FnMut(crate::Row) -> nautilus_core::Result<M>,
) -> nautilus_core::Result<Option<Vec<M>>>
where
    E: Executor,
{
    let Some(state) = write_engine(client, model, has_nested_writes).await? else {
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
    let Some(state) = client.engine_route(Operation::Write).await? else {
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

pub(crate) fn serialize_cursor_for_engine(
    cursor: &std::collections::HashMap<String, nautilus_core::Value>,
) -> serde_json::Value {
    serde_json::Value::Object(
        cursor
            .iter()
            .map(|(key, value)| (key.clone(), value.to_json_plain()))
            .collect(),
    )
}

pub(crate) fn decode_optional_group_by_row_value<T>(
    row: &crate::Row,
    key: &str,
) -> nautilus_core::Result<Option<T>>
where
    T: nautilus_core::FromValue,
{
    match row.get(key) {
        Some(nautilus_core::Value::Null) | None => Ok(None),
        Some(value) => <T as nautilus_core::FromValue>::from_value(value).map(Some),
    }
}

pub(crate) fn decode_optional_group_by_object<'a>(
    row: &'a crate::Row,
    key: &str,
) -> nautilus_core::Result<Option<&'a serde_json::Map<String, serde_json::Value>>> {
    match row.get(key) {
        Some(nautilus_core::Value::Json(serde_json::Value::Object(obj))) => Ok(Some(obj)),
        Some(nautilus_core::Value::Null) | None => Ok(None),
        Some(other) => Err(nautilus_core::Error::TypeError(format!(
            "expected object-valued aggregate '{}' in groupBy output, got {:?}",
            key, other
        ))),
    }
}

/// Decode one `_avg` / `_sum` entry of an aggregate result.
///
/// PostgreSQL computes `AVG` (and `SUM` over exact numerics) as `numeric`,
/// which the engine puts on the wire as a JSON string so no precision is lost
/// in transit; SQLite and MySQL send a plain JSON number. Both spellings decode
/// into the same Rust type here.
pub(crate) fn decode_optional_numeric_aggregate<T>(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> nautilus_core::Result<Option<T>>
where
    T: nautilus_core::FromValue + std::str::FromStr,
{
    match obj.get(key) {
        Some(serde_json::Value::String(text)) => text.parse::<T>().map(Some).map_err(|_| {
            nautilus_core::Error::TypeError(format!(
                "aggregate '{}' is not a number: {}",
                key, text
            ))
        }),
        _ => decode_optional_group_by_json_value(obj, key),
    }
}

pub(crate) fn decode_optional_group_by_json_value<T>(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> nautilus_core::Result<Option<T>>
where
    T: nautilus_core::FromValue,
{
    match obj.get(key) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => <T as nautilus_core::FromValue>::from_value_owned(
            wire_value_to_core_value(key, value),
        )
        .map(Some),
    }
}

/// The engine's JSON form of a write filter, or `None` when the filter uses
/// something the protocol cannot express.
///
/// A filter the protocol rejects is not an error: it means the write has to
/// take the direct connector path, which renders the expression as SQL.
pub(crate) fn serialize_update_filter_for_engine(
    filter: Option<&nautilus_core::Expr>,
) -> nautilus_core::Result<Option<JsonValue>> {
    let filter_json = match filter {
        Some(expr) => match nautilus_core::where_expr_to_protocol_json(expr) {
            Ok(value) => value,
            Err(Error::InvalidQuery(_)) => return Ok(None),
            Err(err) => return Err(err),
        },
        None => return Ok(None),
    };

    Ok(Some(filter_json))
}

/// Whether `filter` pins at most one row.
///
/// `field_name` maps a column as a filter may spell it to the model's field
/// name and `constraints` lists the field sets that identify a row: both are
/// facts only the model has. Walking the expression is the same everywhere,
/// so a filter qualifies when it is a conjunction of equalities on known
/// columns covering exactly one of those sets.
pub(crate) fn is_single_record_filter(
    filter: &nautilus_core::Expr,
    field_name: fn(&str) -> Option<&'static str>,
    constraints: &[&[&str]],
) -> bool {
    let mut fields = std::collections::HashSet::new();
    if !collect_single_record_filter_fields(filter, field_name, &mut fields) {
        return false;
    }

    constraints.iter().any(|constraint| {
        fields.len() == constraint.len() && constraint.iter().all(|field| fields.contains(field))
    })
}

fn collect_single_record_filter_fields(
    expr: &nautilus_core::Expr,
    field_name: fn(&str) -> Option<&'static str>,
    fields: &mut std::collections::HashSet<&'static str>,
) -> bool {
    use nautilus_core::{BinaryOp, Expr};

    match expr {
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
        } => {
            collect_single_record_filter_fields(left, field_name, fields)
                && collect_single_record_filter_fields(right, field_name, fields)
        }
        Expr::Binary {
            left,
            op: BinaryOp::Eq,
            right,
        } => match (&**left, &**right) {
            (Expr::Column(column), Expr::Param(_)) | (Expr::Param(_), Expr::Column(column)) => {
                let Some(field) = field_name(column) else {
                    return false;
                };
                fields.insert(field)
            }
            _ => false,
        },
        _ => false,
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
