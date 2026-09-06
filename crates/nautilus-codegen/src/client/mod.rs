//! Generating a client: from the IR to the files of a package.
//!
//! Nothing here writes, installs or prints. A backend answers with the files it
//! produced, the [`Delivery`] they are headed for, and any warning the run has
//! to pass on, so a caller can generate a client and inspect it without a
//! directory to put it in.

mod java;
mod js;
mod python;
mod rust;

use anyhow::Result;
use std::path::Path;

use nautilus_schema::ir::SchemaIr;

use crate::extension_types::ExtensionRegistry;
use crate::install::Delivery;
use crate::package::GeneratedPackage;
use crate::GenerateOptions;

/// A generated client: its files, where they are headed, and what the run has
/// to tell the user.
pub(crate) struct GeneratedClient {
    /// The language named in the line that reports the finished client.
    pub(crate) language: &'static str,
    pub(crate) package: GeneratedPackage,
    pub(crate) delivery: Delivery,
    pub(crate) warnings: Vec<String>,
}

/// Everything the per-language generators need: the validated IR plus the
/// generator settings resolved once from it.
pub(crate) struct GenerationContext<'a> {
    pub(super) ir: &'a SchemaIr,
    pub(super) schema_path: &'a Path,
    pub(super) source: &'a str,
    pub(super) options: &'a GenerateOptions,
    pub(super) is_async: bool,
    pub(super) recursive_type_depth: usize,
    pub(super) output_path: Option<String>,
    pub(super) registry: ExtensionRegistry,
}

impl<'a> GenerationContext<'a> {
    pub(crate) fn new(
        ir: &'a SchemaIr,
        schema_path: &'a Path,
        source: &'a str,
        options: &'a GenerateOptions,
    ) -> Self {
        let generator = ir.generator.as_ref();
        Self {
            ir,
            schema_path,
            source,
            options,
            is_async: generator
                .map(|g| g.interface == nautilus_schema::ir::InterfaceKind::Async)
                .unwrap_or(false),
            recursive_type_depth: generator.map(|g| g.recursive_type_depth).unwrap_or(5),
            output_path: generator.and_then(|g| g.output.clone()),
            registry: ExtensionRegistry::from_schema(ir),
        }
    }

    /// Generate the client the schema's generator block asks for.
    pub(crate) fn generate(&self) -> Result<GeneratedClient> {
        match self.provider() {
            "nautilus-client-rs" => rust::generate(self),
            "nautilus-client-py" => python::generate(self),
            "nautilus-client-js" => js::generate(self),
            "nautilus-client-java" => java::generate(self),
            other => Err(anyhow::anyhow!(
                "Unsupported generator provider: '{}'. Supported: 'nautilus-client-rs', 'nautilus-client-py', 'nautilus-client-js', 'nautilus-client-java'",
                other
            )),
        }
    }

    fn provider(&self) -> &str {
        self.ir
            .generator
            .as_ref()
            .map(|g| g.provider.as_str())
            .unwrap_or("nautilus-client-rs")
    }

    /// Absolute schema path as embedded in generated clients: Windows UNC
    /// prefixes stripped and separators normalised so the literal is valid in
    /// every target language.
    pub(super) fn embedded_schema_path(&self) -> String {
        self.schema_path
            .canonicalize()
            .unwrap_or_else(|_| self.schema_path.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "/")
    }
}
