//! Nautilus Codegen — library entry point.
//!
//! Exposes `generate_command`, `validate_command`, and helpers so they can be
//! called from `nautilus-cli` (the unified binary) as well as from the
//! standalone `nautilus-codegen` binary.
//!
//! A run is four steps, each of which is one module: `load` turns a schema
//! path into the IR, `client` turns the IR into the files of a package,
//! `install` puts those files where the schema asked, and `report` is the
//! only one that prints. Generating a client therefore neither writes nor
//! installs anything by itself.

#![forbid(unsafe_code)]

pub mod backend;
mod client;
pub mod composite_type_gen;
pub mod enum_gen;
pub mod extension_types;
pub mod generator;
mod install;
pub mod java;
pub mod js;
mod load;
pub(crate) mod model_view;
pub mod package;
pub(crate) mod publish;
pub mod python;
mod report;
pub(crate) mod schema_docs;
pub(crate) mod template;
pub mod type_helpers;
pub mod vector_meta;
pub mod writer;

/// A file a backend produced: its path relative to the output directory, and
/// its full contents.
///
/// This pair is the only thing the writers learn about generated output, which
/// is why the same shape carries Rust modules, Python packages, JavaScript
/// runtime files and Java sources alike.
pub type GeneratedFile = (String, String);

/// The two file lists a JavaScript backend produces: the `.js` sources and the
/// `.d.ts` declarations that describe them, each sorted by file name.
pub type GeneratedJsFiles = (Vec<GeneratedFile>, Vec<GeneratedFile>);

use anyhow::{Context, Result};
use std::path::Path;

use nautilus_schema::SchemaSet;

pub use load::{parse_schema, resolve_schema_path};

/// Options controlling code generation behaviour.
#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    /// Whether the generated package is also installed after generation.
    pub install: InstallMode,
    /// Print verbose progress and IR debug output.
    pub verbose: bool,
    /// (Rust only) Also emit a `Cargo.toml` for the generated crate.
    /// Default mode produces bare source files that integrate into an existing
    /// Cargo workspace. Pass `true` when you want a self-contained crate.
    pub standalone: bool,
}

/// Whether `generate` installs the client on top of writing it to `output`.
///
/// Installing copies the Python package into `site-packages/nautilus` and the
/// JavaScript one into `node_modules/nautilus` — machine-wide locations two
/// projects on one machine would overwrite for each other. Under [`Auto`] that
/// only happens when the generator block names no `output` to import from,
/// which is the only case where it is the sole way to reach the client.
///
/// The Rust client is unaffected by the distinction: "installing" it means
/// adding the generated crate to the nearest Cargo workspace, which is local.
///
/// [`Auto`]: InstallMode::Auto
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallMode {
    /// Install only when there is no `output` path to import from.
    #[default]
    Auto,
    /// Always install (`--install`).
    Always,
    /// Never install (`--no-install`).
    Never,
}

/// Parse, validate, and if successful generate client code for the given schema.
///
/// `options.standalone` (Rust provider only): also write a `Cargo.toml` for the output crate.
/// When `false` (default) the code is written without a Cargo.toml so it can be
/// included directly in an existing Cargo workspace.
pub fn generate_command(schema_path: &Path, options: GenerateOptions) -> Result<()> {
    let start = std::time::Instant::now();

    let schema = SchemaSet::load_path(schema_path)
        .with_context(|| format!("Failed to read schema: {}", schema_path.display()))?;

    let ir = load::generation_ir(&schema, options.verbose)?;
    report::loaded_schema(schema_path, &ir);

    let client =
        client::GenerationContext::new(&ir, schema_path, schema.source(), &options).generate()?;
    for warning in &client.warnings {
        report::warning(warning);
    }

    let delivered = install::deliver(&client, schema_path)?;
    for warning in &delivered.warnings {
        report::warning(warning);
    }

    // No output means the run had nothing to write and has said why.
    if let Some(output) = delivered.output {
        report::generated(client.language, &output, start.elapsed());
    }

    Ok(())
}

/// Parse and validate the schema, printing a summary. Does not generate code.
pub fn validate_command(schema_path: &Path) -> Result<()> {
    let schema = SchemaSet::load_path(schema_path)
        .with_context(|| format!("Failed to read schema: {}", schema_path.display()))?;

    let ir = schema
        .validate()
        .map(|validated| validated.ir)
        .map_err(|e| anyhow::anyhow!("Validation failed:\n{}", schema.format_error(&e)))?;

    println!("models: {}, enums: {}", ir.models.len(), ir.enums.len());
    for (name, model) in &ir.models {
        println!("  {} ({} fields)", name, model.fields.len());
    }

    Ok(())
}
