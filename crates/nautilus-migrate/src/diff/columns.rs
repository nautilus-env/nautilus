//! Column-level diff pass: additions, drops, and the per-column type,
//! nullability, identity, default, generation-expression and CHECK comparisons.

use nautilus_schema::ir::FieldIr;

use super::normalize::{
    column_types_match, normalize_check_expr, normalize_default, normalize_generated_expr,
};
use super::DiffAccumulator;
use crate::change::Change;
use crate::ddl::DatabaseProvider;
use crate::live::LiveTable;
use nautilus_core::TableName;

impl DiffAccumulator {
    /// Diff the scalar columns of one table: additions, drops, and per-column
    /// type, nullability, default, generation-expression and CHECK changes.
    pub(super) fn diff_columns(
        &mut self,
        table_name: &TableName,
        live_table: &LiveTable,
        target_scalar_fields: &[&FieldIr],
        unmanaged_columns: &std::collections::HashSet<&str>,
    ) {
        let live_cols: std::collections::HashMap<&str, _> = live_table
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        let target_cols_by_db: std::collections::HashMap<&str, &FieldIr> = target_scalar_fields
            .iter()
            .map(|f| (f.db_name.as_str(), *f))
            .collect();

        for field in target_scalar_fields {
            if !live_cols.contains_key(field.db_name.as_str()) {
                self.changes.push(Change::AddedColumn {
                    table: table_name.clone(),
                    field: (*field).clone(),
                });
            }
        }

        for live_col_name in live_cols.keys() {
            if !target_cols_by_db.contains_key(*live_col_name)
                && !unmanaged_columns.contains(*live_col_name)
            {
                self.changes.push(Change::DroppedColumn {
                    table: table_name.clone(),
                    column: (*live_col_name).to_string(),
                });
            }
        }

        for field in target_scalar_fields {
            let Some(live_col) = live_cols.get(field.db_name.as_str()) else {
                continue; // AddedColumn already emitted
            };

            let target_type = self.ddl.column_type_sql(field).unwrap_or_default();
            if !target_type.is_empty()
                && !column_types_match(self.provider, &live_col.col_type, &target_type)
            {
                self.changes.push(Change::TypeChanged {
                    table: table_name.clone(),
                    column: field.db_name.clone(),
                    from: live_col.col_type.clone(),
                    to: target_type,
                });
            }

            // `field.is_required` means NOT NULL; `live_col.nullable` means NULL allowed.
            let target_nullable = !field.is_required;
            if target_nullable != live_col.nullable {
                self.changes.push(Change::NullabilityChanged {
                    table: table_name.clone(),
                    column: field.db_name.clone(),
                    now_required: !target_nullable,
                });
            }

            // Normalise both sides so that superficial formatting differences
            // (e.g. outer parentheses that some databases strip) don't produce
            // false positives.
            //
            // Skip entirely for `autoincrement()` fields: PostgreSQL SERIAL
            // implicitly creates a `nextval(...)` column default that the
            // inspector reports; `column_default_sql()` returns `None` for
            // autoincrement (it's managed by the SERIAL type, not a plain
            // DEFAULT clause).  Without this guard the diff would see
            // `None` vs `Some("nextval(...)")` and emit `DROP DEFAULT`,
            // destroying the sequence link and breaking all future INSERTs.
            let is_autoincrement = matches!(
                &field.default_value,
                Some(nautilus_schema::ir::DefaultValue::Function(f)) if f.name == "autoincrement"
            );

            // MySQL keeps `AUTO_INCREMENT` on the column rather than in a
            // default or a sequence, so it is the one provider where the
            // attribute can drift from the schema and be repaired in place.
            // A table created before the generator emitted it has a plain
            // integer key that rejects every insert without an explicit id.
            if self.provider == DatabaseProvider::Mysql
                && is_autoincrement != live_col.auto_increment
            {
                self.changes.push(Change::AutoIncrementChanged {
                    table: table_name.clone(),
                    column: field.db_name.clone(),
                    enabled: is_autoincrement,
                });
            }

            if !is_autoincrement {
                let target_default_sql = self.ddl.column_default_sql(field).unwrap_or(None);
                let target_default = target_default_sql.as_deref().map(normalize_default);
                let live_default = live_col.default_value.as_deref().map(normalize_default);
                if target_default != live_default {
                    self.changes.push(Change::DefaultChanged {
                        table: table_name.clone(),
                        column: field.db_name.clone(),
                        from: live_col.default_value.clone(),
                        to: target_default_sql,
                    });
                }
            }

            // Database engines reformat generated expressions heavily
            // (adding casts, parens, spacing), so canonicalise both sides
            // before comparing.
            let target_expr = field
                .computed
                .as_ref()
                .map(|(expr, _)| normalize_generated_expr(expr));
            let live_expr = live_col
                .generated_expr
                .as_deref()
                .map(normalize_generated_expr);
            if target_expr != live_expr {
                self.changes.push(Change::ComputedExprChanged {
                    table: table_name.clone(),
                    column: field.db_name.clone(),
                    field: (*field).clone(),
                });
            }

            // Suppress the change when the expression already exists
            // somewhere in the table's check-constraint pool.  This
            // handles older databases where column checks were created
            // inline (auto-named by PG, e.g. `order_items_quantity_check`)
            // and therefore ended up in live_table.check_constraints rather
            // than live_col.check_expr.
            let target_check = field.check.as_deref().map(normalize_check_expr);
            let live_check = live_col.check_expr.as_deref().map(normalize_check_expr);
            let check_already_in_table_pool = target_check.as_ref().is_some_and(|tc| {
                live_table
                    .check_constraints
                    .iter()
                    .map(|lc| normalize_check_expr(lc))
                    .any(|lc| lc == *tc)
            });
            if target_check != live_check && !check_already_in_table_pool {
                self.changes.push(Change::CheckChanged {
                    table: table_name.clone(),
                    column: Some(field.db_name.clone()),
                    from: live_col.check_expr.clone(),
                    to: field.check.clone(),
                });
            }
        }
    }
}
