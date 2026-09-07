use super::{DatabaseProvider, ProviderStrategy};
use nautilus_core::TableName;

impl ProviderStrategy {
    pub(crate) fn drop_constraint_sql(&self, table: &TableName, name: &str) -> String {
        format!(
            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}",
            self.quote_table(table),
            self.provider.quote_identifier(name),
        )
    }

    /// SQLite callers must rebuild the table or report an unsupported reversal.
    pub(crate) fn drop_foreign_key_sql(&self, table: &TableName, name: &str) -> Option<String> {
        match self.provider {
            DatabaseProvider::Postgres => Some(self.drop_constraint_sql(table, name)),
            DatabaseProvider::Mysql => Some(format!(
                "ALTER TABLE {} DROP FOREIGN KEY {}",
                self.quote_table(table),
                self.provider.quote_identifier(name),
            )),
            DatabaseProvider::Sqlite => None,
        }
    }
}

pub(crate) fn check_constraint_name(table: &str, column: Option<&str>) -> String {
    match column {
        Some(col) => format!("chk_{}_{}", table, col),
        None => format!("chk_{}", table),
    }
}
