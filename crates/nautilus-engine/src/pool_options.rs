use std::time::Duration;

use nautilus_connector::ConnectorPoolOptions;

/// In-flight requests allowed per pooled connection when
/// `max_concurrent_requests` is not set explicitly.
const REQUESTS_PER_POOLED_CONNECTION: usize = 4;

/// Pool size assumed when the datasource leaves `max_connections` unset;
/// matches the largest connector default (PostgreSQL).
const ASSUMED_POOL_CONNECTIONS: usize = 10;

/// Engine-level runtime overrides exposed by the subprocess clients.
///
/// This keeps the engine CLI and generated non-Rust clients decoupled from the
/// connector crate while still mapping 1:1 to the underlying pool controls.
/// [`Self::max_concurrent_requests`] is the one knob that has no connector
/// counterpart: it bounds the engine transport rather than the pool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnginePoolOptions {
    max_connections: Option<u32>,
    min_connections: Option<u32>,
    acquire_timeout: Option<Duration>,
    idle_timeout: Option<Option<Duration>>,
    test_before_acquire: Option<bool>,
    statement_cache_capacity: Option<usize>,
    statement_timeout: Option<Duration>,
    max_concurrent_requests: Option<usize>,
}

impl EnginePoolOptions {
    /// Create an empty set of engine pool overrides.
    pub const fn new() -> Self {
        Self {
            max_connections: None,
            min_connections: None,
            acquire_timeout: None,
            idle_timeout: None,
            test_before_acquire: None,
            statement_cache_capacity: None,
            statement_timeout: None,
            max_concurrent_requests: None,
        }
    }

    /// Override the maximum number of pooled connections.
    pub fn max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = Some(max_connections);
        self
    }

    /// Override the minimum number of pooled connections kept warm.
    pub fn min_connections(mut self, min_connections: u32) -> Self {
        self.min_connections = Some(min_connections);
        self
    }

    /// Override the maximum time spent waiting for a pooled connection.
    pub fn acquire_timeout(mut self, acquire_timeout: Duration) -> Self {
        self.acquire_timeout = Some(acquire_timeout);
        self
    }

    /// Override the maximum time spent waiting for a pooled connection in milliseconds.
    pub fn acquire_timeout_ms(self, acquire_timeout_ms: u64) -> Self {
        self.acquire_timeout(Duration::from_millis(acquire_timeout_ms))
    }

    /// Override the maximum idle duration for pooled connections.
    ///
    /// Pass `None` to disable idle reaping entirely.
    pub fn idle_timeout(mut self, idle_timeout: impl Into<Option<Duration>>) -> Self {
        self.idle_timeout = Some(idle_timeout.into());
        self
    }

    /// Override the maximum idle duration for pooled connections in milliseconds.
    pub fn idle_timeout_ms(self, idle_timeout_ms: u64) -> Self {
        self.idle_timeout(Duration::from_millis(idle_timeout_ms))
    }

    /// Disable idle reaping for pooled connections.
    pub fn disable_idle_timeout(self) -> Self {
        self.idle_timeout(None::<Duration>)
    }

    /// Override whether pooled connections are pinged before acquisition.
    pub fn test_before_acquire(mut self, test_before_acquire: bool) -> Self {
        self.test_before_acquire = Some(test_before_acquire);
        self
    }

    /// Override the per-connection statement cache capacity used by sqlx.
    ///
    /// Set this to `0` to disable statement caching entirely.
    pub fn statement_cache_capacity(mut self, statement_cache_capacity: usize) -> Self {
        self.statement_cache_capacity = Some(statement_cache_capacity);
        self
    }

    /// Cap how long the database runs a single statement before aborting it.
    ///
    /// Unlike `request.cancel`, which only stops the engine from waiting, this
    /// limit is enforced by the server. See
    /// [`ConnectorPoolOptions::statement_timeout`].
    pub fn statement_timeout(mut self, statement_timeout: Duration) -> Self {
        self.statement_timeout = Some(statement_timeout);
        self
    }

    /// Cap the server-side statement duration in milliseconds.
    pub fn statement_timeout_ms(self, statement_timeout_ms: u64) -> Self {
        self.statement_timeout(Duration::from_millis(statement_timeout_ms))
    }

    /// Override how many requests the transport handles concurrently.
    ///
    /// Values below `1` are clamped to `1`. When left unset the limit is
    /// derived from the pool size — see
    /// [`resolved_max_concurrent_requests`](Self::resolved_max_concurrent_requests).
    pub fn max_concurrent_requests(mut self, max_concurrent_requests: usize) -> Self {
        self.max_concurrent_requests = Some(max_concurrent_requests.max(1));
        self
    }

    /// Return the configured maximum-connection override, if any.
    pub const fn get_max_connections(&self) -> Option<u32> {
        self.max_connections
    }

    /// Return the configured minimum-connection override, if any.
    pub const fn get_min_connections(&self) -> Option<u32> {
        self.min_connections
    }

    /// Return the configured acquire-timeout override, if any.
    pub const fn get_acquire_timeout(&self) -> Option<Duration> {
        self.acquire_timeout
    }

    /// Return the configured idle-timeout override, if any.
    pub const fn get_idle_timeout(&self) -> Option<Option<Duration>> {
        self.idle_timeout
    }

    /// Return the configured `test_before_acquire` override, if any.
    pub const fn get_test_before_acquire(&self) -> Option<bool> {
        self.test_before_acquire
    }

    /// Return the configured statement-cache-capacity override, if any.
    pub const fn get_statement_cache_capacity(&self) -> Option<usize> {
        self.statement_cache_capacity
    }

    /// Return the configured server-side statement-timeout override, if any.
    pub const fn get_statement_timeout(&self) -> Option<Duration> {
        self.statement_timeout
    }

    /// Return the configured concurrent-request override, if any.
    pub const fn get_max_concurrent_requests(&self) -> Option<usize> {
        self.max_concurrent_requests
    }

    /// Number of requests the transport may handle concurrently.
    ///
    /// Without an explicit override the limit tracks the pool size: past a few
    /// in-flight requests per connection the extra tasks only queue on the
    /// pool, while their buffers and response slots keep accumulating in the
    /// engine process.
    pub fn resolved_max_concurrent_requests(&self) -> usize {
        self.max_concurrent_requests
            .unwrap_or_else(|| {
                let connections = self
                    .max_connections
                    .map_or(ASSUMED_POOL_CONNECTIONS, |max| max as usize);
                connections.saturating_mul(REQUESTS_PER_POOLED_CONNECTION)
            })
            .max(1)
    }

    /// Convert engine-level overrides into connector-level pool options.
    pub fn to_connector_pool_options(self) -> ConnectorPoolOptions {
        let mut options = ConnectorPoolOptions::new();
        if let Some(max_connections) = self.max_connections {
            options = options.max_connections(max_connections);
        }
        if let Some(min_connections) = self.min_connections {
            options = options.min_connections(min_connections);
        }
        if let Some(acquire_timeout) = self.acquire_timeout {
            options = options.acquire_timeout(acquire_timeout);
        }
        if let Some(idle_timeout) = self.idle_timeout {
            options = options.idle_timeout(idle_timeout);
        }
        if let Some(test_before_acquire) = self.test_before_acquire {
            options = options.test_before_acquire(test_before_acquire);
        }
        if let Some(statement_cache_capacity) = self.statement_cache_capacity {
            options = options.statement_cache_capacity(statement_cache_capacity);
        }
        if let Some(statement_timeout) = self.statement_timeout {
            options = options.statement_timeout(statement_timeout);
        }
        options
    }

    /// Convert connector-level pool overrides into engine-level pool options.
    pub fn from_connector_pool_options(options: ConnectorPoolOptions) -> Self {
        Self {
            max_connections: options.get_max_connections(),
            min_connections: options.get_min_connections(),
            acquire_timeout: options.get_acquire_timeout(),
            idle_timeout: options.get_idle_timeout(),
            test_before_acquire: options.get_test_before_acquire(),
            statement_cache_capacity: options.get_statement_cache_capacity(),
            statement_timeout: options.get_statement_timeout(),
            max_concurrent_requests: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EnginePoolOptions;
    use std::time::Duration;

    #[test]
    fn converts_to_connector_pool_options() {
        let engine = EnginePoolOptions::new()
            .max_connections(24)
            .min_connections(4)
            .acquire_timeout(Duration::from_secs(3))
            .disable_idle_timeout()
            .test_before_acquire(false)
            .statement_cache_capacity(12)
            .statement_timeout_ms(2_500);

        let connector = engine.to_connector_pool_options();

        assert_eq!(connector.get_max_connections(), Some(24));
        assert_eq!(connector.get_min_connections(), Some(4));
        assert_eq!(
            connector.get_acquire_timeout(),
            Some(Duration::from_secs(3))
        );
        assert_eq!(connector.get_idle_timeout(), Some(None));
        assert_eq!(connector.get_test_before_acquire(), Some(false));
        assert_eq!(connector.get_statement_cache_capacity(), Some(12));
        assert_eq!(
            connector.get_statement_timeout(),
            Some(Duration::from_millis(2_500))
        );
    }

    #[test]
    fn max_concurrent_requests_defaults_to_a_multiple_of_the_pool() {
        assert_eq!(
            EnginePoolOptions::new().resolved_max_concurrent_requests(),
            40
        );
        assert_eq!(
            EnginePoolOptions::new()
                .max_connections(25)
                .resolved_max_concurrent_requests(),
            100
        );
    }

    #[test]
    fn max_concurrent_requests_override_wins_and_clamps_to_one() {
        let options = EnginePoolOptions::new()
            .max_connections(25)
            .max_concurrent_requests(7);
        assert_eq!(options.resolved_max_concurrent_requests(), 7);

        let clamped = EnginePoolOptions::new().max_concurrent_requests(0);
        assert_eq!(clamped.resolved_max_concurrent_requests(), 1);
    }

    #[test]
    fn round_trips_connector_pool_options() {
        let connector = nautilus_connector::ConnectorPoolOptions::new()
            .max_connections(16)
            .idle_timeout(Duration::from_secs(30))
            .test_before_acquire(true)
            .statement_cache_capacity(4);

        let engine = EnginePoolOptions::from_connector_pool_options(connector);

        assert_eq!(engine.get_max_connections(), Some(16));
        assert_eq!(
            engine.get_idle_timeout(),
            Some(Some(Duration::from_secs(30)))
        );
        assert_eq!(engine.get_test_before_acquire(), Some(true));
        assert_eq!(engine.get_statement_cache_capacity(), Some(4));
    }
}
