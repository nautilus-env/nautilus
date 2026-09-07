use super::DiffApplier;
use crate::ddl::DatabaseProvider;
use crate::error::{MigrationError, Result};
use nautilus_core::TableName;
use nautilus_schema::ir::ModelIr;

impl DiffApplier<'_> {
    pub(super) fn sql_create_table(&self, model: &ModelIr) -> Result<Vec<String>> {
        let mut stmts = vec![self.ddl.generate_create_table(model, self.schema)?];
        stmts.extend(self.ddl.generate_create_indexes_for_model(model));
        Ok(stmts)
    }

    pub(super) fn sql_drop_table(&self, name: &TableName) -> Result<Vec<String>> {
        Ok(vec![self.strategy().drop_table_sql(
            name,
            self.provider == DatabaseProvider::Postgres,
        )])
    }

    pub(super) fn sql_alter_primary_key(&self, table: &TableName) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let model = self.find_model(table)?;
                let pk_cols = self.pk_col_list(model)?;
                let drop_stmt = match self.provider {
                    DatabaseProvider::Postgres => self
                        .strategy()
                        .drop_constraint_sql(table, &format!("{}_pkey", table.name)),
                    _ => format!("ALTER TABLE {} DROP PRIMARY KEY", self.q_table(table)),
                };
                Ok(vec![
                    drop_stmt,
                    format!(
                        "ALTER TABLE {} ADD PRIMARY KEY ({})",
                        self.q_table(table),
                        pk_cols,
                    ),
                ])
            }
        }
    }

    /// Comma-separated quoted primary-key column list for a model.
    fn pk_col_list(&self, model: &ModelIr) -> Result<String> {
        let cols: Vec<String> = model
            .primary_key
            .fields()
            .iter()
            .map(|name| {
                let field = model.find_field(name).ok_or_else(|| {
                    MigrationError::Other(format!(
                        "primary key field '{}' not found in model '{}'",
                        name, model.logical_name
                    ))
                })?;
                Ok(self.q(&field.db_name))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(cols.join(", "))
    }
}
