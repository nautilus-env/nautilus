//! Shared helpers for all `nautilus migrate` subcommands.

use anyhow::Context;
use nautilus_migrate::{DatabaseProvider, MigrationExecutor, MigrationFileStore};
use nautilus_schema::ir::SchemaIr;
use sqlx::AnyPool;
use std::path::PathBuf;

use crate::commands::db::connection::{
    detect_provider, load_dotenv_for_schema, obfuscate_url, parse_and_validate_schema,
    resolve_db_url, resolve_schema_path,
};

/// Everything a migrate subcommand typically needs.
pub struct MigrateContext {
    pub schema_ir: SchemaIr,
    pub database_url: String,
    pub provider: DatabaseProvider,
    pub executor: MigrationExecutor,
    pub store: MigrationFileStore,
}

impl MigrateContext {
    /// Build a [`MigrateContext`] from the raw CLI arguments shared by all
    /// migrate subcommands.
    pub async fn build(
        schema_arg: Option<String>,
        db_url_arg: Option<String>,
        migrations_dir_arg: Option<String>,
    ) -> anyhow::Result<Self> {
        let schema_path = resolve_schema_path(schema_arg)?;

        load_dotenv_for_schema(&schema_path);

        let schema_ir = parse_and_validate_schema(&schema_path)?;

        let database_url = resolve_db_url(db_url_arg, &schema_ir)?;
        let provider = detect_provider(&database_url)?;

        sqlx::any::install_default_drivers();
        let pool = AnyPool::connect(&database_url)
            .await
            .with_context(|| format!("Failed to connect to {}", obfuscate_url(&database_url)))?;

        let executor = MigrationExecutor::new(pool, provider);
        executor
            .init()
            .await
            .context("Failed to initialise migration tracking table")?;

        let migrations_dir = migrations_dir_arg.map(PathBuf::from).unwrap_or_else(|| {
            schema_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("migrations")
        });

        let store = MigrationFileStore::new(migrations_dir);

        Ok(MigrateContext {
            schema_ir,
            database_url,
            provider,
            executor,
            store,
        })
    }
}
