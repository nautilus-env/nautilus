use super::{cannot_reverse, missing_snapshot, ChangeReverser};
use crate::ddl::DatabaseProvider;
use nautilus_core::TableName;

impl ChangeReverser<'_> {
    pub(super) fn reverse_added_column(&self, table: &TableName, column: &str) -> Vec<String> {
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                vec![self.strategy.drop_column_sql(table, column)]
            }
            DatabaseProvider::Sqlite => {
                cannot_reverse(format!("ADD COLUMN on SQLite: {}.{}", table, column))
            }
        }
    }

    pub(super) fn reverse_dropped_column(&self, table: &TableName, column: &str) -> Vec<String> {
        let missing = || missing_snapshot(format!("column {}.{} was dropped", table, column));
        let Some(live_column) = self
            .live
            .tables
            .get(table)
            .and_then(|live_table| live_table.columns.iter().find(|c| c.name == column))
        else {
            return missing();
        };

        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let not_null = if live_column.nullable {
                    ""
                } else {
                    " NOT NULL"
                };
                let default_clause = live_column
                    .default_value
                    .as_deref()
                    .map(|default| format!(" DEFAULT {}", default))
                    .unwrap_or_default();

                vec![format!(
                    "ALTER TABLE {} ADD COLUMN {} {}{}{}",
                    self.quote_table(table),
                    self.quote(column),
                    live_column.col_type.to_uppercase(),
                    not_null,
                    default_clause,
                )]
            }
            DatabaseProvider::Sqlite => {
                cannot_reverse(format!("dropped column on SQLite: {}.{}", table, column))
            }
        }
    }

    /// Restore the column's `AUTO_INCREMENT` state as the live snapshot recorded
    /// it, by restating the definition MySQL had before the change.
    pub(super) fn reverse_auto_increment_change(
        &self,
        table: &TableName,
        column: &str,
    ) -> Vec<String> {
        if self.provider != DatabaseProvider::Mysql {
            return cannot_reverse(format!("AUTO_INCREMENT change: {}.{}", table, column));
        }

        let Some(live_column) = self
            .live
            .tables
            .get(table)
            .and_then(|live_table| live_table.columns.iter().find(|c| c.name == column))
        else {
            return missing_snapshot(format!("AUTO_INCREMENT change on {}.{}", table, column));
        };

        let not_null = if live_column.nullable {
            ""
        } else {
            " NOT NULL"
        };
        let auto_increment = if live_column.auto_increment {
            " AUTO_INCREMENT"
        } else {
            ""
        };

        vec![format!(
            "ALTER TABLE {} MODIFY COLUMN {} {}{}{}",
            self.quote_table(table),
            self.quote(column),
            live_column.col_type.to_uppercase(),
            not_null,
            auto_increment,
        )]
    }

    pub(super) fn reverse_type_change(
        &self,
        table: &TableName,
        column: &str,
        from: &str,
    ) -> Vec<String> {
        self.strategy
            .reverse_column_type_sql(table, column, from)
            .unwrap_or_else(|| {
                cannot_reverse(format!(
                    "TYPE change on {}.{} (was {})",
                    table, column, from
                ))
            })
    }

    pub(super) fn reverse_nullability_change(
        &self,
        table: &TableName,
        column: &str,
        now_required: bool,
    ) -> Vec<String> {
        self.strategy
            .reverse_nullability_change_sql(table, column, now_required)
            .unwrap_or_else(|| cannot_reverse(format!("nullability change: {}.{}", table, column)))
    }

    pub(super) fn reverse_default_change(
        &self,
        table: &TableName,
        column: &str,
        from: Option<&str>,
    ) -> Vec<String> {
        self.strategy
            .reverse_default_change_sql(table, column, from)
            .unwrap_or_else(|| cannot_reverse(format!("DEFAULT change: {}.{}", table, column)))
    }
}
