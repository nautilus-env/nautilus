use super::{DatabaseProvider, ProviderStrategy};
use crate::normalize::predicates::normalize_check_expr;
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

/// Name of a field-level `@check` constraint, which the inspectors use to give
/// the expression back to its column.
pub(crate) fn column_check_constraint_name(table: &str, column: &str) -> String {
    format!("chk_{}_{}", table, column)
}

/// Name of a model-level `@@check` constraint: the table name and a short hash
/// of the expression's comparison form.
///
/// A model may declare several, and reformatting a predicate in the schema
/// does not rename it. A constraint already in the database is always dropped
/// by the name it has there (see [`crate::LiveSchema::check_constraint_names`]),
/// so older ones named `chk_<table>` keep working.
pub(crate) fn table_check_constraint_name(table: &str, expr: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(normalize_check_expr(expr).as_bytes());
    let hash: String = hasher
        .finalize()
        .iter()
        .take(4)
        .map(|b| format!("{:02x}", b))
        .collect();
    format!("chk_{}_{}", table, hash)
}
