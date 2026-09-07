use super::DdlGenerator;
use crate::error::Result;
use crate::provider::ProviderStrategy;
use nautilus_core::ident::quote_ident;
use nautilus_schema::ir::{CompositeTypeIr, EnumIr, PostgresExtensionIr};

impl DdlGenerator {
    /// Create the enum under its lowercase physical name, matching introspection.
    pub(crate) fn generate_enum_type(&self, enum_def: &EnumIr) -> String {
        ProviderStrategy::new(self.provider)
            .create_enum_sql(&enum_def.logical_name.to_lowercase(), &enum_def.variants)
    }

    /// Generate CREATE EXTENSION IF NOT EXISTS for a PostgreSQL extension.
    ///
    /// The extension name is quoted with double quotes because some common
    /// names contain hyphens (e.g. `uuid-ossp`) which cannot appear in an
    /// unquoted identifier. When a target schema is provided, emit
    /// `WITH SCHEMA "<schema>"` so the extension is installed in that namespace
    /// rather than the default (`public`).
    pub(crate) fn generate_create_extension(&self, ext: &PostgresExtensionIr) -> String {
        let name = quote_ident(&ext.name, '"');
        match ext.schema.as_deref() {
            Some(schema) => format!(
                "CREATE EXTENSION IF NOT EXISTS {} WITH SCHEMA {}",
                name,
                quote_ident(schema, '"')
            ),
            None => format!("CREATE EXTENSION IF NOT EXISTS {}", name),
        }
    }

    /// Drop the extension without CASCADE, preserving dependent objects.
    pub(crate) fn generate_drop_extension(&self, name: &str) -> String {
        ProviderStrategy::new(self.provider).drop_extension_sql(name)
    }

    /// Generate CREATE TYPE ... AS (...) statement for a composite type (Postgres only).
    ///
    /// Uses a `DO` block so re-running against an existing DB is idempotent.
    pub(crate) fn generate_composite_type(&self, ct: &CompositeTypeIr) -> Result<String> {
        let columns: Vec<String> = ct
            .fields
            .iter()
            .map(|f| {
                let col_type = self.generate_column_type(
                    &f.field_type,
                    !f.is_required,
                    f.is_array,
                    f.storage_strategy,
                )?;
                Ok(format!(
                    "{} {}",
                    self.quote_identifier(&f.db_name),
                    col_type
                ))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(format!(
            "DO $$ BEGIN CREATE TYPE {} AS ({}); \
             EXCEPTION WHEN duplicate_object THEN NULL; END $$",
            self.quote_type_identifier(&ct.db_name),
            columns.join(", ")
        ))
    }
}
