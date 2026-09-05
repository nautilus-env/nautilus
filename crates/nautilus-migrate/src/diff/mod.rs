//! Schema diff engine — compares a [`LiveSchema`] snapshot against a target
//! [`SchemaIr`] and returns a list of [`Change`]s that need to be applied.
//!
//! The comparison is split by domain: `extensions` and `user_types` cover the
//! objects a table may depend on, `tables`, `columns`, `indexes` and
//! `foreign_keys` cover the tables themselves, `normalize` owns the rules that
//! decide when two spellings mean the same thing, and `ordering` turns a set of
//! changes into an executable sequence.

mod columns;
mod extensions;
mod foreign_keys;
mod indexes;
mod normalize;
mod ordering;
mod tables;
mod user_types;

use nautilus_schema::ir::SchemaIr;

use crate::ddl::{DatabaseProvider, DdlGenerator};
use crate::live::LiveSchema;

pub use crate::change::{change_risk, Change, ChangeDescription, ChangeRisk};
pub use ordering::order_changes_for_apply;

pub(crate) use ordering::topo_sort_models;

/// Computes the difference between a live database and a target schema.
pub struct SchemaDiff;

impl SchemaDiff {
    /// Compare `live` (current DB state) against `target` (desired schema) and
    /// return an ordered list of changes that must be applied to make the live
    /// DB match the target.
    pub fn compute(
        live: &LiveSchema,
        target: &SchemaIr,
        provider: DatabaseProvider,
    ) -> Vec<Change> {
        let mut acc = DiffAccumulator::new(provider);
        acc.diff_schemas(live, target);
        acc.diff_extensions(live, target);
        acc.diff_composite_types(live, target);
        acc.diff_enums(live, target);
        acc.diff_new_tables(live, target);
        acc.diff_dropped_tables(live, target);
        acc.diff_existing_tables(live, target);
        acc.finish()
    }
}

/// Accumulates the changes produced by the per-domain diff passes.
///
/// Changes land in one of three buckets because type definitions must be
/// created before the tables that reference them and dropped only after those
/// tables are gone: `pre_type` runs first, then the structural `changes`, then
/// `post_type`.  [`DiffAccumulator::finish`] concatenates them in that order.
struct DiffAccumulator {
    ddl: DdlGenerator,
    provider: DatabaseProvider,
    changes: Vec<Change>,
    pre_type: Vec<Change>,
    post_type: Vec<Change>,
}

impl DiffAccumulator {
    fn new(provider: DatabaseProvider) -> Self {
        Self {
            ddl: DdlGenerator::new(provider),
            provider,
            changes: Vec::new(),
            pre_type: Vec::new(),
            post_type: Vec::new(),
        }
    }

    /// Consume the accumulator and return the three buckets concatenated in
    /// execution order.
    fn finish(mut self) -> Vec<Change> {
        let mut all_changes = self.pre_type;
        all_changes.append(&mut self.changes);
        all_changes.append(&mut self.post_type);
        all_changes
    }
}
