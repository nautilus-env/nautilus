use super::ProviderStrategy;
use nautilus_core::ident::quote_ident;

impl ProviderStrategy {
    pub(crate) fn create_schema_sql(&self, name: &str) -> String {
        format!(
            "CREATE SCHEMA IF NOT EXISTS {}",
            self.provider.quote_identifier(name)
        )
    }

    /// PostgreSQL has no `CREATE TYPE IF NOT EXISTS` syntax.
    pub(crate) fn create_enum_sql(&self, name: &str, variants: &[String]) -> String {
        let variants = variants
            .iter()
            .map(|v| format!("'{}'", v))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "DO $$ BEGIN CREATE TYPE {} AS ENUM ({}); \
             EXCEPTION WHEN duplicate_object THEN NULL; END $$",
            self.provider.quote_identifier(name),
            variants,
        )
    }

    pub(crate) fn drop_type_sql(&self, name: &str) -> String {
        format!(
            "DROP TYPE IF EXISTS {}",
            self.provider.quote_identifier(name)
        )
    }

    /// Omits CASCADE so dependent objects prevent the drop.
    pub(crate) fn drop_extension_sql(&self, name: &str) -> String {
        format!("DROP EXTENSION IF EXISTS {}", quote_ident(name, '"'))
    }
}
