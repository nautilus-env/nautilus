//! Foreign-key diff pass, and the descriptor that turns a relation field into
//! the constraint it implies.

use nautilus_schema::ir::{ModelIr, ResolvedFieldType, SchemaIr};

use super::DiffAccumulator;
use crate::change::Change;
use crate::ddl::DatabaseProvider;
use crate::live::LiveTable;
use nautilus_core::TableName;

impl DiffAccumulator {
    /// Diff the foreign keys of one table.
    ///
    /// A referential-action change cannot be altered in place, so it is emitted
    /// as a drop followed by an add of the same constraint.
    pub(super) fn diff_foreign_keys(
        &mut self,
        table_name: &TableName,
        live_table: &LiveTable,
        model: &ModelIr,
        target: &SchemaIr,
        unmanaged_columns: &std::collections::HashSet<&str>,
    ) {
        let target_fks = collect_target_foreign_keys(model, target);

        for tfk in &target_fks {
            let live_match = live_table.foreign_keys.iter().find(|lk| {
                lk.columns == tfk.columns
                    && lk.referenced_table == tfk.referenced_table
                    && lk.referenced_columns == tfk.referenced_columns
            });

            match live_match {
                None => {
                    self.changes.push(tfk.as_added(table_name));
                }
                Some(live_fk) => {
                    let actions_differ = !fk_actions_equal(
                        self.provider,
                        live_fk.on_delete.as_deref(),
                        tfk.on_delete.as_deref(),
                    ) || !fk_actions_equal(
                        self.provider,
                        live_fk.on_update.as_deref(),
                        tfk.on_update.as_deref(),
                    );
                    if actions_differ {
                        self.changes.push(Change::ForeignKeyDropped {
                            table: table_name.clone(),
                            constraint_name: live_fk.constraint_name.clone(),
                        });
                        self.changes.push(tfk.as_added(table_name));
                    }
                }
            }
        }

        for live_fk in &live_table.foreign_keys {
            if live_fk
                .columns
                .iter()
                .any(|column| unmanaged_columns.contains(column.as_str()))
            {
                continue;
            }
            let still_in_target = target_fks.iter().any(|tf| {
                tf.columns == live_fk.columns
                    && tf.referenced_table == live_fk.referenced_table
                    && tf.referenced_columns == live_fk.referenced_columns
            });
            if !still_in_target {
                self.changes.push(Change::ForeignKeyDropped {
                    table: table_name.clone(),
                    constraint_name: live_fk.constraint_name.clone(),
                });
            }
        }
    }
}

/// Collect the foreign keys implied by the relation fields of `model`.
fn collect_target_foreign_keys(model: &ModelIr, target: &SchemaIr) -> Vec<TargetFkDescriptor> {
    model
        .fields
        .iter()
        .filter_map(|field| {
            let ResolvedFieldType::Relation(rel) = &field.field_type else {
                return None;
            };
            if rel.fields.is_empty() {
                return None;
            }
            let target_model = target.models.get(&rel.target_model)?;
            let fk_cols: Vec<String> = rel
                .fields
                .iter()
                .filter_map(|fname| model.find_field(fname))
                .map(|f| f.db_name.clone())
                .collect();
            if fk_cols.is_empty() {
                return None;
            }
            let ref_cols: Vec<String> = rel
                .references
                .iter()
                .filter_map(|rname| target_model.find_field(rname))
                .map(|f| f.db_name.clone())
                .collect();
            Some(TargetFkDescriptor {
                columns: fk_cols,
                referenced_table: crate::live::model_table(target_model),
                referenced_columns: ref_cols,
                on_delete: rel.on_delete.as_ref().map(fk_action_to_str),
                on_update: rel.on_update.as_ref().map(fk_action_to_str),
            })
        })
        .collect()
}

/// Internal descriptor for a foreign-key constraint derived from the target schema IR.
struct TargetFkDescriptor {
    columns: Vec<String>,
    referenced_table: TableName,
    referenced_columns: Vec<String>,
    on_delete: Option<String>,
    on_update: Option<String>,
}

impl TargetFkDescriptor {
    /// Build the [`Change::ForeignKeyAdded`] that creates this constraint on
    /// `table`.
    fn as_added(&self, table: &TableName) -> Change {
        Change::ForeignKeyAdded {
            table: table.clone(),
            constraint_name: fk_auto_name(table, &self.columns),
            columns: self.columns.clone(),
            referenced_table: self.referenced_table.clone(),
            referenced_columns: self.referenced_columns.clone(),
            on_delete: self.on_delete.clone(),
            on_update: self.on_update.clone(),
        }
    }
}

/// Convert a [`ReferentialAction`] to its SQL keyword string (upper-cased).
fn fk_action_to_str(action: &nautilus_schema::ast::ReferentialAction) -> String {
    use nautilus_schema::ast::ReferentialAction;
    match action {
        ReferentialAction::Cascade => "CASCADE".to_string(),
        ReferentialAction::Restrict => "RESTRICT".to_string(),
        ReferentialAction::NoAction => "NO ACTION".to_string(),
        ReferentialAction::SetNull => "SET NULL".to_string(),
        ReferentialAction::SetDefault => "SET DEFAULT".to_string(),
    }
}

/// Compare two FK action values, treating `None` and `"NO ACTION"` as equivalent
/// (both represent the database default).
fn fk_actions_equal(provider: DatabaseProvider, live: Option<&str>, target: Option<&str>) -> bool {
    fn normalise(provider: DatabaseProvider, action: Option<&str>) -> &str {
        match action {
            None => match provider {
                DatabaseProvider::Mysql => "restrict",
                DatabaseProvider::Postgres | DatabaseProvider::Sqlite => "no action",
            },
            Some(action) if action.eq_ignore_ascii_case("NO ACTION") => match provider {
                DatabaseProvider::Mysql => "restrict",
                DatabaseProvider::Postgres | DatabaseProvider::Sqlite => "no action",
            },
            Some(action)
                if provider == DatabaseProvider::Mysql
                    && action.eq_ignore_ascii_case("RESTRICT") =>
            {
                "restrict"
            }
            Some(action) => action,
        }
    }

    normalise(provider, live).eq_ignore_ascii_case(normalise(provider, target))
}

/// Derive a deterministic FK constraint name from table and FK column list.
fn fk_auto_name(table: &TableName, columns: &[String]) -> String {
    format!("fk_{}_{}", table.name, columns.join("_"))
}
