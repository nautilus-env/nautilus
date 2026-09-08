//! The application context a command runs in.
//!
//! `db` and `migrate` both start from the same three services — find and read
//! the schema, resolve the database URL and connect, then report what an apply
//! did — so those live here rather than inside either command. What differs is
//! what each family of commands then holds: a live [`Connection`] for `db`,
//! a migration executor and file store for `migrate`.

pub mod database;
pub mod report;
pub mod schema;

use anyhow::Context;
use nautilus_migrate::{DatabaseProvider, MigrationExecutor, MigrationFileStore, SchemaInspector};
use nautilus_schema::ir::SchemaIr;
use sqlx::AnyPool;
use std::path::PathBuf;

use crate::tui;
use database::{detect_provider, inspector_for, obfuscate_url, resolve_db_url, Connection};
use schema::{load_dotenv_for_schema, parse_and_validate_schema, resolve_schema_path};

/// Everything a `nautilus db` subcommand typically needs after loading the
/// schema, resolving the URL, and connecting to the database.
pub struct DbContext {
    pub schema_ir: SchemaIr,
    pub database_url: String,
    pub provider: DatabaseProvider,
    pub conn: Connection,
}

impl DbContext {
    /// An inspector scoped to the schemas this datasource declares.
    pub fn inspector(&self) -> SchemaInspector {
        inspector_for(self.provider, &self.database_url, &self.schema_ir)
    }

    /// Parse a schema file, resolve the database URL, connect, and inspect the
    /// provider — the shared preamble of `push`, `status`, and `reset`.
    pub async fn build(
        schema_arg: Option<String>,
        db_url_arg: Option<String>,
    ) -> anyhow::Result<Self> {
        let schema_path = resolve_schema_path(schema_arg)?;

        load_dotenv_for_schema(&schema_path);

        let sp = tui::spinner("Parsing schema…");
        let schema_ir = parse_and_validate_schema(&schema_path)?;

        let model_count = schema_ir.models.len();
        let provider_name = schema_ir
            .datasource
            .as_ref()
            .map(|ds| ds.provider.clone())
            .unwrap_or_else(|| "unknown".to_string());

        tui::spinner_ok(
            sp,
            &format!(
                "Schema parsed  ({} model{}, {})",
                model_count,
                if model_count == 1 { "" } else { "s" },
                provider_name,
            ),
        );

        let database_url = resolve_db_url(db_url_arg, &schema_ir)?;

        let sp = tui::spinner("Connecting to database…");
        let provider = detect_provider(&database_url)?;
        let conn = Connection::connect(&database_url, provider)
            .await
            .with_context(|| format!("Failed to connect to {}", database_url))?;
        tui::spinner_ok(sp, &format!("Connected  {}", obfuscate_url(&database_url)));

        Ok(DbContext {
            schema_ir,
            database_url,
            provider,
            conn,
        })
    }
}

/// Everything a migrate subcommand typically needs.
pub struct MigrateContext {
    pub schema_ir: SchemaIr,
    pub database_url: String,
    pub provider: DatabaseProvider,
    pub executor: MigrationExecutor,
    pub store: MigrationFileStore,
}

impl MigrateContext {
    /// An inspector scoped to the schemas this datasource declares.
    pub fn inspector(&self) -> SchemaInspector {
        inspector_for(self.provider, &self.database_url, &self.schema_ir)
    }

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
