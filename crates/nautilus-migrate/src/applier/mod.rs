//! Diff applier — translates each [`Change`] into one or more SQL statements
//! that, when executed, bring the live database in sync with the target schema.
//!
//! Statements retain their dependency order. Transaction boundaries belong to
//! the executor: PostgreSQL enum additions may require a separate phase, while
//! MySQL DDL may commit implicitly. SQLite rebuilds need one transaction.

use crate::ddl::{DatabaseProvider, DdlGenerator};
use crate::diff::Change;
use crate::error::{MigrationError, Result};
use crate::live::LiveSchema;
use crate::provider::{ProviderSqlPlan, ProviderStrategy};
use nautilus_core::TableName;
use nautilus_schema::ir::{FieldIr, ModelIr, PostgresExtensionIr, SchemaIr};

mod columns;
mod constraints;
mod indexes;
mod sqlite;
mod tables;
mod user_types;

use constraints::AddForeignKey;

/// Translates schema [`Change`]s into executable SQL statements.
///
/// ```ignore
/// let applier = DiffApplier::new(provider, &ddl, &schema_ir, &live);
/// let statements: Vec<String> = changes
///     .iter()
///     .flat_map(|c| applier.sql_for(c).unwrap())
///     .collect();
/// let phases = nautilus_migrate::plan_apply_phases(&statements);
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
    /// Preserve statement order and use [`crate::plan_apply_phases`] to identify
    /// transaction boundaries. A multi-statement change is not always atomic.
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

            Change::CreateSchema { name } => {
                self.sql_for_user_type(|this| Ok(vec![this.strategy().create_schema_sql(name)]))
            }

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
}
