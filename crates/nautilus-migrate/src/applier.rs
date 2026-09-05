//! Diff applier — translates each [`Change`] into one or more SQL statements
//! that, when executed, bring the live database in sync with the target schema.
//!
//! For single-statement operations the caller can use
//! [`Connection::execute`](crate). For multi-statement operations (e.g. SQLite
//! full-table rebuilds or nullable-required with default on Postgres) the
//! caller **must** execute all returned statements inside a single transaction.

use std::collections::HashSet;

use nautilus_core::TableName;
use nautilus_schema::ir::{
    DefaultValue, FieldIr, ModelIr, PostgresExtensionIr, ResolvedFieldType, SchemaIr,
};

use crate::ddl::{DatabaseProvider, DdlGenerator};
use crate::diff::Change;
use crate::error::{MigrationError, Result};
use crate::live::LiveSchema;
use crate::provider::{
    AlterColumnDefault, AlterColumnNullability, AlterColumnType, CreateIndex, ProviderSqlPlan,
    ProviderStrategy,
};

/// Parameters for a `ADD CONSTRAINT ... FOREIGN KEY` statement, mirroring the
/// fields of [`Change::ForeignKeyAdded`].
struct AddForeignKey<'a> {
    table: &'a TableName,
    constraint_name: &'a str,
    columns: &'a [String],
    referenced_table: &'a TableName,
    referenced_columns: &'a [String],
    /// ON DELETE action, or `None` for the database default.
    on_delete: Option<&'a str>,
    /// ON UPDATE action, or `None` for the database default.
    on_update: Option<&'a str>,
}

/// Translates schema [`Change`]s into executable SQL statements.
///
/// # Usage
///
/// ```ignore
/// let applier = DiffApplier::new(provider, &ddl, &schema_ir, &live);
/// // Collect all SQL first, then execute atomically in one transaction.
/// let all_stmts: Vec<String> = changes
///     .iter()
///     .flat_map(|c| applier.sql_for(c).unwrap())
///     .collect();
/// conn.execute_in_transaction(&all_stmts).await?;
/// ```
pub struct DiffApplier<'a> {
    provider: DatabaseProvider,
    ddl: &'a DdlGenerator,
    schema: &'a SchemaIr,
    live: &'a LiveSchema,
}

impl<'a> DiffApplier<'a> {
    /// Create a new applier.
    pub fn new(
        provider: DatabaseProvider,
        ddl: &'a DdlGenerator,
        schema: &'a SchemaIr,
        live: &'a LiveSchema,
    ) -> Self {
        Self {
            provider,
            ddl,
            schema,
            live,
        }
    }

    /// Generate SQL statement(s) for a single [`Change`].
    ///
    /// Returns a `Vec<String>`.  When the vec contains **more than one**
    /// element the caller must execute all statements in a single transaction.
    pub fn sql_for(&self, change: &Change) -> Result<Vec<String>> {
        match change {
            Change::NewTable(model) => self.sql_create_table(model),
            Change::DroppedTable { name } => self.sql_drop_table(name),
            Change::PrimaryKeyChanged { table } => self.sql_alter_primary_key(table),

            Change::AddedColumn { table, field } => self.sql_add_column(table, field),
            Change::DroppedColumn { table, column } => self.sql_drop_column(table, column),
            Change::TypeChanged { table, column, .. } => self.sql_alter_column_type(table, column),
            Change::NullabilityChanged {
                table,
                column,
                now_required,
            } => self.sql_alter_column_nullability(table, column, *now_required),
            Change::DefaultChanged {
                table, column, to, ..
            } => self.sql_alter_column_default(table, column, to.as_deref()),
            Change::AutoIncrementChanged { table, column, .. } => {
                self.sql_alter_auto_increment(table, column)
            }
            Change::ComputedExprChanged { table, field, .. } => {
                self.sql_alter_computed_column(table, field)
            }

            Change::IndexAdded {
                table,
                columns,
                unique,
                kind,
                index_name,
                predicate,
            } => self.sql_create_index(
                table,
                columns,
                *unique,
                kind,
                index_name.as_deref(),
                predicate.as_deref(),
            ),
            Change::IndexDropped {
                table, index_name, ..
            } => self.sql_drop_index(table, index_name),

            Change::CheckChanged {
                table,
                column,
                from,
                to,
            } => self.sql_alter_check(table, column.as_deref(), from.is_some(), to.as_deref()),
            Change::ForeignKeyAdded {
                table,
                constraint_name,
                columns,
                referenced_table,
                referenced_columns,
                on_delete,
                on_update,
            } => self.sql_add_foreign_key(AddForeignKey {
                table,
                constraint_name,
                columns,
                referenced_table,
                referenced_columns,
                on_delete: on_delete.as_deref(),
                on_update: on_update.as_deref(),
            }),
            Change::ForeignKeyDropped {
                table,
                constraint_name,
            } => self.sql_drop_foreign_key(table, constraint_name),

            Change::CreateSchema { name } => self.sql_for_user_type(|this| {
                Ok(vec![format!(
                    "CREATE SCHEMA IF NOT EXISTS {}",
                    this.q(name)
                )])
            }),

            Change::CreateCompositeType { name } => {
                self.sql_for_user_type(|this| this.sql_create_composite_type(name))
            }
            Change::DropCompositeType { name } => {
                self.sql_for_user_type(|this| Ok(vec![this.sql_drop_type(name)]))
            }
            Change::AlterCompositeType {
                name,
                added_fields,
                dropped_fields,
                type_changed_fields,
            } => self.sql_for_user_type(|this| {
                Ok(this.sql_alter_composite_type(
                    name,
                    added_fields,
                    dropped_fields,
                    type_changed_fields,
                ))
            }),
            Change::CreateEnum { name, variants } => {
                self.sql_for_user_type(|this| Ok(vec![this.sql_create_enum(name, variants)]))
            }
            Change::DropEnum { name } => {
                self.sql_for_user_type(|this| Ok(vec![this.sql_drop_type(name)]))
            }
            Change::AlterEnum {
                name,
                added_variants,
                removed_variants,
            } => self.sql_for_user_type(|this| {
                this.sql_alter_enum(name, added_variants, removed_variants)
            }),

            Change::CreateExtension { name, schema } => self.sql_for_user_type(|this| {
                let ext = PostgresExtensionIr {
                    name: name.clone(),
                    schema: schema.clone(),
                };
                Ok(vec![this.ddl.generate_create_extension(&ext)])
            }),
            Change::DropExtension { name } => {
                self.sql_for_user_type(|this| Ok(vec![this.ddl.generate_drop_extension(name)]))
            }
        }
    }

    /// Run `build` only when the provider supports user-defined types and
    /// extensions; otherwise the change is a silent no-op.
    fn sql_for_user_type<F>(&self, build: F) -> Result<Vec<String>>
    where
        F: FnOnce(&Self) -> Result<Vec<String>>,
    {
        if !self.strategy().supports_user_defined_types() {
            return Ok(vec![]);
        }
        build(self)
    }

    fn strategy(&self) -> ProviderStrategy {
        ProviderStrategy::new(self.provider)
    }

    fn sql_create_table(&self, model: &ModelIr) -> Result<Vec<String>> {
        let mut stmts = vec![self.ddl.generate_create_table(model, self.schema)?];
        stmts.extend(self.ddl.generate_create_indexes_for_model(model));
        Ok(stmts)
    }

    fn sql_drop_table(&self, name: &TableName) -> Result<Vec<String>> {
        Ok(vec![self.strategy().drop_table_sql(
            name,
            self.provider == DatabaseProvider::Postgres,
        )])
    }

    fn sql_alter_primary_key(&self, table: &TableName) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let model = self.find_model(table)?;
                let pk_cols = self.pk_col_list(model)?;
                let drop_stmt = match self.provider {
                    DatabaseProvider::Postgres => format!(
                        "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}",
                        self.q_table(table),
                        self.q_segments(&[&table.name, "_pkey"]),
                    ),
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

    fn sql_add_column(&self, table: &TableName, field: &FieldIr) -> Result<Vec<String>> {
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

    fn sql_drop_column(&self, table: &TableName, column: &str) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => Ok(vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                self.q_table(table),
                self.q(column),
            )]),
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
        }
    }

    fn sql_alter_column_type(&self, table: &TableName, column: &str) -> Result<Vec<String>> {
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

    fn sql_alter_column_nullability(
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

    fn sql_alter_column_default(
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
    fn sql_alter_auto_increment(&self, table: &TableName, column: &str) -> Result<Vec<String>> {
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
    fn sql_alter_computed_column(&self, table: &TableName, field: &FieldIr) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let col_def = self.column_definition(table, field)?;
                Ok(vec![
                    format!(
                        "ALTER TABLE {} DROP COLUMN {}",
                        self.q_table(table),
                        self.q(&field.db_name),
                    ),
                    format!("ALTER TABLE {} ADD COLUMN {}", self.q_table(table), col_def,),
                ])
            }
        }
    }

    fn sql_create_index(
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
            .unwrap_or_else(|| index_name_auto(table, columns));
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

    fn sql_drop_index(&self, table: &TableName, index_name: &str) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Sqlite => {
                Ok(vec![format!("DROP INDEX IF EXISTS {}", self.q(index_name))])
            }
            DatabaseProvider::Mysql => Ok(vec![format!(
                "DROP INDEX {} ON {}",
                self.q(index_name),
                self.q_table(table),
            )]),
        }
    }

    fn sql_alter_check(
        &self,
        table: &TableName,
        column: Option<&str>,
        drop_existing: bool,
        new_expr: Option<&str>,
    ) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let mut stmts = Vec::new();
                let constraint_name = check_constraint_name(table, column);

                if drop_existing {
                    stmts.push(match self.provider {
                        DatabaseProvider::Mysql => format!(
                            "ALTER TABLE {} DROP CHECK {}",
                            self.q_table(table),
                            self.q(&constraint_name),
                        ),
                        _ => format!(
                            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}",
                            self.q_table(table),
                            self.q(&constraint_name),
                        ),
                    });
                }

                if let Some(expr) = new_expr {
                    stmts.push(format!(
                        "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({})",
                        self.q_table(table),
                        self.q(&constraint_name),
                        expr,
                    ));
                }

                Ok(stmts)
            }
        }
    }

    fn sql_add_foreign_key(&self, fk: AddForeignKey<'_>) -> Result<Vec<String>> {
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

    fn sql_drop_foreign_key(
        &self,
        table: &TableName,
        constraint_name: &str,
    ) -> Result<Vec<String>> {
        match self.provider {
            DatabaseProvider::Sqlite => self.sqlite_rebuild(table),
            DatabaseProvider::Postgres => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}",
                self.q_table(table),
                self.q(constraint_name),
            )]),
            DatabaseProvider::Mysql => Ok(vec![format!(
                "ALTER TABLE {} DROP FOREIGN KEY {}",
                self.q_table(table),
                self.q(constraint_name),
            )]),
        }
    }

    fn sql_create_composite_type(&self, name: &str) -> Result<Vec<String>> {
        let ct = self
            .schema
            .composite_types
            .values()
            .find(|ct| ct.db_name == *name)
            .ok_or_else(|| {
                MigrationError::Other(format!(
                    "Composite type definition not found for '{}'",
                    name
                ))
            })?;
        Ok(vec![self.ddl.generate_composite_type(ct)?])
    }

    fn sql_drop_type(&self, name: &str) -> String {
        format!("DROP TYPE IF EXISTS {}", self.type_q(name))
    }

    fn sql_alter_composite_type(
        &self,
        name: &str,
        added_fields: &[(String, String)],
        dropped_fields: &[String],
        type_changed_fields: &[(String, String, String)],
    ) -> Vec<String> {
        let mut stmts: Vec<String> = Vec::new();
        for (field_name, sql_type) in added_fields {
            stmts.push(format!(
                "ALTER TYPE {} ADD ATTRIBUTE {} {}",
                self.type_q(name),
                self.q(field_name),
                sql_type,
            ));
        }
        for (field_name, _from, to) in type_changed_fields {
            stmts.push(format!(
                "ALTER TYPE {} ALTER ATTRIBUTE {} TYPE {} CASCADE",
                self.type_q(name),
                self.q(field_name),
                to,
            ));
        }
        for field_name in dropped_fields {
            stmts.push(format!(
                "ALTER TYPE {} DROP ATTRIBUTE {} CASCADE",
                self.type_q(name),
                self.q(field_name),
            ));
        }
        stmts
    }

    fn sql_create_enum(&self, name: &str, variants: &[String]) -> String {
        if let Some(def) = self
            .schema
            .enums
            .values()
            .find(|e| e.logical_name.eq_ignore_ascii_case(name))
        {
            return self.ddl.generate_enum_type(def);
        }

        let variants_sql = variants
            .iter()
            .map(|v| format!("'{}'", v))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "DO $$ BEGIN CREATE TYPE {} AS ENUM ({}); \
             EXCEPTION WHEN duplicate_object THEN NULL; END $$",
            self.type_q(name),
            variants_sql,
        )
    }

    /// Alter an enum type.
    ///
    /// Adding variants is a cheap `ADD VALUE`.  Removing them is not supported
    /// by PostgreSQL, so the type is renamed, recreated, every dependent column
    /// is cast across via `text`, and the old type is dropped.
    fn sql_alter_enum(
        &self,
        name: &str,
        added_variants: &[String],
        removed_variants: &[String],
    ) -> Result<Vec<String>> {
        if removed_variants.is_empty() {
            return Ok(added_variants
                .iter()
                .map(|v| {
                    format!(
                        "ALTER TYPE {} ADD VALUE IF NOT EXISTS '{}'",
                        self.type_q(name),
                        v
                    )
                })
                .collect());
        }

        let enum_def = self
            .schema
            .enums
            .values()
            .find(|e| e.logical_name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                MigrationError::Other(format!("Enum definition not found for '{}'", name))
            })?;

        let old_name = format!("{}_old", name);
        let variants_sql = enum_def
            .variants
            .iter()
            .map(|v| format!("'{}'", v))
            .collect::<Vec<_>>()
            .join(", ");

        let mut stmts = vec![
            format!(
                "ALTER TYPE {} RENAME TO {}",
                self.type_q(name),
                self.q(&old_name)
            ),
            format!(
                "CREATE TYPE {} AS ENUM ({})",
                self.type_q(name),
                variants_sql
            ),
        ];

        for (table_name, table) in &self.live.tables {
            for col in &table.columns {
                if col.col_type != *name {
                    continue;
                }
                stmts.extend(self.recast_enum_column(table_name, col, name, &old_name));
            }
        }

        stmts.push(format!("DROP TYPE {}", self.type_q(&old_name)));
        Ok(stmts)
    }

    /// Statements that move one column from the renamed old enum type onto the
    /// freshly created one, preserving its DEFAULT across the cast.
    fn recast_enum_column(
        &self,
        table_name: &TableName,
        col: &crate::live::LiveColumn,
        enum_name: &str,
        old_name: &str,
    ) -> Vec<String> {
        let mut stmts = Vec::new();

        if col.default_value.is_some() {
            stmts.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                self.q_table(table_name),
                self.q(&col.name),
            ));
        }

        stmts.push(format!(
            "ALTER TABLE {} ALTER COLUMN {} TYPE {} \
             USING {}::text::{}",
            self.q_table(table_name),
            self.q(&col.name),
            self.type_q(enum_name),
            self.q(&col.name),
            self.type_q(enum_name),
        ));

        if let Some(default) = &col.default_value {
            let new_default = if let Some(val) = default.strip_suffix(&format!("::{}", old_name)) {
                format!("{}::{}", val, enum_name)
            } else if let Some(val) = default.strip_suffix(&format!("::{}", self.type_q(old_name)))
            {
                format!("{}::{}", val, enum_name)
            } else {
                default.clone()
            };
            stmts.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                self.q_table(table_name),
                self.q(&col.name),
                new_default,
            ));
        }

        stmts
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

    fn quote_join(&self, columns: &[String]) -> String {
        columns
            .iter()
            .map(|c| self.q(c))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Quote an identifier for the target provider.
    fn q(&self, name: &str) -> String {
        self.provider.quote_identifier(name)
    }

    /// Quote a table in the statement's table position, qualifying it with its
    /// schema when it has one.
    fn q_table(&self, table: &TableName) -> String {
        self.strategy().quote_table(table)
    }

    fn materialize_provider_plan(
        &self,
        table: &TableName,
        plan: ProviderSqlPlan,
    ) -> Result<Vec<String>> {
        match plan {
            ProviderSqlPlan::Statements(stmts) => Ok(stmts),
            ProviderSqlPlan::RequiresTableRebuild => self.sqlite_rebuild(table),
        }
    }

    /// Quote several parts as one identifier, e.g. the `"<table>_pkey"` name
    /// PostgreSQL gives an implicit primary-key constraint.
    fn q_segments(&self, segments: &[&str]) -> String {
        let mut sql = String::new();
        nautilus_core::ident::push_quoted_ident_segments(
            &mut sql,
            segments,
            self.provider.identifier_quote(),
        );
        sql
    }

    /// Quote a PostgreSQL type identifier without folding its case.
    fn type_q(&self, name: &str) -> String {
        self.provider.quote_identifier(name)
    }

    /// Find a [`FieldIr`] by table DB-name and column DB-name.
    fn find_field(&self, table: &TableName, column: &str) -> Result<&FieldIr> {
        let model = self.find_model(table)?;
        model
            .fields
            .iter()
            .find(|f| f.db_name == column)
            .ok_or_else(|| MigrationError::Other(format!("Field not found: {}.{}", table, column)))
    }

    /// Find a [`ModelIr`] by table DB-name.
    fn find_model(&self, table: &TableName) -> Result<&ModelIr> {
        self.schema
            .models
            .values()
            .find(|m| crate::live::model_table(m) == *table)
            .ok_or_else(|| MigrationError::Other(format!("Model not found for table: {}", table)))
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

    /// Generate the 4-statement SQLite full-table rebuild for `table`.
    ///
    /// All four statements must be executed inside a single transaction.
    fn sqlite_rebuild(&self, table: &TableName) -> Result<Vec<String>> {
        let model = self.find_model(table)?;
        let live_table = self
            .live
            .tables
            .get(table)
            .ok_or_else(|| MigrationError::Other(format!("Live table not found: {}", table)))?;

        let tmp_name = format!("__tmp_{}", table.name);

        // Generate CREATE TABLE for the temp name by cloning the model with
        // a different db_name so the DDL generator quotes it correctly.
        let mut tmp_model = model.clone();
        tmp_model.db_name = tmp_name.clone();
        let create_tmp = self.ddl.generate_create_table(&tmp_model, self.schema)?;

        // SQLite refuses an INSERT that names a generated column, so the copy
        // lists only the columns the rebuilt table lets it write; the database
        // recomputes the rest.
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

/// Derive a deterministic index name from the table and column list.
fn index_name_auto(table: &TableName, columns: &[String]) -> String {
    format!("idx_{}_{}", table.name, columns.join("_"))
}

/// Derive a deterministic CHECK constraint name from table and optional column.
fn check_constraint_name(table: &TableName, column: Option<&str>) -> String {
    match column {
        Some(col) => format!("chk_{}_{}", table.name, col),
        None => format!("chk_{}", table.name),
    }
}
