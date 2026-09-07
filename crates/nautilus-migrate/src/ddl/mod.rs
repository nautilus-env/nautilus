use crate::error::Result;
use crate::live::model_table;
use crate::provider::ProviderStrategy;
use nautilus_core::TableName;
use nautilus_schema::ir::{DefaultValue, FieldIr, ModelIr, ResolvedFieldType, SchemaIr};

mod columns;
mod defaults;
mod indexes;
mod reset;
mod tables;
mod types;
mod user_types;

pub use crate::provider::DatabaseProvider;

/// Every model Nautilus manages, i.e. everything except `@@ignore`d models and
/// `view` blocks.
///
/// An ignored model names a table that exists but that Nautilus neither creates
/// nor drops; a view names a relation the database owns outright. Neither may
/// appear in any generated DDL.
pub(crate) fn managed_models(schema: &SchemaIr) -> Vec<&ModelIr> {
    schema
        .models
        .values()
        .filter(|model| !model.is_ignored && !model.is_view)
        .collect()
}

/// Every field of `model` that maps to a column Nautilus manages.
///
/// Relation fields carry no column of their own, and `@ignore`d fields name a
/// column Nautilus neither creates nor alters.
pub(crate) fn managed_scalar_fields(model: &ModelIr) -> impl Iterator<Item = &FieldIr> {
    model.fields.iter().filter(|field| {
        !field.is_ignored && !matches!(field.field_type, ResolvedFieldType::Relation(_))
    })
}

/// Generates DDL (Data Definition Language) SQL from schema IR
pub struct DdlGenerator {
    provider: DatabaseProvider,
}

impl DdlGenerator {
    /// Create a new DDL generator
    pub fn new(provider: DatabaseProvider) -> Self {
        Self { provider }
    }

    /// Return the database provider this generator is configured for.
    pub fn provider(&self) -> DatabaseProvider {
        self.provider
    }

    /// Create managed tables in foreign-key dependency order, after their schemas,
    /// types and extensions; emit each table before its secondary indexes.
    pub fn generate_create_tables(&self, schema: &SchemaIr) -> Result<Vec<String>> {
        let mut statements = Vec::new();
        let strategy = ProviderStrategy::new(self.provider);

        if strategy.supports_user_defined_types() {
            if let Some(ds) = &schema.datasource {
                for declared in &ds.schemas {
                    statements.push(strategy.create_schema_sql(declared));
                }
                for ext in &ds.extensions {
                    statements.push(self.generate_create_extension(ext));
                }
            }
            for enum_def in schema.enums.values() {
                statements.push(self.generate_enum_type(enum_def));
            }
            for composite_type in schema.composite_types.values() {
                statements.push(self.generate_composite_type(composite_type)?);
            }
        }
        let all_models = managed_models(schema);
        for model in crate::diff::topo_sort_models(&all_models) {
            statements.push(self.generate_create_table(model, schema)?);
            statements.extend(self.generate_create_indexes_for_model(model));
        }

        Ok(statements)
    }

    /// Generate DROP TABLE statements for all models (in reverse dependency order)
    pub fn generate_drop_tables(&self, schema: &SchemaIr) -> Result<Vec<String>> {
        let mut statements = Vec::new();
        let strategy = ProviderStrategy::new(self.provider);
        let all_models = managed_models(schema);
        let sorted = crate::diff::topo_sort_models(&all_models);
        for model in sorted.into_iter().rev() {
            statements.push(strategy.drop_table_sql(&model_table(model), true));
        }

        if strategy.supports_user_defined_types() {
            for composite_type in schema.composite_types.values() {
                statements.push(strategy.drop_type_sql(&composite_type.db_name));
            }
            for enum_def in schema.enums.values() {
                statements.push(strategy.drop_type_sql(&enum_def.logical_name.to_lowercase()));
            }
        }

        Ok(statements)
    }

    /// Quote an identifier for the target database
    pub(crate) fn quote_identifier(&self, name: &str) -> String {
        self.provider.quote_identifier(name)
    }

    /// Quote a table in the statement's table position, qualifying it with its
    /// schema when it has one.
    pub(crate) fn quote_table(&self, table: &TableName) -> String {
        ProviderStrategy::new(self.provider).quote_table(table)
    }

    /// Quote a PostgreSQL type identifier.
    ///
    /// We quote all emitted type names so mixed-case live types like
    /// `"PostStatus"` are addressed exactly as stored and never folded to
    /// `poststatus` by the SQL parser.
    fn quote_type_identifier(&self, name: &str) -> String {
        self.quote_identifier(name)
    }
}

/// Whether a field carries the `autoincrement()` default.
pub(crate) fn field_is_autoincrement(field: &FieldIr) -> bool {
    matches!(
        &field.default_value,
        Some(DefaultValue::Function(function)) if function.name == "autoincrement"
    )
}
