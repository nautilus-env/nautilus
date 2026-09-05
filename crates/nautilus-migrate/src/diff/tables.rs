//! Table-level diff passes: which tables appear or disappear, and — for the
//! tables present on both sides — their primary key and table-level CHECK
//! constraints.

use nautilus_schema::ir::{FieldIr, ModelIr, SchemaIr};

use super::ordering::topo_sort_models;
use super::DiffAccumulator;
use crate::change::Change;
use crate::live::{LiveSchema, LiveTable};
use crate::normalize::predicates::normalize_check_expr;
use nautilus_core::TableName;

impl DiffAccumulator {
    /// Emit a [`Change::NewTable`] for every target model without a live table,
    /// ordered so that referenced tables are created before their dependents.
    pub(super) fn diff_new_tables(&mut self, live: &LiveSchema, target: &SchemaIr) {
        let new_models: Vec<&ModelIr> = crate::ddl::managed_models(target)
            .into_iter()
            .filter(|m| !live.tables.contains_key(&crate::live::model_table(m)))
            .collect();

        for model in topo_sort_models(&new_models) {
            self.changes.push(Change::NewTable(model.clone()));
        }
    }

    /// Emit a [`Change::DroppedTable`] for every live table with no target model.
    pub(super) fn diff_dropped_tables(&mut self, live: &LiveSchema, target: &SchemaIr) {
        let target_by_db = target_models_by_db_name(target);
        for live_table_name in live.tables.keys() {
            if !target_by_db.contains_key(live_table_name) {
                self.changes.push(Change::DroppedTable {
                    name: live_table_name.clone(),
                });
            }
        }
    }

    /// Diff every table present in both the live database and the target schema.
    pub(super) fn diff_existing_tables(&mut self, live: &LiveSchema, target: &SchemaIr) {
        let target_by_db = target_models_by_db_name(target);

        for (table_name, live_table) in &live.tables {
            let Some(model) = target_by_db.get(table_name) else {
                continue; // already emitted DroppedTable
            };

            // An `@@ignore`d model — and a `view`, which names a relation the
            // database owns outright — exists only so the diff knows the table
            // is not orphaned. Nothing about it is Nautilus's to change.
            if model.is_ignored || model.is_view {
                continue;
            }

            let target_scalar_fields: Vec<&FieldIr> =
                crate::ddl::managed_scalar_fields(model).collect();
            let unmanaged_columns: std::collections::HashSet<&str> = model
                .fields
                .iter()
                .filter(|f| f.is_ignored)
                .map(|f| f.db_name.as_str())
                .collect();

            self.diff_columns(
                table_name,
                live_table,
                &target_scalar_fields,
                &unmanaged_columns,
            );
            self.diff_table_checks(
                table_name,
                live_table,
                model,
                &target_scalar_fields,
                &unmanaged_columns,
            );
            self.diff_primary_key(table_name, live_table, model);
            self.diff_indexes(table_name, live_table, model, &unmanaged_columns);
            self.diff_foreign_keys(table_name, live_table, model, target, &unmanaged_columns);
        }
    }

    /// Diff the table-level CHECK constraints of one table.
    pub(super) fn diff_table_checks(
        &mut self,
        table_name: &TableName,
        live_table: &LiveTable,
        model: &ModelIr,
        target_scalar_fields: &[&FieldIr],
        unmanaged_columns: &std::collections::HashSet<&str>,
    ) {
        // Expressions covered by column-level @check fields — we must
        // not emit a "drop" for auto-named column constraints that were
        // bucketed into the table pool by the inspector.
        let column_check_exprs: std::collections::HashSet<String> = target_scalar_fields
            .iter()
            .filter_map(|f| f.check.as_deref())
            .map(normalize_check_expr)
            .collect();

        let mut target_checks: Vec<String> = model
            .check_constraints
            .iter()
            .map(|s| normalize_check_expr(s))
            .collect();
        target_checks.sort();
        let mut live_checks: Vec<String> = live_table
            .check_constraints
            .iter()
            .map(|s| normalize_check_expr(s))
            .collect();
        live_checks.sort();

        for tc in &target_checks {
            if !live_checks.contains(tc) {
                self.changes.push(Change::CheckChanged {
                    table: table_name.clone(),
                    column: None,
                    from: None,
                    to: Some(tc.clone()),
                });
            }
        }
        // Do not drop auto-named column constraints that belong to a
        // column-level @check target.
        for lc in &live_checks {
            if mentions_any_column(lc, unmanaged_columns) {
                continue;
            }
            if !target_checks.contains(lc) && !column_check_exprs.contains(lc.as_str()) {
                self.changes.push(Change::CheckChanged {
                    table: table_name.clone(),
                    column: None,
                    from: Some(lc.clone()),
                    to: None,
                });
            }
        }
    }

    /// Compare the live primary key against the target model's.
    pub(super) fn diff_primary_key(
        &mut self,
        table_name: &TableName,
        live_table: &LiveTable,
        model: &ModelIr,
    ) {
        // `model.primary_key.fields()` returns *logical* field names; the
        // live PKs come from the DB and use *db* column names.  Resolve
        // logical -> db before comparing so @map doesn't cause false positives.
        let mut target_pk: Vec<String> = model
            .primary_key
            .fields()
            .iter()
            .map(|logical| {
                model
                    .find_field(logical)
                    .map(|f| f.db_name.clone())
                    .unwrap_or_else(|| (*logical).to_string())
            })
            .collect();
        target_pk.sort();
        let mut live_pk: Vec<String> = live_table.primary_key.clone();
        live_pk.sort();

        if target_pk != live_pk {
            self.changes.push(Change::PrimaryKeyChanged {
                table: table_name.clone(),
            });
        }
    }
}

/// Whether a normalised SQL expression mentions any of `columns` as a whole
/// identifier.
///
/// Used to leave a live CHECK constraint alone when it constrains a column the
/// schema marks `@ignore`: Nautilus does not model that column, so it cannot
/// tell whether the constraint still belongs and must not propose dropping it.
fn mentions_any_column(expr: &str, columns: &std::collections::HashSet<&str>) -> bool {
    if columns.is_empty() {
        return false;
    }
    expr.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| columns.contains(word))
}

/// Index the target models by their DB table name.
fn target_models_by_db_name(target: &SchemaIr) -> std::collections::HashMap<TableName, &ModelIr> {
    target
        .models
        .values()
        .map(|m| (crate::live::model_table(m), m))
        .collect()
}
