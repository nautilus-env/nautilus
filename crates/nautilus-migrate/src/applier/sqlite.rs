use super::DiffApplier;
use crate::error::{MigrationError, Result};
use nautilus_core::TableName;
use nautilus_schema::ir::ResolvedFieldType;
use std::collections::HashSet;

impl DiffApplier<'_> {
    /// Rebuild the table through a temporary copy in one transaction.
    ///
    /// Reuse the table renderer under a temporary physical name. Copy only
    /// surviving writable columns: SQLite rejects INSERTs that name generated
    /// columns and recomputes their values itself.
    pub(super) fn sqlite_rebuild(&self, table: &TableName) -> Result<Vec<String>> {
        let model = self.find_model(table)?;
        let live_table = self
            .live
            .tables
            .get(table)
            .ok_or_else(|| MigrationError::Other(format!("Live table not found: {}", table)))?;

        let tmp_name = format!("__tmp_{}", table.name);
        let mut tmp_model = model.clone();
        tmp_model.db_name = tmp_name.clone();
        let create_tmp = self.ddl.generate_create_table(&tmp_model, self.schema)?;
        let target_cols: HashSet<&str> = model
            .fields
            .iter()
            .filter(|f| !matches!(f.field_type, ResolvedFieldType::Relation(_)))
            .filter(|f| f.computed.is_none())
            .map(|f| f.db_name.as_str())
            .collect();

        let common_cols: Vec<String> = live_table
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .filter(|&name| target_cols.contains(name))
            .map(|name| self.q(name))
            .collect();

        let cols_sql = common_cols.join(", ");

        Ok(vec![
            format!("DROP TABLE IF EXISTS {}", self.q(&tmp_name)),
            create_tmp,
            format!(
                "INSERT INTO {} ({}) SELECT {} FROM {}",
                self.q(&tmp_name),
                cols_sql,
                cols_sql,
                self.q_table(table),
            ),
            format!("DROP TABLE {}", self.q_table(table)),
            format!(
                "ALTER TABLE {} RENAME TO {}",
                self.q(&tmp_name),
                self.q_table(table),
            ),
        ])
    }
}
