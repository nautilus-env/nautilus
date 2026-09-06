//! Engine state: the schema, connections and caches every handler reads from.

mod database;
mod execution;
#[cfg(test)]
mod tests;
mod transactions;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use nautilus_connector::ConnectorPoolOptions;
use nautilus_dialect::Dialect;
use nautilus_migrate::DatabaseProvider;
use nautilus_protocol::{EngineMetricsResult, ProtocolError};
use nautilus_schema::ir::{ModelIr, SchemaIr};

use crate::filter::RelationMap;
use crate::metadata::ModelMetadata;
use crate::metrics::EngineMetrics;
use crate::plan_cache::PlanCache;
use crate::pool_options::EnginePoolOptions;

use database::{build_client, resolve_database_url};

pub(crate) use database::connector_to_protocol;
pub use database::DatabaseClient;
pub use transactions::{ActiveTransaction, TransactionClient};

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

impl EngineState {
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
        let (dialect, client) = build_client(provider, &resolved_url, pool_options).await?;

        let direct_client = if let Some(raw_direct) = direct_url {
            let resolved_direct = resolve_database_url(&raw_direct)?;
            let (_, dc) = build_client(provider, &resolved_direct, pool_options).await?;
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
}
