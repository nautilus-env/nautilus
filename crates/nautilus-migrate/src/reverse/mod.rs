use crate::ddl::DatabaseProvider;
use crate::diff::Change;
use crate::live::LiveSchema;
use crate::provider::ProviderStrategy;
use nautilus_core::TableName;

mod columns;
mod indexes;
mod snapshot;
mod tables;

/// Builds best-effort down-SQL for a single [`Change`].
///
/// Reversal is deliberately partial: a destructive change carries only the
/// information needed to apply it forward, so arms that cannot be recovered
/// from the diff plus the live snapshot emit a comment placeholder rather than
/// SQL that would silently lose data.
pub(crate) struct ChangeReverser<'a> {
    provider: DatabaseProvider,
    strategy: ProviderStrategy,
    live: &'a LiveSchema,
}

impl<'a> ChangeReverser<'a> {
    pub(crate) fn new(provider: DatabaseProvider, live: &'a LiveSchema) -> Self {
        Self {
            provider,
            strategy: ProviderStrategy::new(provider),
            live,
        }
    }

    /// Reverse a change where the provider and live snapshot support it.
    /// Creating a schema has no reversal: it may contain unmanaged objects.
    pub(crate) fn reverse(&self, change: &Change) -> Vec<String> {
        match change {
            Change::NewTable(model) => self.reverse_new_table(&crate::live::model_table(model)),
            Change::DroppedTable { name } => self.reverse_dropped_table(name),
            Change::PrimaryKeyChanged { table } => self.reverse_primary_key_change(table),

            Change::AddedColumn { table, field } => {
                self.reverse_added_column(table, &field.db_name)
            }
            Change::DroppedColumn { table, column } => self.reverse_dropped_column(table, column),
            Change::TypeChanged {
                table,
                column,
                from,
                ..
            } => self.reverse_type_change(table, column, from),
            Change::NullabilityChanged {
                table,
                column,
                now_required,
            } => self.reverse_nullability_change(table, column, *now_required),
            Change::DefaultChanged {
                table,
                column,
                from,
                ..
            } => self.reverse_default_change(table, column, from.as_deref()),
            Change::AutoIncrementChanged { table, column, .. } => {
                self.reverse_auto_increment_change(table, column)
            }
            Change::ComputedExprChanged { table, column, .. } => {
                cannot_reverse(format!("computed expression change: {}.{}", table, column))
            }

            Change::IndexAdded {
                table,
                columns,
                index_name,
                ..
            } => self.reverse_index_added(table, columns, index_name.as_deref()),
            Change::IndexDropped {
                table,
                columns,
                unique,
                index_name,
            } => self.reverse_index_dropped(table, columns, *unique, index_name),

            Change::CheckChanged { table, column, .. } => {
                let target = match column {
                    Some(col) => format!("{}.{}", table, col),
                    None => table.to_string(),
                };
                cannot_reverse(format!("CHECK constraint change on {}", target))
            }
            Change::ForeignKeyAdded {
                table,
                constraint_name,
                ..
            } => self.reverse_foreign_key_added(table, constraint_name),
            Change::ForeignKeyDropped {
                table,
                constraint_name,
            } => cannot_reverse(format!(
                "DROP FOREIGN KEY {} on {}; restore manually",
                constraint_name, table
            )),

            Change::CreateCompositeType { name } | Change::CreateEnum { name, .. } => {
                self.reverse_user_type(|| vec![self.drop_type_sql(name)])
            }
            Change::DropCompositeType { name } | Change::AlterCompositeType { name, .. } => self
                .reverse_user_type(|| {
                    cannot_reverse(format!(
                        "composite type change for '{}'; restore manually",
                        name
                    ))
                }),
            Change::DropEnum { name } | Change::AlterEnum { name, .. } => {
                self.reverse_user_type(|| {
                    cannot_reverse(format!("enum type change for '{}'; restore manually", name))
                })
            }
            Change::CreateExtension { name, .. } => {
                self.reverse_user_type(|| vec![self.strategy.drop_extension_sql(name)])
            }
            Change::CreateSchema { .. } => Vec::new(),
            Change::DropExtension { name } => self.reverse_user_type(|| {
                cannot_reverse(format!("extension drop for '{}'; reinstall manually", name))
            }),
        }
    }

    fn quote(&self, name: &str) -> String {
        self.provider.quote_identifier(name)
    }

    /// Quote a table in the statement's table position, qualifying it with its
    /// schema when it has one.
    fn quote_table(&self, table: &TableName) -> String {
        self.strategy.quote_table(table)
    }

    /// Run `build` only on providers with user-defined types; elsewhere the
    /// forward change was itself a no-op, so its reversal must be empty.
    fn reverse_user_type(&self, build: impl FnOnce() -> Vec<String>) -> Vec<String> {
        if self.strategy.supports_user_defined_types() {
            build()
        } else {
            Vec::new()
        }
    }

    fn drop_type_sql(&self, name: &str) -> String {
        self.strategy.drop_type_sql(name)
    }

    fn reverse_foreign_key_added(&self, table: &TableName, constraint_name: &str) -> Vec<String> {
        match self.strategy.drop_foreign_key_sql(table, constraint_name) {
            Some(sql) => vec![sql],
            None => cannot_reverse(format!("ADD FOREIGN KEY on SQLite: {}", constraint_name)),
        }
    }

    fn quote_all(&self, columns: &[String]) -> String {
        columns
            .iter()
            .map(|column| self.quote(column))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn cannot_reverse(detail: String) -> Vec<String> {
    vec![format!("-- Cannot auto-reverse {}", detail)]
}

fn missing_snapshot(detail: String) -> Vec<String> {
    vec![format!(
        "-- Cannot auto-reverse: {} (no live snapshot)",
        detail
    )]
}
