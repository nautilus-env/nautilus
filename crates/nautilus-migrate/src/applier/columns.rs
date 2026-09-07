use super::DiffApplier;
use crate::ddl::DatabaseProvider;
use crate::error::{MigrationError, Result};
use crate::provider::{AlterColumnDefault, AlterColumnNullability, AlterColumnType};
use nautilus_core::TableName;
use nautilus_schema::ir::{DefaultValue, FieldIr};

impl DiffApplier<'_> {
    pub(super) fn sql_add_column(&self, table: &TableName, field: &FieldIr) -> Result<Vec<String>> {
        if field.is_required && field.default_value.is_none() && field.computed.is_none() {
            return Err(MigrationError::UnsupportedChange(format!(
                "Column {}.{} is NOT NULL but has no @default(). \
                 Add a default value to the field in your schema before re-running \
                 `db push`, or make the field optional.",
                table, field.db_name
            )));
        }
        let col_def = self.column_definition(table, field)?;
        Ok(vec![format!(
            "ALTER TABLE {} ADD COLUMN {}",
            self.q_table(table),
            col_def,
        )])
    }

    pub(super) fn sql_drop_column(&self, table: &TableName, column: &str) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                Ok(vec![self.strategy().drop_column_sql(table, column)])
            }
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
        }
    }

    pub(super) fn sql_alter_column_type(
        &self,
        table: &TableName,
        column: &str,
    ) -> Result<Vec<String>> {
        let field = self.find_field(table, column)?;
        let type_sql = self.ddl.column_type_sql(field)?;
        let col_def = self.mysql_full_col_def(table, field)?;

        self.materialize_provider_plan(
            table,
            self.strategy().alter_column_type_sql(AlterColumnType {
                table,
                column,
                target_type: &type_sql,
                full_column_definition: col_def.as_deref(),
            })?,
        )
    }

    pub(super) fn sql_alter_column_nullability(
        &self,
        table: &TableName,
        column: &str,
        now_required: bool,
    ) -> Result<Vec<String>> {
        let field = self.find_field(table, column)?;
        let default_sql = match &field.default_value {
            Some(default)
                if !matches!(
                    default,
                    DefaultValue::Function(func) if func.name == "autoincrement"
                ) =>
            {
                Some(
                    self.ddl
                        .generate_default_value(default, &field.field_type)?,
                )
            }
            _ => None,
        };
        let col_def = self.mysql_full_col_def(table, field)?;

        self.materialize_provider_plan(
            table,
            self.strategy()
                .alter_column_nullability_sql(AlterColumnNullability {
                    table,
                    column,
                    now_required,
                    is_generated: field.computed.is_some(),
                    default_sql: default_sql.as_deref(),
                    full_column_definition: col_def.as_deref(),
                })?,
        )
    }

    pub(super) fn sql_alter_column_default(
        &self,
        table: &TableName,
        column: &str,
        new_default: Option<&str>,
    ) -> Result<Vec<String>> {
        let field = if self.provider == DatabaseProvider::Mysql || new_default.is_none() {
            Some(self.find_field(table, column)?)
        } else {
            None
        };
        let preserve_implicit_default = field.is_some_and(|field| {
            matches!(
                &field.default_value,
                Some(DefaultValue::Function(func)) if func.name == "autoincrement"
            )
        });
        let col_def = if self.provider == DatabaseProvider::Mysql {
            Some(self.full_col_def(
                table,
                field.expect("field required for MySQL default change"),
            )?)
        } else {
            None
        };

        self.materialize_provider_plan(
            table,
            self.strategy()
                .alter_column_default_sql(AlterColumnDefault {
                    table,
                    column,
                    new_default,
                    preserve_implicit_default,
                    full_column_definition: col_def.as_deref(),
                })?,
        )
    }

    /// Add or drop MySQL's `AUTO_INCREMENT` on an existing column.
    ///
    /// MySQL has no dedicated statement for the attribute; it is restated as
    /// part of a full `MODIFY COLUMN`, which [`Self::full_col_def`] already
    /// renders with or without `AUTO_INCREMENT` depending on the target schema.
    pub(super) fn sql_alter_auto_increment(
        &self,
        table: &TableName,
        column: &str,
    ) -> Result<Vec<String>> {
        if self.provider != DatabaseProvider::Mysql {
            return Err(MigrationError::UnsupportedChange(format!(
                "AUTO_INCREMENT is a MySQL column attribute; {}.{} cannot be altered on {:?}",
                table, column, self.provider
            )));
        }

        let field = self.find_field(table, column)?;
        Ok(vec![format!(
            "ALTER TABLE {} MODIFY COLUMN {}",
            self.q_table(table),
            self.full_col_def(table, field)?,
        )])
    }

    /// Generated columns cannot be altered in-place on any provider, so the
    /// column is dropped and re-added with the new expression.
    pub(super) fn sql_alter_computed_column(
        &self,
        table: &TableName,
        field: &FieldIr,
    ) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let col_def = self.column_definition(table, field)?;
                Ok(vec![
                    self.strategy().drop_column_sql(table, &field.db_name),
                    format!("ALTER TABLE {} ADD COLUMN {}", self.q_table(table), col_def,),
                ])
            }
        }
    }

    /// Whether `field` is the table's single-column `autoincrement()` primary
    /// key, which MySQL and SQLite spell out in the column definition.
    fn is_autoincrement_pk(&self, table: &TableName, field: &FieldIr) -> bool {
        self.find_model(table).is_ok_and(|model| {
            crate::ddl::DdlGenerator::autoincrement_primary_key(model)
                .is_some_and(|name| name == field.logical_name)
        })
    }

    /// Render the full column definition for `field`, erroring out with the
    /// table/column name when the generator cannot produce one.
    fn column_definition(&self, table: &TableName, field: &FieldIr) -> Result<String> {
        self.ddl
            .generate_column_definition(field, self.schema, self.is_autoincrement_pk(table, field))?
            .ok_or_else(|| {
                MigrationError::UnsupportedChange(format!(
                    "Cannot generate column definition for {}.{}",
                    table, field.db_name
                ))
            })
    }

    /// MySQL `ALTER COLUMN` restates the whole column definition; the other
    /// providers do not need it.
    fn mysql_full_col_def(&self, table: &TableName, field: &FieldIr) -> Result<Option<String>> {
        if self.provider == DatabaseProvider::Mysql {
            Ok(Some(self.full_col_def(table, field)?))
        } else {
            Ok(None)
        }
    }

    /// Generate the full column definition string for a field.
    /// Used for MySQL `MODIFY COLUMN` which needs the complete definition.
    fn full_col_def(&self, table: &TableName, field: &FieldIr) -> Result<String> {
        self.ddl
            .generate_column_definition(field, self.schema, self.is_autoincrement_pk(table, field))?
            .ok_or_else(|| {
                MigrationError::UnsupportedChange(format!(
                    "Cannot generate column definition for field {}",
                    field.db_name,
                ))
            })
    }
}
