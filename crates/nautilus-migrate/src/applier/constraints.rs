use super::DiffApplier;
use crate::ddl::DatabaseProvider;
use crate::error::Result;
use crate::provider::{column_check_constraint_name, table_check_constraint_name};
use nautilus_core::TableName;

/// Parameters for a `ADD CONSTRAINT ... FOREIGN KEY` statement, mirroring the
/// fields of [`Change::ForeignKeyAdded`](crate::Change::ForeignKeyAdded).
pub(super) struct AddForeignKey<'a> {
    pub(super) table: &'a TableName,
    pub(super) constraint_name: &'a str,
    pub(super) columns: &'a [String],
    pub(super) referenced_table: &'a TableName,
    pub(super) referenced_columns: &'a [String],
    /// ON DELETE action, or `None` for the database default.
    pub(super) on_delete: Option<&'a str>,
    /// ON UPDATE action, or `None` for the database default.
    pub(super) on_update: Option<&'a str>,
}

impl DiffApplier<'_> {
    pub(super) fn sql_alter_check(
        &self,
        table: &TableName,
        column: Option<&str>,
        old_expr: Option<&str>,
        new_expr: Option<&str>,
    ) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let mut stmts = Vec::new();

                if let Some(old) = old_expr {
                    let name = match column {
                        Some(col) => column_check_constraint_name(&table.name, col),
                        None => self.live.table_check_name(table, old).map_or_else(
                            || table_check_constraint_name(&table.name, old),
                            str::to_string,
                        ),
                    };
                    stmts.push(match self.provider {
                        DatabaseProvider::Mysql => format!(
                            "ALTER TABLE {} DROP CHECK {}",
                            self.q_table(table),
                            self.q(&name),
                        ),
                        _ => self.strategy().drop_constraint_sql(table, &name),
                    });
                }

                if let Some(expr) = new_expr {
                    let name = match column {
                        Some(col) => column_check_constraint_name(&table.name, col),
                        None => table_check_constraint_name(&table.name, expr),
                    };
                    stmts.push(self.strategy().add_check_constraint_sql(table, &name, expr));
                }

                Ok(stmts)
            }
        }
    }

    pub(super) fn sql_add_foreign_key(&self, fk: AddForeignKey<'_>) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(fk.table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let mut sql = format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
                    self.q_table(fk.table),
                    self.q(fk.constraint_name),
                    self.quote_join(fk.columns),
                    self.q_table(fk.referenced_table),
                    self.quote_join(fk.referenced_columns),
                );
                if let Some(action) = fk.on_delete {
                    sql.push_str(&format!(" ON DELETE {}", action));
                }
                if let Some(action) = fk.on_update {
                    sql.push_str(&format!(" ON UPDATE {}", action));
                }
                Ok(vec![sql])
            }
        }
    }

    pub(super) fn sql_drop_foreign_key(
        &self,
        table: &TableName,
        constraint_name: &str,
    ) -> Result<Vec<String>> {
        match self.strategy().drop_foreign_key_sql(table, constraint_name) {
            Some(sql) => Ok(vec![sql]),
            None => self.sqlite_rebuild(table),
        }
    }

    fn quote_join(&self, columns: &[String]) -> String {
        columns
            .iter()
            .map(|c| self.q(c))
            .collect::<Vec<_>>()
            .join(", ")
    }
}
