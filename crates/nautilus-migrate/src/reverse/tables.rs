use super::snapshot::create_table_sql_from_live;
use super::{cannot_reverse, missing_snapshot, ChangeReverser};
use crate::ddl::DatabaseProvider;
use nautilus_core::TableName;

impl ChangeReverser<'_> {
    pub(super) fn reverse_new_table(&self, table: &TableName) -> Vec<String> {
        vec![self
            .strategy
            .drop_table_sql(table, self.provider == DatabaseProvider::Postgres)]
    }

    pub(super) fn reverse_dropped_table(&self, table: &TableName) -> Vec<String> {
        match self.live.tables.get(table) {
            Some(live_table) => create_table_sql_from_live(live_table, self.provider),
            None => missing_snapshot(format!("table {} was dropped", table)),
        }
    }

    pub(super) fn reverse_primary_key_change(&self, table: &TableName) -> Vec<String> {
        let Some(live_table) = self.live.tables.get(table) else {
            return cannot_reverse(format!("PRIMARY KEY change on {}: no live snapshot", table));
        };
        if live_table.primary_key.is_empty() {
            return cannot_reverse(format!("PRIMARY KEY change on {}: no live PK info", table));
        }

        let pk_cols = self.quote_all(&live_table.primary_key);
        match self.provider {
            DatabaseProvider::Postgres => vec![
                self.strategy
                    .drop_constraint_sql(table, &format!("{}_pkey", table.name)),
                format!(
                    "ALTER TABLE {} ADD PRIMARY KEY ({})",
                    self.quote_table(table),
                    pk_cols
                ),
            ],
            DatabaseProvider::Mysql => vec![format!(
                "ALTER TABLE {} DROP PRIMARY KEY, ADD PRIMARY KEY ({})",
                self.quote_table(table),
                pk_cols,
            )],
            DatabaseProvider::Sqlite => cannot_reverse(format!(
                "PRIMARY KEY change on {} (SQLite requires table rebuild)",
                table
            )),
        }
    }
}
