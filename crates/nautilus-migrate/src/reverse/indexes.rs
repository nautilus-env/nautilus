use super::snapshot::create_index_sql_from_live;
use super::ChangeReverser;
use crate::provider::CreateIndex;
use nautilus_core::TableName;
use nautilus_schema::ir::IndexKind;

impl ChangeReverser<'_> {
    pub(super) fn reverse_index_added(
        &self,
        table: &TableName,
        columns: &[String],
        index_name: Option<&str>,
    ) -> Vec<String> {
        let index_name = index_name
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!("idx_{}_{}", table, columns.join("_")));
        vec![self.strategy.drop_index_sql(table, &index_name)]
    }

    pub(super) fn reverse_index_dropped(
        &self,
        table: &TableName,
        columns: &[String],
        unique: bool,
        index_name: &str,
    ) -> Vec<String> {
        if let Some(live_index) = self
            .live
            .tables
            .get(table)
            .and_then(|t| t.indexes.iter().find(|i| i.name == *index_name))
        {
            return vec![create_index_sql_from_live(table, live_index, self.provider)];
        }

        vec![self.strategy.create_index_sql(CreateIndex {
            table,
            name: index_name,
            columns,
            unique,
            kind: &IndexKind::Default,
            if_not_exists: true,
            predicate: None,
        })]
    }
}
