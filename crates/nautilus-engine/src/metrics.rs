//! Runtime counters served by `engine.metrics`.
//!
//! Everything here is cumulative since the engine state was built (or since the
//! last `reset`), so a caller samples twice and subtracts to obtain a rate. The
//! counters are advisory: they are read and written with relaxed ordering and
//! never coordinate with each other, so a snapshot taken under load can mix
//! values from adjacent instants.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use nautilus_protocol::MethodMetrics;

/// Counters for one JSON-RPC method.
#[derive(Debug, Default)]
struct MethodCounters {
    calls: AtomicU64,
    errors: AtomicU64,
    total_ms: AtomicU64,
    max_ms: AtomicU64,
}

impl MethodCounters {
    fn record(&self, elapsed: Duration, failed: bool) {
        let elapsed_ms = elapsed.as_millis() as u64;
        self.calls.fetch_add(1, Ordering::Relaxed);
        if failed {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        self.total_ms.fetch_add(elapsed_ms, Ordering::Relaxed);
        self.max_ms.fetch_max(elapsed_ms, Ordering::Relaxed);
    }

    fn snapshot(&self, method: String) -> MethodMetrics {
        MethodMetrics {
            method,
            calls: self.calls.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            total_ms: self.total_ms.load(Ordering::Relaxed),
            max_ms: self.max_ms.load(Ordering::Relaxed),
        }
    }

    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.errors.store(0, Ordering::Relaxed);
        self.total_ms.store(0, Ordering::Relaxed);
        self.max_ms.store(0, Ordering::Relaxed);
    }
}

/// Per-method counters plus the engine's start instant.
///
/// The map is keyed by method name and only ever grows by the size of the
/// dispatch table, so the write lock is taken once per distinct method over the
/// lifetime of the process; every subsequent call needs the read lock alone and
/// increments atomics under it.
#[derive(Debug)]
pub(crate) struct EngineMetrics {
    started_at: Instant,
    methods: RwLock<HashMap<String, Arc<MethodCounters>>>,
}

impl Default for EngineMetrics {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            methods: RwLock::new(HashMap::new()),
        }
    }
}

impl EngineMetrics {
    /// Record one dispatched request. Batch members are counted individually,
    /// under their own method names.
    pub(crate) fn record(&self, method: &str, elapsed: Duration, failed: bool) {
        if let Some(counters) = self
            .methods
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(method)
        {
            counters.record(elapsed, failed);
            return;
        }

        let counters = Arc::clone(
            self.methods
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(method.to_string())
                .or_default(),
        );
        counters.record(elapsed, failed);
    }

    /// Seconds elapsed since the engine state was built.
    pub(crate) fn uptime(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Per-method counters, sorted by method name so successive snapshots line
    /// up positionally for a diffing caller.
    pub(crate) fn method_snapshot(&self) -> Vec<MethodMetrics> {
        let guard = self.methods.read().unwrap_or_else(PoisonError::into_inner);
        let mut snapshot: Vec<MethodMetrics> = guard
            .iter()
            .map(|(method, counters)| counters.snapshot(method.clone()))
            .collect();
        snapshot.sort_by(|left, right| left.method.cmp(&right.method));
        snapshot
    }

    /// Zero every method's counters, keeping the method keys.
    pub(crate) fn reset(&self) {
        for counters in self
            .methods
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            counters.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_calls_errors_and_latency_per_method() {
        let metrics = EngineMetrics::default();
        metrics.record("query.findMany", Duration::from_millis(10), false);
        metrics.record("query.findMany", Duration::from_millis(30), true);
        metrics.record("query.count", Duration::from_millis(1), false);

        let snapshot = metrics.method_snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].method, "query.count");

        let find_many = &snapshot[1];
        assert_eq!(find_many.method, "query.findMany");
        assert_eq!(find_many.calls, 2);
        assert_eq!(find_many.errors, 1);
        assert_eq!(find_many.total_ms, 40);
        assert_eq!(find_many.max_ms, 30);
    }

    #[test]
    fn reset_zeroes_counters_but_keeps_methods() {
        let metrics = EngineMetrics::default();
        metrics.record("query.count", Duration::from_millis(5), false);
        metrics.reset();

        let snapshot = metrics.method_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].calls, 0);
        assert_eq!(snapshot[0].total_ms, 0);
        assert_eq!(snapshot[0].max_ms, 0);
    }
}
