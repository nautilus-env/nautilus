use super::DiffApplier;
use crate::error::Result;
use crate::provider::CreateIndex;
use crate::utils::default_index_name;
use nautilus_core::TableName;

impl DiffApplier<'_> {
    pub(super) fn sql_create_index(
        &self,
        table: &TableName,
        columns: &[String],
        unique: bool,
        kind: &nautilus_schema::ir::IndexKind,
        index_name: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<String>> {
        let idx_name = index_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_index_name(&table.name, columns));
        Ok(vec![self.strategy().create_index_sql(CreateIndex {
            table,
            name: &idx_name,
            columns,
            unique,
            kind,
            if_not_exists: true,
            predicate,
        })])
    }

    pub(super) fn sql_drop_index(
        &self,
        table: &TableName,
        index_name: &str,
    ) -> Result<Vec<String>> {
        Ok(vec![self.strategy().drop_index_sql(table, index_name)])
    }
}
