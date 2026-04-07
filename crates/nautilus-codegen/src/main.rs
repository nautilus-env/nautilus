//! Nautilus Schema Codegen — standalone binary.
//! All logic lives in the library crate (`nautilus_codegen`).

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "nautilus-codegen")]
#[command(about = "Nautilus Schema Codegen Tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate code from a schema file
    Generate {
        #[arg(short, long, value_name = "FILE")]
        schema: Option<PathBuf>,
        /// Skip automatic package installation (Python only)
        #[arg(long)]
        no_install: bool,
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
        /// (Rust only) Also generate a Cargo.toml for the output crate.
        /// Default mode assumes integration into an existing Cargo workspace.
        #[arg(long)]
        standalone: bool,
    },
    /// Validate schema file without generating code
    Validate {
        #[arg(short, long, value_name = "FILE")]
        schema: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Generate {
            schema,
            no_install,
            verbose,
            standalone,
        } => {
            let path = nautilus_codegen::resolve_schema_path(schema)?;
            nautilus_codegen::generate_command(
                &path,
                nautilus_codegen::GenerateOptions {
                    install: !no_install,
                    verbose,
                    standalone,
                },
            )
        }
        Commands::Validate { schema } => {
            let path = nautilus_codegen::resolve_schema_path(schema)?;
            nautilus_codegen::validate_command(&path)
        }
    }
}
