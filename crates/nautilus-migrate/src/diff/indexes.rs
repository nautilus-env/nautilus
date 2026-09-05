//! Index diff pass, and the matching rules that decide whether a declared index
//! and an inspected one describe the same physical index.

use nautilus_schema::ir::{BasicIndexType, IndexKind, ModelIr};

use super::DiffAccumulator;
use crate::change::Change;
use crate::live::{LiveIndex, LiveIndexKind, LiveTable};
use crate::normalize::predicates::normalize_check_expr;
use nautilus_core::TableName;

impl DiffAccumulator {
    /// Diff the indexes of one table, matching on sorted column list,
    /// uniqueness, access method, and — for explicitly named indexes — name.
    pub(super) fn diff_indexes(
        &mut self,
        table_name: &TableName,
        live_table: &LiveTable,
        model: &ModelIr,
        unmanaged_columns: &std::collections::HashSet<&str>,
    ) {
        let target_indexes = collect_target_indexes(table_name, model);

        let live_indexes: Vec<NormalizedLiveIndex<'_>> = live_table
            .indexes
            .iter()
            .filter(|i| {
                !i.columns
                    .iter()
                    .any(|column| unmanaged_columns.contains(column.as_str()))
            })
            .map(|i| {
                let mut cols = i.columns.clone();
                cols.sort();
                NormalizedLiveIndex {
                    sorted_cols: cols,
                    unique: i.unique,
                    live: i,
                }
            })
            .collect();

        for ti in &target_indexes {
            let found = live_indexes.iter().any(|li| indexes_match(ti, li));
            if !found {
                self.changes.push(Change::IndexAdded {
                    table: table_name.clone(),
                    columns: ti.sorted_cols.clone(),
                    unique: ti.unique,
                    kind: ti.kind.clone(),
                    index_name: Some(ti.effective_name.clone()),
                    predicate: ti.predicate.clone(),
                });
            }
        }

        for li in &live_indexes {
            let found = target_indexes.iter().any(|ti| indexes_match(ti, li));
            if !found {
                self.changes.push(Change::IndexDropped {
                    table: table_name.clone(),
                    columns: li.sorted_cols.clone(),
                    unique: li.unique,
                    index_name: li.live.name.clone(),
                });
            }
        }
    }
}

/// An index declared in the target schema, normalised for comparison against
/// the live database.
struct TargetIndex {
    sorted_cols: Vec<String>,
    unique: bool,
    kind: IndexKind,
    predicate: Option<String>,
    effective_name: String,
    /// Only indexes with an explicit `map:`/`name:` require the physical name
    /// to match; auto-named ones are matched structurally.
    name_must_match: bool,
}

/// A live index with its column list sorted, so it can be compared against a
/// [`TargetIndex`] regardless of declaration order.
struct NormalizedLiveIndex<'a> {
    sorted_cols: Vec<String>,
    unique: bool,
    live: &'a LiveIndex,
}

/// Collect the target indexes of `model`, merging `@@index` declarations and
/// `@@unique` constraints into a single comparable list.
fn collect_target_indexes(table_name: &TableName, model: &ModelIr) -> Vec<TargetIndex> {
    let resolve_cols = |fields: &[String]| -> Vec<String> {
        let mut cols: Vec<String> = fields
            .iter()
            .map(|name| {
                model
                    .find_field(name)
                    .map(|f| f.db_name.clone())
                    .unwrap_or_else(|| name.clone())
            })
            .collect();
        cols.sort();
        cols
    };

    let mut idxs: Vec<TargetIndex> = Vec::new();

    for idx in &model.indexes {
        let cols = resolve_cols(&idx.fields);
        let ddl_name = idx
            .map
            .clone()
            .unwrap_or_else(|| format!("idx_{}_{}", table_name, cols.join("_")));
        idxs.push(TargetIndex {
            sorted_cols: cols,
            unique: false,
            kind: idx.kind.clone(),
            predicate: idx.predicate.clone(),
            effective_name: ddl_name,
            name_must_match: idx.map.is_some(),
        });
    }

    for uc in &model.unique_constraints {
        let cols = resolve_cols(&uc.fields);
        let ddl_name = format!("idx_{}_{}", table_name, cols.join("_"));
        idxs.push(TargetIndex {
            sorted_cols: cols,
            unique: true,
            kind: IndexKind::Default,
            predicate: None,
            effective_name: ddl_name,
            name_must_match: false,
        });
    }

    idxs
}

/// Returns `true` when a target index and a live index describe the same
/// physical index.
fn indexes_match(target: &TargetIndex, live: &NormalizedLiveIndex<'_>) -> bool {
    live.sorted_cols == target.sorted_cols
        && live.unique == target.unique
        && (!target.name_must_match || live.live.name == target.effective_name)
        && index_kinds_match(&target.kind, &live.live.kind)
        && index_predicates_match(target.predicate.as_deref(), live.live.predicate.as_deref())
}

/// Compares a declared partial-index predicate against the one the database
/// reports.
///
/// Both PostgreSQL and SQLite re-render the predicate from their own parse
/// tree, so the stored text differs from the schema source in whitespace,
/// added parentheses and (on PostgreSQL) explicit casts on every literal.
/// Comparing the normalised forms — the same normalisation `CHECK` constraints
/// already use — avoids a permanent drop/recreate cycle for an index that has
/// not actually changed.
fn index_predicates_match(target: Option<&str>, live: Option<&str>) -> bool {
    match (target, live) {
        (None, None) => true,
        (Some(t), Some(l)) => normalize_check_expr(t) == normalize_check_expr(l),
        _ => false,
    }
}

/// Returns `true` when a target [`IndexKind`] and an inspected [`LiveIndexKind`]
/// describe the same access method.
///
/// The interesting subtlety is that a target schema with no `type:` argument
/// (`IndexKind::Default`) must compare equal to a live BTree index — which
/// the database reports either as `LiveIndexKind::Basic(BTree)` (Postgres,
/// MySQL) or `LiveIndexKind::Unknown(None)` (SQLite, which does not expose
/// access methods at all).
fn index_kinds_match(target: &IndexKind, live: &LiveIndexKind) -> bool {
    match (target, live) {
        (IndexKind::Default, LiveIndexKind::Unknown(None)) => true,
        (IndexKind::Default, LiveIndexKind::Basic(BasicIndexType::BTree)) => true,
        (IndexKind::Basic(BasicIndexType::BTree), LiveIndexKind::Unknown(None)) => true,
        (IndexKind::Basic(t), LiveIndexKind::Basic(l)) => t == l,
        (IndexKind::Pgvector(t), LiveIndexKind::Pgvector(l)) => t == l,
        _ => false,
    }
}
