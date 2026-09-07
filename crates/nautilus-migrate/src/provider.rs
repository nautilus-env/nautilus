use crate::error::{MigrationError, Result};
use nautilus_core::TableName;
use nautilus_schema::ast::StorageStrategy;

mod constraints;
mod database;
mod indexes;
mod pgvector;
mod user_types;

pub(crate) use constraints::check_constraint_name;
pub use database::DatabaseProvider;
pub(crate) use indexes::CreateIndex;

pub(crate) struct AlterColumnType<'a> {
    pub(crate) table: &'a TableName,
    pub(crate) column: &'a str,
    pub(crate) target_type: &'a str,
    pub(crate) full_column_definition: Option<&'a str>,
}

pub(crate) struct AlterColumnNullability<'a> {
    pub(crate) table: &'a TableName,
    pub(crate) column: &'a str,
    pub(crate) now_required: bool,
    pub(crate) is_generated: bool,
    pub(crate) default_sql: Option<&'a str>,
    pub(crate) full_column_definition: Option<&'a str>,
}

pub(crate) struct AlterColumnDefault<'a> {
    pub(crate) table: &'a TableName,
    pub(crate) column: &'a str,
    pub(crate) new_default: Option<&'a str>,
    pub(crate) preserve_implicit_default: bool,
    pub(crate) full_column_definition: Option<&'a str>,
}

pub(crate) enum ProviderSqlPlan {
    Statements(Vec<String>),
    RequiresTableRebuild,
}

/// Provider-aware SQL fragments shared across DDL generation and migration flows.
pub(crate) struct ProviderStrategy {
    provider: DatabaseProvider,
}

impl ProviderStrategy {
    pub(crate) fn new(provider: DatabaseProvider) -> Self {
        Self { provider }
    }

    /// Quote a table in the statement's table position, qualifying it with its
    /// schema when it has one.
    pub(crate) fn quote_table(&self, table: &TableName) -> String {
        nautilus_core::ident::quote_table_name(table, self.provider.identifier_quote())
    }

    pub(crate) fn drop_table_sql(&self, table: &TableName, cascade: bool) -> String {
        if self.provider == DatabaseProvider::Postgres && cascade {
            format!("DROP TABLE IF EXISTS {} CASCADE", self.quote_table(table))
        } else {
            format!("DROP TABLE IF EXISTS {}", self.quote_table(table))
        }
    }

    pub(crate) fn supports_user_defined_types(&self) -> bool {
        self.provider == DatabaseProvider::Postgres
    }

    pub(crate) fn drop_column_sql(&self, table: &TableName, column: &str) -> String {
        format!(
            "ALTER TABLE {} DROP COLUMN {}",
            self.quote_table(table),
            self.provider.quote_identifier(column)
        )
    }

    pub(crate) fn array_storage_sql(
        &self,
        storage_strategy: Option<StorageStrategy>,
    ) -> Option<&'static str> {
        match (self.provider, storage_strategy) {
            (DatabaseProvider::Mysql, Some(StorageStrategy::Json)) => Some("JSON"),
            (DatabaseProvider::Sqlite, Some(StorageStrategy::Json)) => Some("TEXT"),
            _ => None,
        }
    }

    pub(crate) fn composite_storage_sql(
        &self,
        storage_strategy: Option<StorageStrategy>,
    ) -> Option<&'static str> {
        self.array_storage_sql(storage_strategy)
    }

    pub(crate) fn native_array_support_error(&self) -> String {
        format!(
            "{} does not support native array types. Use @store(json) attribute.",
            self.provider_name()
        )
    }

    pub(crate) fn native_composite_support_error(&self, type_name: &str, is_array: bool) -> String {
        let subject = if is_array {
            "native composite type arrays"
        } else {
            "native composite types"
        };
        format!(
            "{} does not support {}. Add @store(Json) to the field using type '{}'.",
            self.provider_name(),
            subject,
            type_name,
        )
    }

    pub(crate) fn alter_column_type_sql(
        &self,
        alteration: AlterColumnType<'_>,
    ) -> Result<ProviderSqlPlan> {
        let q = |name: &str| self.provider.quote_identifier(name);

        match self.provider {
            DatabaseProvider::Postgres => Ok(ProviderSqlPlan::Statements(vec![format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                self.quote_table(alteration.table),
                q(alteration.column),
                alteration.target_type,
            )])),
            DatabaseProvider::Mysql => {
                let col_def = alteration.full_column_definition.ok_or_else(|| {
                    MigrationError::Other(format!(
                        "Missing full column definition for MySQL type rewrite on {}.{}",
                        alteration.table, alteration.column
                    ))
                })?;
                Ok(ProviderSqlPlan::Statements(vec![format!(
                    "ALTER TABLE {} MODIFY COLUMN {}",
                    self.quote_table(alteration.table),
                    col_def,
                )]))
            }
            DatabaseProvider::Sqlite => Ok(ProviderSqlPlan::RequiresTableRebuild),
        }
    }

    pub(crate) fn alter_column_nullability_sql(
        &self,
        alteration: AlterColumnNullability<'_>,
    ) -> Result<ProviderSqlPlan> {
        let q = |name: &str| self.provider.quote_identifier(name);

        match (self.provider, alteration.now_required) {
            (DatabaseProvider::Postgres, false) => Ok(ProviderSqlPlan::Statements(vec![format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                self.quote_table(alteration.table),
                q(alteration.column),
            )])),
            (DatabaseProvider::Postgres, true) => {
                if alteration.is_generated {
                    return Ok(ProviderSqlPlan::Statements(vec![format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                        self.quote_table(alteration.table),
                        q(alteration.column),
                    )]));
                }

                if let Some(default_sql) = alteration.default_sql {
                    return Ok(ProviderSqlPlan::Statements(vec![
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                            self.quote_table(alteration.table),
                            q(alteration.column),
                            default_sql,
                        ),
                        format!(
                            "UPDATE {} SET {} = {} WHERE {} IS NULL",
                            self.quote_table(alteration.table),
                            q(alteration.column),
                            default_sql,
                            q(alteration.column),
                        ),
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                            self.quote_table(alteration.table),
                            q(alteration.column),
                        ),
                    ]));
                }

                Err(MigrationError::UnsupportedChange(format!(
                    "Column {}.{} cannot be made NOT NULL: no @default() is defined. \
                     Add a default value to the field in your schema before re-running \
                     `db push`, or manually backfill NULLs and apply the constraint by hand.",
                    alteration.table, alteration.column
                )))
            }
            (DatabaseProvider::Mysql, _) => {
                let col_def = alteration.full_column_definition.ok_or_else(|| {
                    MigrationError::Other(format!(
                        "Missing full column definition for MySQL nullability change on {}.{}",
                        alteration.table, alteration.column
                    ))
                })?;
                Ok(ProviderSqlPlan::Statements(vec![format!(
                    "ALTER TABLE {} MODIFY COLUMN {}",
                    self.quote_table(alteration.table),
                    col_def,
                )]))
            }
            (DatabaseProvider::Sqlite, _) => Ok(ProviderSqlPlan::RequiresTableRebuild),
        }
    }

    pub(crate) fn alter_column_default_sql(
        &self,
        alteration: AlterColumnDefault<'_>,
    ) -> Result<ProviderSqlPlan> {
        let q = |name: &str| self.provider.quote_identifier(name);

        match self.provider {
            DatabaseProvider::Postgres => {
                if alteration.new_default.is_none() && alteration.preserve_implicit_default {
                    return Ok(ProviderSqlPlan::Statements(vec![]));
                }

                Ok(ProviderSqlPlan::Statements(vec![
                    if let Some(default_sql) = alteration.new_default {
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                            self.quote_table(alteration.table),
                            q(alteration.column),
                            default_sql,
                        )
                    } else {
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                            self.quote_table(alteration.table),
                            q(alteration.column),
                        )
                    },
                ]))
            }
            DatabaseProvider::Mysql => {
                let col_def = alteration.full_column_definition.ok_or_else(|| {
                    MigrationError::Other(format!(
                        "Missing full column definition for MySQL default change on {}.{}",
                        alteration.table, alteration.column
                    ))
                })?;
                Ok(ProviderSqlPlan::Statements(vec![format!(
                    "ALTER TABLE {} MODIFY COLUMN {}",
                    self.quote_table(alteration.table),
                    col_def,
                )]))
            }
            DatabaseProvider::Sqlite => Ok(ProviderSqlPlan::RequiresTableRebuild),
        }
    }

    pub(crate) fn reverse_nullability_change_sql(
        &self,
        table: &TableName,
        column: &str,
        now_required: bool,
    ) -> Option<Vec<String>> {
        let q = |name: &str| self.provider.quote_identifier(name);

        match (self.provider, now_required) {
            (DatabaseProvider::Postgres, true) => Some(vec![format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                self.quote_table(table),
                q(column),
            )]),
            (DatabaseProvider::Postgres, false) => Some(vec![format!(
                "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                self.quote_table(table),
                q(column),
            )]),
            _ => None,
        }
    }

    pub(crate) fn reverse_default_change_sql(
        &self,
        table: &TableName,
        column: &str,
        old_default: Option<&str>,
    ) -> Option<Vec<String>> {
        let q = |name: &str| self.provider.quote_identifier(name);

        match self.provider {
            DatabaseProvider::Postgres => Some(vec![if let Some(default_sql) = old_default {
                format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                    self.quote_table(table),
                    q(column),
                    default_sql,
                )
            } else {
                format!(
                    "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                    self.quote_table(table),
                    q(column),
                )
            }]),
            DatabaseProvider::Mysql | DatabaseProvider::Sqlite => None,
        }
    }

    pub(crate) fn reverse_column_type_sql(
        &self,
        table: &TableName,
        column: &str,
        old_type: &str,
    ) -> Option<Vec<String>> {
        let q = |name: &str| self.provider.quote_identifier(name);

        match self.provider {
            DatabaseProvider::Postgres => Some(vec![format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                self.quote_table(table),
                q(column),
                old_type,
            )]),
            DatabaseProvider::Mysql | DatabaseProvider::Sqlite => None,
        }
    }

    fn provider_name(&self) -> &'static str {
        match self.provider {
            DatabaseProvider::Postgres => "PostgreSQL",
            DatabaseProvider::Sqlite => "SQLite",
            DatabaseProvider::Mysql => "MySQL",
        }
    }
}

#[cfg(test)]
mod tests;
