use super::{managed_models, DatabaseProvider, DdlGenerator};
use crate::error::Result;
use crate::live::model_table;
use crate::provider::ProviderStrategy;
use nautilus_core::TableName;
use nautilus_schema::ir::{ModelIr, SchemaIr};

impl DdlGenerator {
    /// Drop the inspected tables, including ones removed from the target schema.
    ///
    /// PostgreSQL uses CASCADE; MySQL disables foreign-key checks around the
    /// drops. SQLite defers checks until commit because `PRAGMA foreign_keys`
    /// cannot change enforcement inside the transaction. Managed enum/composite
    /// types are dropped after the tables.
    pub fn generate_drop_live_tables(&self, live: &crate::live::LiveSchema) -> Vec<String> {
        let mut statements: Vec<String> = Vec::new();
        let strategy = ProviderStrategy::new(self.provider);

        if live.tables.is_empty()
            && (!strategy.supports_user_defined_types()
                || (live.enums.is_empty() && live.composite_types.is_empty()))
        {
            return statements;
        }

        let mut names: Vec<&TableName> = live.tables.keys().collect();
        names.sort_unstable();

        match self.provider {
            DatabaseProvider::Postgres => {
                for name in &names {
                    statements.push(strategy.drop_table_sql(name, true));
                }
            }
            DatabaseProvider::Mysql => {
                statements.push("SET FOREIGN_KEY_CHECKS=0".to_string());
                for name in &names {
                    statements.push(strategy.drop_table_sql(name, false));
                }
                statements.push("SET FOREIGN_KEY_CHECKS=1".to_string());
            }
            DatabaseProvider::Sqlite => {
                statements.push("PRAGMA defer_foreign_keys = ON".to_string());
                for name in &names {
                    statements.push(strategy.drop_table_sql(name, false));
                }
            }
        }

        if strategy.supports_user_defined_types() {
            let mut ct_names: Vec<&str> = live
                .composite_types
                .values()
                .map(|ct| ct.name.as_str())
                .collect();
            ct_names.sort_unstable();
            for name in &ct_names {
                statements.push(strategy.drop_type_sql(name));
            }

            let mut enum_names: Vec<&str> = live.enums.keys().map(String::as_str).collect();
            enum_names.sort_unstable();
            for name in &enum_names {
                statements.push(strategy.drop_type_sql(name));
            }
        }

        statements
    }

    /// Clear managed tables in reverse foreign-key dependency order.
    ///
    /// PostgreSQL uses TRUNCATE with RESTART IDENTITY CASCADE; MySQL wraps
    /// TRUNCATEs in disabled foreign-key checks. SQLite uses DELETE and clears
    /// the corresponding sqlite_sequence entries to reset generated keys.
    pub fn generate_truncate_tables(&self, schema: &SchemaIr) -> Result<Vec<String>> {
        let all_models = managed_models(schema);
        let sorted: Vec<&ModelIr> = crate::diff::topo_sort_models(&all_models)
            .into_iter()
            .rev()
            .collect();

        let mut statements: Vec<String> = Vec::new();

        match self.provider {
            DatabaseProvider::Postgres => {
                for model in &sorted {
                    statements.push(format!(
                        "TRUNCATE TABLE {} RESTART IDENTITY CASCADE",
                        self.quote_table(&model_table(model))
                    ));
                }
            }
            DatabaseProvider::Mysql => {
                statements.push("SET FOREIGN_KEY_CHECKS=0".to_string());
                for model in &sorted {
                    statements.push(format!(
                        "TRUNCATE TABLE {}",
                        self.quote_table(&model_table(model))
                    ));
                }
                statements.push("SET FOREIGN_KEY_CHECKS=1".to_string());
            }
            DatabaseProvider::Sqlite => {
                for model in &sorted {
                    statements.push(format!(
                        "DELETE FROM {}",
                        self.quote_table(&model_table(model))
                    ));
                    statements.push(format!(
                        "DELETE FROM sqlite_sequence WHERE name = '{}'",
                        model.db_name.replace('\'', "''")
                    ));
                }
            }
        }

        Ok(statements)
    }

    /// Generate statements to delete all rows from every table currently present
    /// in the live database, preserving the table structure.
    ///
    /// Unlike [`Self::generate_truncate_tables`] (which uses the schema IR), this
    /// method operates on the inspected database state so it also clears tables
    /// that still exist live but have already been removed from the schema file.
    pub fn generate_truncate_live_tables(&self, live: &crate::live::LiveSchema) -> Vec<String> {
        let table_names = live_table_names_for_truncate(live);
        let mut statements: Vec<String> = Vec::new();

        match self.provider {
            DatabaseProvider::Postgres => {
                for table_name in &table_names {
                    statements.push(format!(
                        "TRUNCATE TABLE {} RESTART IDENTITY CASCADE",
                        self.quote_table(table_name)
                    ));
                }
            }
            DatabaseProvider::Mysql => {
                if !table_names.is_empty() {
                    statements.push("SET FOREIGN_KEY_CHECKS=0".to_string());
                    for table_name in &table_names {
                        statements.push(format!("TRUNCATE TABLE {}", self.quote_table(table_name)));
                    }
                    statements.push("SET FOREIGN_KEY_CHECKS=1".to_string());
                }
            }
            DatabaseProvider::Sqlite => {
                for table_name in &table_names {
                    statements.push(format!("DELETE FROM {}", self.quote_table(table_name)));
                    statements.push(format!(
                        "DELETE FROM sqlite_sequence WHERE name = '{}'",
                        table_name.name.replace('\'', "''")
                    ));
                }
            }
        }

        statements
    }
}

fn live_table_names_for_truncate(live: &crate::live::LiveSchema) -> Vec<TableName> {
    use std::collections::{BTreeSet, HashMap};

    let mut remaining: BTreeSet<TableName> = live.tables.keys().cloned().collect();
    let mut dependencies: HashMap<TableName, BTreeSet<TableName>> = live
        .tables
        .iter()
        .map(|(name, table)| {
            let deps = table
                .foreign_keys
                .iter()
                .map(|fk| fk.referenced_table.clone())
                .filter(|dep| dep != name && live.tables.contains_key(dep))
                .collect::<BTreeSet<_>>();
            (name.clone(), deps)
        })
        .collect();

    let mut ordered = Vec::new();

    while !remaining.is_empty() {
        let ready: Vec<TableName> = remaining
            .iter()
            .filter(|name| {
                dependencies
                    .get(*name)
                    .map(|deps| deps.is_empty())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        if ready.is_empty() {
            ordered.extend(remaining);
            break;
        }

        for name in &ready {
            remaining.remove(name);
        }

        for deps in dependencies.values_mut() {
            for name in &ready {
                deps.remove(name);
            }
        }

        ordered.extend(ready);
    }

    ordered.reverse();
    ordered
}
