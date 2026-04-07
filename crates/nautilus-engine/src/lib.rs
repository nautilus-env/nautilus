//! Nautilus Engine — library entry point.
//!
//! The binary (`nautilus-engine` / `nautilus engine serve`) is a thin shell over this crate.

#![forbid(unsafe_code)]

pub mod args;
pub mod conversion;
pub mod filter;
pub mod handlers;
pub mod state;
pub mod transport;

use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_schema::validate_schema_source;

pub use args::CliArgs;
pub use state::EngineState;

/// Run the engine with explicit parameters.
///
/// Parses the schema, connects to the database, optionally runs migrations,
/// then serves JSON-RPC requests on stdin/stdout until EOF.
pub async fn run_engine(
    schema_path: String,
    database_url: Option<String>,
    migrate: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let schema_source = std::fs::read_to_string(&schema_path)?;
    let schema_ir = validate_schema_source(&schema_source)?.ir;

    let resolved_url = database_url
        .or_else(|| {
            schema_ir
                .datasource
                .as_ref()
                .filter(|ds| !ds.url.is_empty())
                .map(|ds| ds.url.clone())
        })
        .ok_or("No database URL provided. Use --database-url, DATABASE_URL env var, or set 'url' in the schema datasource block.")?;

    let state = EngineState::new(schema_ir.clone(), resolved_url).await?;

    if migrate {
        eprintln!("[engine] Running schema migrations (--migrate)...");

        let datasource = schema_ir
            .datasource
            .as_ref()
            .ok_or("No datasource found in schema")?;

        let db_provider =
            DatabaseProvider::from_schema_provider(&datasource.provider).ok_or_else(|| {
                format!(
                    "Unsupported provider for migration: {}",
                    datasource.provider
                )
            })?;

        let generator = DdlGenerator::new(db_provider);
        let statements = generator.generate_create_tables(&schema_ir)?;
        state.execute_ddl_sql(statements).await?;

        eprintln!("[engine] Migrations applied successfully");
    }

    eprintln!("[engine] Engine initialized, entering request loop");

    transport::run_request_loop(state).await?;

    eprintln!("[engine] Shutting down gracefully");
    Ok(())
}

/// Convenience entry point for the standalone binary: parses argv then calls [`run_engine`].
pub async fn run_engine_from_cli() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse()?;
    run_engine(args.schema_path, args.database_url, args.migrate).await
}
