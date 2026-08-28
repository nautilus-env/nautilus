//! Cached query plans for the hot read paths.
//!
//! When a typed `findUnique` — or any `findMany`/`findFirst` without
//! include/cursor/distinct — receives a request whose argument shape has
//! already been seen (same model, same projection, same flat AND chain of
//! `Column <op> Param` predicates, same ORDER BY / LIMIT / OFFSET), we reuse
//! the previously rendered SQL text and the precomputed scalar value hints.
//! Only the parameter values are bound per call, skipping the AST build, the
//! filter qualification clone and the dialect render entirely.
//!
//! Both cache sections are bounded ([`PLAN_CACHE_CAP`]) and reclaim their
//! least-recently-used entries in batches ([`EVICTION_BATCH`]), so adversarial
//! or highly dynamic workloads cannot grow them without limit and cannot make
//! every miss pay for a full recency scan under the write lock.

use std::collections::HashMap;
use std::hash::Hash;
use std::mem::Discriminant;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use nautilus_core::{BinaryOp, Expr, OrderDir, Value};
use nautilus_protocol::{PlanCacheMetrics, PlanCacheSectionMetrics};

use crate::conversion::ValueHint;

/// Maximum number of cached plans per section (`findUnique`, `findMany`).
/// When a section is full, its least-recently-used entries are evicted on
/// insert — see [`EVICTION_BATCH`].
const PLAN_CACHE_CAP: usize = 1024;

/// How many entries one eviction pass reclaims.
///
/// Ranking entries by recency costs a full scan of the section, taken while
/// the write lock is held and therefore with every reader blocked. Reclaiming
/// a batch amortises that scan over the inserts that follow, so a workload
/// that keeps the section saturated pays the scan once per batch rather than
/// on every miss.
const EVICTION_BATCH: usize = PLAN_CACHE_CAP / 8;

/// Cache key for `findUnique` plans.
///
/// Two requests share a plan when they target the same model, request the same
/// resolved projection (selected logical fields plus implicit primary keys),
/// and produce the same ordered list of qualified filter columns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FindUniquePlanKey {
    pub(crate) model_db_name: String,
    pub(crate) selected_logical_fields: Vec<String>,
    pub(crate) filter_columns: Vec<String>,
}

/// One `(column, operator, value variant)` predicate of a cacheable filter.
///
/// The value variant is part of the shape because the PostgreSQL renderer
/// appends a cast suffix for some variants (`$1::uuid`, `$1::vector`, ...):
/// the same column receiving a different variant must not replay a plan
/// rendered for another cast.
pub(crate) type FilterPredicateShape = (String, BinaryOp, Discriminant<Value>);

/// Cache key for `findMany`/`findFirst` plans.
///
/// Two requests share a plan when they target the same model and resolved
/// projection, their filters have the same [`FilterPredicateShape`] list (flat
/// AND chain, in input order) and they request the same ORDER BY, LIMIT and
/// OFFSET. `take`/`skip` render as SQL literals — not placeholders — so their
/// *values* are part of the key; real workloads use a handful of page sizes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FindManyPlanKey {
    pub(crate) model_db_name: String,
    pub(crate) selected_logical_fields: Vec<String>,
    pub(crate) filter_shape: Vec<FilterPredicateShape>,
    pub(crate) order_by: Vec<(String, OrderDir)>,
    pub(crate) take: Option<i32>,
    pub(crate) skip: Option<u32>,
}

/// SQL plan reusable across calls with the same cache key.
#[derive(Debug)]
pub(crate) struct CachedReadPlan {
    pub(crate) sql_text: String,
    pub(crate) row_hints: Vec<Option<ValueHint>>,
}

#[derive(Debug)]
struct CacheSlot {
    plan: Arc<CachedReadPlan>,
    last_used: AtomicU64,
}

/// One cache section: an LRU-bounded map from key to rendered plan.
#[derive(Debug)]
struct BoundedPlanMap<K> {
    entries: RwLock<HashMap<K, CacheSlot>>,
    section: &'static str,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl<K: Eq + Hash + Clone> BoundedPlanMap<K> {
    fn new(section: &'static str) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            section,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Take the read guard, ignoring poisoning.
    ///
    /// The transport converts handler panics into JSON-RPC errors instead of
    /// aborting the process (see `transport.rs`), so a panic taken while this
    /// lock is held would poison it for the remaining lifetime of the engine.
    /// Honouring the poison would turn the cache into a permanent no-op — a
    /// silent, unrecoverable slowdown with no error surface. The map only ever
    /// holds fully built plans, so no partial write can be observed here.
    fn read_entries(&self) -> RwLockReadGuard<'_, HashMap<K, CacheSlot>> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take the write guard, ignoring poisoning. See [`Self::read_entries`].
    fn write_entries(&self) -> RwLockWriteGuard<'_, HashMap<K, CacheSlot>> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn get(&self, key: &K, clock: &AtomicU64) -> Option<Arc<CachedReadPlan>> {
        let guard = self.read_entries();
        let Some(slot) = guard.get(key) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        self.hits.fetch_add(1, Ordering::Relaxed);
        slot.last_used
            .store(clock.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        Some(Arc::clone(&slot.plan))
    }

    fn insert(&self, key: K, plan: Arc<CachedReadPlan>, clock: &AtomicU64) {
        let mut guard = self.write_entries();
        if !guard.contains_key(&key) && guard.len() >= PLAN_CACHE_CAP {
            let evicted = evict_oldest_batch(&mut guard, self.section);
            self.evictions.fetch_add(evicted as u64, Ordering::Relaxed);
        }
        let stamp = clock.fetch_add(1, Ordering::Relaxed);
        guard.entry(key).or_insert_with(|| CacheSlot {
            plan,
            last_used: AtomicU64::new(stamp),
        });
    }

    fn metrics(&self) -> PlanCacheSectionMetrics {
        PlanCacheSectionMetrics {
            entries: self.read_entries().len(),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    fn reset_counters(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.read_entries().len()
    }
}

/// Drop the [`EVICTION_BATCH`] least-recently-used entries of a section,
/// returning how many were actually reclaimed.
///
/// Ranking is done over the recency stamps alone — one `u64` per entry — so
/// the pass never clones a key, and the map is walked twice instead of once
/// per evicted entry.
fn evict_oldest_batch<K: Eq + Hash>(
    entries: &mut HashMap<K, CacheSlot>,
    section: &'static str,
) -> usize {
    let batch = EVICTION_BATCH.min(entries.len());
    if batch == 0 {
        return 0;
    }

    let mut stamps: Vec<u64> = entries
        .values()
        .map(|slot| slot.last_used.load(Ordering::Relaxed))
        .collect();
    let (_, cutoff, _) = stamps.select_nth_unstable(batch - 1);
    let cutoff = *cutoff;

    let mut remaining = batch;
    entries.retain(|_, slot| {
        if remaining > 0 && slot.last_used.load(Ordering::Relaxed) <= cutoff {
            remaining -= 1;
            false
        } else {
            true
        }
    });

    let evicted = batch - remaining;
    tracing::debug!(
        section,
        evicted,
        retained = entries.len(),
        "plan cache evicted least-recently-used entries"
    );
    evicted
}

/// Process-wide read-plan cache held by `EngineState`.
#[derive(Debug)]
pub(crate) struct PlanCache {
    clock: AtomicU64,
    find_unique: BoundedPlanMap<FindUniquePlanKey>,
    find_many: BoundedPlanMap<FindManyPlanKey>,
}

impl Default for PlanCache {
    fn default() -> Self {
        Self {
            clock: AtomicU64::new(0),
            find_unique: BoundedPlanMap::new("findUnique"),
            find_many: BoundedPlanMap::new("findMany"),
        }
    }
}

impl PlanCache {
    pub(crate) fn get_find_unique(&self, key: &FindUniquePlanKey) -> Option<Arc<CachedReadPlan>> {
        self.find_unique.get(key, &self.clock)
    }

    pub(crate) fn insert_find_unique(&self, key: FindUniquePlanKey, plan: Arc<CachedReadPlan>) {
        self.find_unique.insert(key, plan, &self.clock);
    }

    pub(crate) fn get_find_many(&self, key: &FindManyPlanKey) -> Option<Arc<CachedReadPlan>> {
        self.find_many.get(key, &self.clock)
    }

    pub(crate) fn insert_find_many(&self, key: FindManyPlanKey, plan: Arc<CachedReadPlan>) {
        self.find_many.insert(key, plan, &self.clock);
    }

    /// Snapshot both sections' counters for `engine.metrics`.
    pub(crate) fn metrics(&self) -> PlanCacheMetrics {
        PlanCacheMetrics {
            capacity: PLAN_CACHE_CAP,
            find_unique: self.find_unique.metrics(),
            find_many: self.find_many.metrics(),
        }
    }

    /// Zero the cumulative counters, leaving the cached plans in place.
    pub(crate) fn reset_metrics(&self) {
        self.find_unique.reset_counters();
        self.find_many.reset_counters();
    }

    #[cfg(test)]
    pub(crate) fn find_unique_len(&self) -> usize {
        self.find_unique.len()
    }

    #[cfg(test)]
    pub(crate) fn find_many_len(&self) -> usize {
        self.find_many.len()
    }
}

/// True when a parameter value can be replayed against a cached SQL text.
///
/// The rendered SQL must depend only on the value's *variant*, never on its
/// content: `Null` renders as a literal `NULL` (PostgreSQL cannot resolve a
/// typed NULL over the binary protocol), enum/composite casts embed the type
/// name carried inside the value, and array params change the SQL when their
/// elements are geometries/geographies.
fn replayable_param(value: &Value) -> bool {
    !matches!(
        value,
        Value::Null
            | Value::Array(_)
            | Value::Array2D(_)
            | Value::Enum { .. }
            | Value::Composite { .. }
    )
}

/// Operators rendered as `<column> <op> <placeholder>` with exactly one
/// placeholder regardless of the bound value. `In`/`NotIn` are excluded: the
/// rendered SQL contains one placeholder per list element, so the text
/// depends on the value.
fn slot_safe_op(op: &BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::Like
            | BinaryOp::ArrayContains
            | BinaryOp::ArrayContainedBy
            | BinaryOp::ArrayOverlaps
    )
}

/// Borrowed shape extracted from a cacheable `findUnique` filter expression.
pub(crate) struct EqFilterShape<'a> {
    pub(crate) columns: Vec<&'a str>,
    pub(crate) values: Vec<&'a Value>,
}

/// Detect whether `expr` is a flat AND chain of `Column = Param` predicates
/// (or a single equality) whose values are replayable, returning the columns
/// and parameter values in rendering order. Returns `None` for any other
/// shape so the caller falls back to the general path.
pub(crate) fn extract_simple_eq_filter(expr: &Expr) -> Option<EqFilterShape<'_>> {
    let mut columns = Vec::new();
    let mut values = Vec::new();
    walk_eq_chain(expr, &mut columns, &mut values).then_some(EqFilterShape { columns, values })
}

fn walk_eq_chain<'a>(
    expr: &'a Expr,
    columns: &mut Vec<&'a str>,
    values: &mut Vec<&'a Value>,
) -> bool {
    match expr {
        Expr::Binary {
            left,
            op: BinaryOp::Eq,
            right,
        } => match (left.as_ref(), right.as_ref()) {
            (Expr::Column(col), Expr::Param(val)) if replayable_param(val) => {
                columns.push(col.as_str());
                values.push(val);
                true
            }
            _ => false,
        },
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
        } => walk_eq_chain(left, columns, values) && walk_eq_chain(right, columns, values),
        _ => false,
    }
}

/// Borrowed shape extracted from a cacheable parametric filter: a flat AND
/// chain of `Column <op> Param` predicates where `op` is slot-safe and the
/// value is replayable.
pub(crate) struct ParamFilterShape<'a> {
    pub(crate) predicates: Vec<(&'a str, BinaryOp, Discriminant<Value>)>,
    pub(crate) values: Vec<&'a Value>,
}

/// Generalisation of [`extract_simple_eq_filter`] used by the
/// `findMany`/`findFirst` plan cache: non-equality comparison operators are
/// accepted as parametric slots. Returns `None` for any other shape so the
/// caller falls back to the general path.
pub(crate) fn extract_param_filter(expr: &Expr) -> Option<ParamFilterShape<'_>> {
    let mut predicates = Vec::new();
    let mut values = Vec::new();
    walk_param_chain(expr, &mut predicates, &mut values)
        .then_some(ParamFilterShape { predicates, values })
}

fn walk_param_chain<'a>(
    expr: &'a Expr,
    predicates: &mut Vec<(&'a str, BinaryOp, Discriminant<Value>)>,
    values: &mut Vec<&'a Value>,
) -> bool {
    match expr {
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
        } => {
            walk_param_chain(left, predicates, values)
                && walk_param_chain(right, predicates, values)
        }
        Expr::Binary { left, op, right } if slot_safe_op(op) => {
            match (left.as_ref(), right.as_ref()) {
                (Expr::Column(col), Expr::Param(val)) if replayable_param(val) => {
                    predicates.push((col.as_str(), op.clone(), std::mem::discriminant(val)));
                    values.push(val);
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;

    fn col(name: &str) -> Expr {
        Expr::Column(name.to_string())
    }

    fn param(value: i64) -> Expr {
        Expr::Param(Value::I64(value))
    }

    #[test]
    fn extracts_single_eq() {
        let expr = col("users__id").eq(param(7));
        let shape = extract_simple_eq_filter(&expr).expect("should extract shape");
        assert_eq!(shape.columns, vec!["users__id"]);
        assert_eq!(shape.values, vec![&Value::I64(7)]);
    }

    #[test]
    fn extracts_and_chain_in_input_order() {
        let expr = col("posts__tenant")
            .eq(param(1))
            .and(col("posts__id").eq(param(99)));
        let shape = extract_simple_eq_filter(&expr).expect("should extract shape");
        assert_eq!(shape.columns, vec!["posts__tenant", "posts__id"]);
        assert_eq!(shape.values, vec![&Value::I64(1), &Value::I64(99)]);
    }

    #[test]
    fn rejects_non_equality_operators() {
        let expr = col("users__id").gt(param(5));
        assert!(extract_simple_eq_filter(&expr).is_none());
    }

    #[test]
    fn rejects_or_chains() {
        let expr = col("users__id")
            .eq(param(1))
            .or(col("users__id").eq(param(2)));
        assert!(extract_simple_eq_filter(&expr).is_none());
    }

    #[test]
    fn rejects_param_on_left() {
        let expr = Expr::Binary {
            left: Box::new(param(1)),
            op: BinaryOp::Eq,
            right: Box::new(col("users__id")),
        };
        assert!(extract_simple_eq_filter(&expr).is_none());
    }

    #[test]
    fn rejects_null_params_in_both_extractors() {
        let expr = col("users__deleted_at").eq(Expr::Param(Value::Null));
        assert!(extract_simple_eq_filter(&expr).is_none());
        assert!(extract_param_filter(&expr).is_none());
    }

    #[test]
    fn param_filter_accepts_comparison_ops_as_slots() {
        let expr = col("users__age")
            .gt(param(18))
            .and(col("users__name").eq(Expr::Param(Value::String("x".to_string()))));
        let shape = extract_param_filter(&expr).expect("should extract shape");
        assert_eq!(shape.predicates.len(), 2);
        assert_eq!(shape.predicates[0].0, "users__age");
        assert_eq!(shape.predicates[0].1, BinaryOp::Gt);
        assert_eq!(shape.predicates[1].1, BinaryOp::Eq);
        assert_eq!(shape.values.len(), 2);
        assert_ne!(
            shape.predicates[0].2,
            std::mem::discriminant(&Value::String(String::new()))
        );
    }

    #[test]
    fn param_filter_rejects_in_lists_and_or() {
        let in_expr = Expr::Binary {
            left: Box::new(col("users__id")),
            op: BinaryOp::In,
            right: Box::new(Expr::List(vec![param(1), param(2)])),
        };
        assert!(extract_param_filter(&in_expr).is_none());

        let or_expr = col("users__id")
            .eq(param(1))
            .or(col("users__id").eq(param(2)));
        assert!(extract_param_filter(&or_expr).is_none());
    }

    #[test]
    fn param_filter_rejects_enum_and_array_values() {
        let enum_expr = col("users__role").eq(Expr::Param(Value::Enum {
            value: "ADMIN".to_string(),
            type_name: "role".to_string(),
        }));
        assert!(extract_param_filter(&enum_expr).is_none());

        let array_expr = Expr::Binary {
            left: Box::new(col("users__tags")),
            op: BinaryOp::ArrayContains,
            right: Box::new(Expr::Param(Value::Array(vec![Value::I64(1)]))),
        };
        assert!(extract_param_filter(&array_expr).is_none());
    }

    fn test_plan(sql: &str) -> Arc<CachedReadPlan> {
        Arc::new(CachedReadPlan {
            sql_text: sql.to_string(),
            row_hints: vec![None],
        })
    }

    fn many_key(tag: usize) -> FindManyPlanKey {
        FindManyPlanKey {
            model_db_name: format!("model_{tag}"),
            selected_logical_fields: Vec::new(),
            filter_shape: Vec::new(),
            order_by: Vec::new(),
            take: None,
            skip: None,
        }
    }

    #[test]
    fn cache_returns_inserted_plan() {
        let cache = PlanCache::default();
        let key = FindUniquePlanKey {
            model_db_name: "User".to_string(),
            selected_logical_fields: vec!["id".to_string(), "name".to_string()],
            filter_columns: vec!["users__id".to_string()],
        };
        let plan = test_plan("SELECT 1");
        cache.insert_find_unique(key.clone(), Arc::clone(&plan));
        let got = cache.get_find_unique(&key).expect("plan should be cached");
        assert!(Arc::ptr_eq(&plan, &got));
        assert_eq!(cache.find_unique_len(), 1);
    }

    #[test]
    fn cache_keeps_serving_after_lock_poisoning() {
        let cache = PlanCache::default();
        let key = many_key(0);
        cache.insert_find_many(key.clone(), test_plan("SELECT 1"));

        let poisoned = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = cache
                .find_many
                .entries
                .write()
                .expect("lock starts healthy");
            panic!("handler panic while holding the plan cache lock");
        }));
        assert!(poisoned.is_err());
        assert!(cache.find_many.entries.is_poisoned());

        assert!(
            cache.get_find_many(&key).is_some(),
            "existing plans must still be served after a poisoning panic"
        );

        cache.insert_find_many(many_key(1), test_plan("SELECT 2"));
        assert!(
            cache.get_find_many(&many_key(1)).is_some(),
            "inserts must still land after a poisoning panic"
        );
    }

    #[test]
    fn cache_evicts_least_recently_used_at_capacity() {
        let cache = PlanCache::default();
        for tag in 0..PLAN_CACHE_CAP {
            cache.insert_find_many(many_key(tag), test_plan("SELECT 1"));
        }
        assert_eq!(cache.find_many_len(), PLAN_CACHE_CAP);

        assert!(cache.get_find_many(&many_key(0)).is_some());

        cache.insert_find_many(many_key(PLAN_CACHE_CAP), test_plan("SELECT 2"));
        assert_eq!(
            cache.find_many_len(),
            PLAN_CACHE_CAP - EVICTION_BATCH + 1,
            "one eviction pass reclaims a whole batch"
        );
        assert!(
            cache.get_find_many(&many_key(0)).is_some(),
            "recently touched entry must survive eviction"
        );
        assert!(
            cache.get_find_many(&many_key(1)).is_none(),
            "least-recently-used entry must be evicted"
        );
        assert!(cache.get_find_many(&many_key(PLAN_CACHE_CAP)).is_some());
    }

    #[test]
    fn cache_never_grows_past_the_cap_across_repeated_evictions() {
        let cache = PlanCache::default();
        for tag in 0..(PLAN_CACHE_CAP * 3) {
            cache.insert_find_many(many_key(tag), test_plan("SELECT 1"));
            assert!(
                cache.find_many_len() <= PLAN_CACHE_CAP,
                "cap must hold after every insert"
            );
        }
        assert!(
            cache
                .get_find_many(&many_key(PLAN_CACHE_CAP * 3 - 1))
                .is_some(),
            "the most recent insert must still be cached"
        );
    }
}
