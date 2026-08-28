//! Nautilus Codegen — library entry point.
//!
//! Exposes `generate_command`, `validate_command`, and helpers so they can be
//! called from `nautilus-cli` (the unified binary) as well as from the
//! standalone `nautilus-codegen` binary.

#![forbid(unsafe_code)]

pub mod backend;
pub mod composite_type_gen;
pub mod enum_gen;
pub mod extension_types;
pub mod generator;
pub mod java;
pub mod js;
pub(crate) mod model_view;
pub mod python;
pub(crate) mod schema_docs;
pub(crate) mod template;
pub mod type_helpers;
pub mod vector_meta;
pub mod writer;

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::composite_type_gen::generate_all_composite_types_with_registry;
use crate::enum_gen::generate_all_enums;
use crate::extension_types::{
    generate_js_extension_files, generate_python_extension_files, generate_rust_extension_files,
    ExtensionRegistry,
};
use crate::generator::generate_all_models_with_registry;
use crate::java::build_java_bundle;
use crate::js::{
    generate_js_client, generate_js_composite_types, generate_js_enums, generate_js_models_index,
    js_runtime_files,
};
use crate::python::{generate_python_composite_types, generate_python_enums, python_runtime_files};
use crate::writer::{write_java_code, write_js_code, write_python_code, write_rust_code};
use nautilus_schema::ir::{JavaGenerationMode, ResolvedFieldType, SchemaIr};
use nautilus_schema::{parse_schema_source, validate_schema_source};

fn eprint_warning(message: &str) {
    eprintln!(
        "{} {}",
        console::style("warning:").yellow().bold(),
        console::style(message).yellow()
    );
}

/// Auto-detect the first `.nautilus` file in the current directory, or return
/// `schema` as-is if explicitly provided.
pub fn resolve_schema_path(schema: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = schema {
        return Ok(path);
    }

    let nautilus_files = nautilus_schema::discover_schema_paths_in_current_dir()
        .context("Failed to inspect current directory for .nautilus schema files")?;

    if nautilus_files.is_empty() {
        return Err(anyhow::anyhow!(
            "No .nautilus schema file found in current directory.\n\n\
            Hint: Create a schema file (e.g. 'schema.nautilus') or specify the path:\n\
            nautilus generate --schema path/to/schema.nautilus"
        ));
    }

    let schema_file = &nautilus_files[0];

    if nautilus_files.len() > 1 {
        eprint_warning(&format!(
            "multiple .nautilus files found, using: {}",
            schema_file.display()
        ));
    }

    Ok(schema_file.clone())
}

/// Options controlling code generation behaviour.
#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    /// Install the generated package after generation.
    /// Python: copy to site-packages. Rust: add to workspace `Cargo.toml`.
    pub install: bool,
    /// Print verbose progress and IR debug output.
    pub verbose: bool,
    /// (Rust only) Also emit a `Cargo.toml` for the generated crate.
    /// Default mode produces bare source files that integrate into an existing
    /// Cargo workspace. Pass `true` when you want a self-contained crate.
    pub standalone: bool,
}

/// Verify that all type references in the IR resolve to known definitions.
///
/// The schema validator already checks these, but this acts as a defense-in-depth
/// guard so codegen never silently produces broken output from a malformed IR.
fn validate_ir_references(ir: &SchemaIr) -> Result<()> {
    for (model_name, model) in &ir.models {
        for field in &model.fields {
            match &field.field_type {
                ResolvedFieldType::Enum { enum_name, .. } => {
                    if !ir.enums.contains_key(enum_name) {
                        return Err(anyhow::anyhow!(
                            "Model '{}' field '{}' references unknown enum '{}'",
                            model_name,
                            field.logical_name,
                            enum_name
                        ));
                    }
                }
                ResolvedFieldType::Relation(rel) => {
                    if !ir.models.contains_key(&rel.target_model) {
                        return Err(anyhow::anyhow!(
                            "Model '{}' field '{}' references unknown model '{}'",
                            model_name,
                            field.logical_name,
                            rel.target_model
                        ));
                    }
                }
                ResolvedFieldType::CompositeType { type_name, .. } => {
                    if !ir.composite_types.contains_key(type_name) {
                        return Err(anyhow::anyhow!(
                            "Model '{}' field '{}' references unknown composite type '{}'",
                            model_name,
                            field.logical_name,
                            type_name
                        ));
                    }
                }
                ResolvedFieldType::Scalar(_) => {}
            }
        }
    }
    Ok(())
}

/// Parse, validate, and if successful generate client code for the given schema.
///
/// `options.standalone` (Rust provider only): also write a `Cargo.toml` for the output crate.
/// When `false` (default) the code is written without a Cargo.toml so it can be
/// included directly in an existing Cargo workspace.
pub fn generate_command(schema_path: &PathBuf, options: GenerateOptions) -> Result<()> {
    let start = std::time::Instant::now();

    let source = fs::read_to_string(schema_path)
        .with_context(|| format!("Failed to read schema file: {}", schema_path.display()))?;

    let ir = load_generation_ir(schema_path, &source, options.verbose)?;
    report_loaded_schema(schema_path, &ir);

    let ctx = GenerationContext::new(&ir, schema_path, &source, &options);
    let generated = match ctx.provider() {
        "nautilus-client-rs" => ctx.generate_rust()?,
        "nautilus-client-py" => ctx.generate_python()?,
        "nautilus-client-js" => ctx.generate_js()?,
        "nautilus-client-java" => ctx.generate_java()?,
        other => {
            return Err(anyhow::anyhow!(
                "Unsupported generator provider: '{}'. Supported: 'nautilus-client-rs', 'nautilus-client-py', 'nautilus-client-js', 'nautilus-client-java'",
                other
            ));
        }
    };

    // `None` means the run had nothing to write and already told the user why.
    let Some(generated) = generated else {
        return Ok(());
    };

    println!(
        "\nGenerated {} {} {} {}\n",
        console::style(format!(
            "Nautilus Client for {} (v{})",
            generated.client_name,
            env!("CARGO_PKG_VERSION")
        ))
        .bold(),
        console::style("to").dim(),
        console::style(generated.output).italic().dim(),
        console::style(format!("({}ms)", start.elapsed().as_millis())).italic()
    );

    Ok(())
}

fn load_generation_ir(schema_path: &Path, source: &str, verbose: bool) -> Result<SchemaIr> {
    let validated = validate_schema_source(source).map_err(|e| {
        anyhow::anyhow!(
            "Validation failed:\n{}",
            e.format_with_file(&schema_path.display().to_string(), source)
        )
    })?;
    let nautilus_schema::ValidatedSchema { ast, ir } = validated;

    if verbose {
        println!("parsed {} declarations", ast.declarations.len());
    }

    validate_ir_references(&ir)?;

    if verbose {
        println!("{:#?}", ir);
    }

    Ok(ir)
}

fn report_loaded_schema(schema_path: &Path, ir: &SchemaIr) {
    if let Some(ds) = &ir.datasource {
        if let Some(var_name) = ds
            .url
            .strip_prefix("env(")
            .and_then(|s| s.strip_suffix(')'))
        {
            println!(
                "{} {} {}",
                console::style("Loaded").dim(),
                console::style(var_name).bold(),
                console::style("from .env").dim()
            );
        }
    }

    println!(
        "{} {}",
        console::style("Nautilus schema loaded from").dim(),
        console::style(schema_path.display()).italic().dim()
    );
}

/// A finished generation run: which client was produced and where it landed.
struct GeneratedClient {
    client_name: &'static str,
    output: String,
}

/// Everything the per-language generators need: the validated IR plus the
/// generator settings resolved once from it.
struct GenerationContext<'a> {
    ir: &'a SchemaIr,
    schema_path: &'a Path,
    source: &'a str,
    options: &'a GenerateOptions,
    is_async: bool,
    recursive_type_depth: usize,
    output_path: Option<String>,
    registry: ExtensionRegistry,
}

impl<'a> GenerationContext<'a> {
    fn new(
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
    fn embedded_schema_path(&self) -> String {
        self.schema_path
            .canonicalize()
            .unwrap_or_else(|_| self.schema_path.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "/")
    }

    fn generate_rust(&self) -> Result<Option<GeneratedClient>> {
        let models = generate_all_models_with_registry(self.ir, self.is_async, &self.registry)?;
        let enums_code = (!self.ir.enums.is_empty())
            .then(|| generate_all_enums(&self.ir.enums))
            .transpose()?;
        let composite_types_code =
            generate_all_composite_types_with_registry(self.ir, &self.registry)?;
        let extension_files = generate_rust_extension_files(&self.registry)?;

        // Rust integration always needs a persistent output path because
        // `integrate_rust_package` adds a Cargo path-dependency pointing to
        // the generated crate on disk.
        let output_path = self
            .output_path
            .as_deref()
            .unwrap_or("./generated")
            .to_string();

        write_rust_code(
            &output_path,
            &models,
            enums_code,
            composite_types_code,
            &extension_files,
            self.source,
            self.options.standalone,
        )?;

        if self.options.install {
            integrate_rust_package(&output_path, self.schema_path)?;
        }

        Ok(Some(GeneratedClient {
            client_name: "Rust",
            output: output_path,
        }))
    }

    fn generate_python(&self) -> Result<Option<GeneratedClient>> {
        let models = crate::python::generator::generate_all_python_models_with_registry(
            self.ir,
            self.is_async,
            self.recursive_type_depth,
            &self.registry,
        )?;
        let enums_code = (!self.ir.enums.is_empty())
            .then(|| generate_python_enums(&self.ir.enums))
            .transpose()?;
        let composite_types_code = generate_python_composite_types(&self.ir.composite_types)?;
        let extension_files = generate_python_extension_files(&self.registry)?;
        let client_code = python::generate_python_client(
            &self.ir.models,
            &self.embedded_schema_path(),
            self.is_async,
        )?;
        let runtime = python_runtime_files();

        let output = self.emit_package(
            "nautilus_codegen_tmp",
            |path| {
                write_python_code(
                    path,
                    &models,
                    enums_code.clone(),
                    composite_types_code.clone(),
                    &extension_files,
                    Some(client_code.clone()),
                    &runtime,
                )
            },
            install_python_package,
        )?;

        Ok(output.map(|output| GeneratedClient {
            client_name: "Python",
            output,
        }))
    }

    fn generate_js(&self) -> Result<Option<GeneratedClient>> {
        let (js_models, dts_models) =
            crate::js::generator::generate_all_js_models_with_registry(self.ir, &self.registry)?;
        let (js_enums, dts_enums) = if !self.ir.enums.is_empty() {
            let (js, dts) = generate_js_enums(&self.ir.enums)?;
            (Some(js), Some(dts))
        } else {
            (None, None)
        };
        let dts_composite_types = generate_js_composite_types(&self.ir.composite_types)?;
        let (js_extension_files, dts_extension_files) =
            generate_js_extension_files(&self.registry)?;
        let (js_client, dts_client) =
            generate_js_client(&self.ir.models, &self.embedded_schema_path())?;
        let (js_models_index, dts_models_index) = generate_js_models_index(&js_models)?;
        let runtime = js_runtime_files();

        let output = self.emit_package(
            "nautilus_codegen_js_tmp",
            |path| {
                write_js_code(
                    path,
                    &js_models,
                    &dts_models,
                    js_enums.clone(),
                    dts_enums.clone(),
                    dts_composite_types.clone(),
                    &js_extension_files,
                    &dts_extension_files,
                    Some(js_client.clone()),
                    Some(dts_client.clone()),
                    Some(js_models_index.clone()),
                    Some(dts_models_index.clone()),
                    &runtime,
                )
            },
            |path| install_js_package(path, self.schema_path),
        )?;

        Ok(output.map(|output| GeneratedClient {
            client_name: "JavaScript",
            output,
        }))
    }

    fn generate_java(&self) -> Result<Option<GeneratedClient>> {
        let java_mode = self
            .ir
            .generator
            .as_ref()
            .and_then(|g| g.java_mode)
            .unwrap_or(JavaGenerationMode::Maven);

        if self.options.install {
            match java_mode {
                JavaGenerationMode::Maven => eprint_warning(
                    "install = true is currently ignored for 'nautilus-client-java'; the generated Maven module is written only to the configured output path",
                ),
                JavaGenerationMode::Jar => eprint_warning(
                    "install = true is currently ignored for 'nautilus-client-java'; generation writes the Maven module to the configured output path and the plain Java bundle to output/dist",
                ),
            }
        }

        let output_path = self
            .output_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Java generation requires generator.output"))?;
        let files = crate::java::generator::generate_java_client_with_registry(
            self.ir,
            &self.embedded_schema_path(),
            self.is_async,
            &self.registry,
        )?;
        write_java_code(output_path, &files)?;

        let output = if java_mode == JavaGenerationMode::Jar {
            let artifact_id = self
                .ir
                .generator
                .as_ref()
                .and_then(|g| g.java_artifact_id.as_deref())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Java bundle mode requires generator.artifact_id to build the jar name"
                    )
                })?;
            build_java_bundle(output_path, artifact_id)?
                .display()
                .to_string()
        } else {
            output_path.to_string()
        };

        Ok(Some(GeneratedClient {
            client_name: "Java",
            output,
        }))
    }

    /// Write a generated package and optionally install it, returning the path
    /// reported to the user.
    ///
    /// Without a configured output path the package is only materialised in a
    /// temporary directory long enough to install it; `Ok(None)` means there
    /// was nowhere to write it and the user has been warned.
    fn emit_package(
        &self,
        tmp_dir_name: &str,
        write: impl Fn(&str) -> Result<()>,
        install: impl Fn(&str) -> Result<PathBuf>,
    ) -> Result<Option<String>> {
        let Some(output_path) = self.output_path.as_deref() else {
            if !self.options.install {
                eprint_warning("no output path specified and --no-install given; nothing written");
                return Ok(None);
            }

            let tmp_dir = std::env::temp_dir().join(tmp_dir_name);
            let tmp_path = tmp_dir.to_string_lossy().to_string();
            write(&tmp_path)?;
            let installed = install(&tmp_path)?;
            let _ = fs::remove_dir_all(&tmp_dir);
            return Ok(Some(installed.display().to_string()));
        };

        write(output_path)?;
        if self.options.install {
            Ok(Some(install(output_path)?.display().to_string()))
        } else {
            Ok(Some(output_path.to_string()))
        }
    }
}

/// Parse and validate the schema, printing a summary. Does not generate code.
pub fn validate_command(schema_path: &PathBuf) -> Result<()> {
    let source = fs::read_to_string(schema_path)
        .with_context(|| format!("Failed to read schema file: {}", schema_path.display()))?;

    let ir = validate_schema_source(&source)
        .map(|validated| validated.ir)
        .map_err(|e| {
            anyhow::anyhow!(
                "Validation failed:\n{}",
                e.format_with_file(&schema_path.display().to_string(), &source)
            )
        })?;

    println!("models: {}, enums: {}", ir.models.len(), ir.enums.len());
    for (name, model) in &ir.models {
        println!("  {} ({} fields)", name, model.fields.len());
    }

    Ok(())
}

pub fn parse_schema(source: &str) -> Result<nautilus_schema::ast::Schema> {
    parse_schema_source(source).map_err(|e| anyhow::anyhow!("{}", e))
}

/// Add the generated crate to the workspace `Cargo.toml` `[members]` array
/// (analogous to `install_python_package` for the Python provider).
///
/// Walks up from `schema_path` until it finds a `Cargo.toml` that contains
/// `[workspace]`. The member entry is expressed as a path relative to that
/// workspace root so the result stays portable.
fn integrate_rust_package(output_path: &str, schema_path: &Path) -> Result<()> {
    use std::io::Write;

    let workspace_toml_path = find_workspace_cargo_toml(schema_path).ok_or_else(|| {
        anyhow::anyhow!(
            "No workspace Cargo.toml found in '{}' or any parent directory.\n\
            Make sure you run 'nautilus generate' from within a Cargo workspace.",
            schema_path.display()
        )
    })?;

    let mut content =
        fs::read_to_string(&workspace_toml_path).context("Failed to read workspace Cargo.toml")?;

    let workspace_dir = workspace_toml_path.parent().unwrap();

    // Resolve the output path to an absolute path (it may be relative to cwd).
    let output_absolute = if Path::new(output_path).is_absolute() {
        PathBuf::from(output_path)
    } else {
        std::env::current_dir()
            .context("Failed to get current directory")?
            .join(output_path)
    };
    // Strip the Windows \\?\ UNC prefix when present.
    let cleaned_output = {
        let s = output_absolute.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            output_absolute.clone()
        }
    };

    let member_path: String = if let Ok(rel) = cleaned_output.strip_prefix(workspace_dir) {
        rel.to_string_lossy().replace('\\', "/")
    } else {
        // Fall back to the absolute path (unusual, but don't panic).
        cleaned_output.to_string_lossy().replace('\\', "/")
    };

    if content.contains(&member_path) {
    } else {
        // Find the closing bracket of the `members = [...]` array and insert
        // our entry before it. We handle both single-line and multi-line forms.
        //
        // Strategy: find "members" key, then find the matching `]` and inject.
        if let Some(members_pos) = content.find("members") {
            // Find the `[` that opens the array.
            if let Some(bracket_open) = content[members_pos..].find('[') {
                let open_abs = members_pos + bracket_open;
                // Find the matching `]`.
                if let Some(bracket_close) = content[open_abs..].find(']') {
                    let close_abs = open_abs + bracket_close;
                    // Insert before the closing bracket, with a trailing comma.
                    let insert = format!(",\n    \"{}\"", member_path);
                    // If the array is empty we don't want a leading comma.
                    let inner = content[open_abs + 1..close_abs].trim();
                    let insert = if inner.is_empty() {
                        format!("\n    \"{}\"", member_path)
                    } else {
                        insert
                    };
                    content.insert_str(close_abs, &insert);
                }
            }
        } else {
            // No `members` key at all — append a new one.
            content.push_str(&format!("\nmembers = [\n    \"{}\"]\n", member_path));
        }

        let mut file = fs::File::create(&workspace_toml_path)
            .context("Failed to open workspace Cargo.toml for writing")?;
        file.write_all(content.as_bytes())
            .context("Failed to write workspace Cargo.toml")?;
    }

    Ok(())
}

/// Walk up from `start` until we find a `Cargo.toml` that contains `[workspace]`.
pub(crate) fn find_workspace_cargo_toml(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_file() {
        start.parent()?
    } else {
        start
    };
    loop {
        let candidate = current.join("Cargo.toml");
        if candidate.exists() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                if content.contains("[workspace]") {
                    return Some(candidate);
                }
            }
        }
        current = current.parent()?;
    }
}

fn detect_site_packages() -> Result<PathBuf> {
    use std::process::Command;

    let script = "import sysconfig; print(sysconfig.get_path('purelib'))";
    for exe in &["python", "python3"] {
        if let Ok(out) = Command::new(exe).arg("-c").arg(script).output() {
            if out.status.success() {
                let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Ok(PathBuf::from(path_str));
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "Could not detect Python site-packages directory.\n\
        Make sure Python is installed and available as 'python' or 'python3'."
    ))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("Failed to create directory: {}", dst.display()))?;

    for entry in
        fs::read_dir(src).with_context(|| format!("Failed to read directory: {}", src.display()))?
    {
        let entry = entry.with_context(|| "Failed to read directory entry")?;
        let file_type = entry
            .file_type()
            .with_context(|| "Failed to get file type")?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} -> {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

const PYTHON_GENERATED_PACKAGE_ENTRIES: &[&str] = &[
    "__init__.py",
    "client.py",
    "transaction.py",
    "py.typed",
    "models",
    "enums",
    "errors",
    "_internal",
    "types",
    "extensions",
];

fn clear_generated_python_package(dst: &Path) -> Result<()> {
    for entry in PYTHON_GENERATED_PACKAGE_ENTRIES {
        let path = dst.join(entry);
        if path.is_dir() {
            fs::remove_dir_all(&path).with_context(|| {
                format!(
                    "Failed to remove generated directory from Python install: {}",
                    path.display()
                )
            })?;
        } else if path.exists() {
            fs::remove_file(&path).with_context(|| {
                format!(
                    "Failed to remove generated file from Python install: {}",
                    path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn install_python_package_into(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        if !dst.is_dir() {
            return Err(anyhow::anyhow!(
                "Python install target exists but is not a directory: {}",
                dst.display()
            ));
        }

        // Keep the CLI wrapper files that pip installs (`__main__.py`,
        // `nautilus`, `nautilus.exe`) and refresh only the generated client tree.
        clear_generated_python_package(dst)?;
    }

    copy_dir_recursive(src, dst)
}

fn install_python_package(output_path: &str) -> Result<std::path::PathBuf> {
    let site_packages = detect_site_packages()?;
    let src = Path::new(output_path);
    let dst = site_packages.join("nautilus");

    install_python_package_into(src, &dst)?;
    Ok(dst)
}

/// Walk up from `schema_path` until we find a `node_modules` directory.
fn detect_node_modules(schema_path: &Path) -> Result<PathBuf> {
    let mut current = if schema_path.is_file() {
        schema_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Schema path has no parent directory"))?
    } else {
        schema_path
    };

    loop {
        let candidate = current.join("node_modules");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        current = current.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "No node_modules directory found in '{}' or any parent directory.\n\
                Make sure you run 'nautilus generate' from within a Node.js project \
                (i.e. a directory with node_modules).",
                schema_path.display()
            )
        })?;
    }
}

fn install_js_package(output_path: &str, schema_path: &Path) -> Result<std::path::PathBuf> {
    let node_modules = detect_node_modules(schema_path)?;
    let src = Path::new(output_path);
    let dst = node_modules.join("nautilus");

    if dst.exists() {
        fs::remove_dir_all(&dst).with_context(|| {
            format!(
                "Failed to remove existing installation at: {}",
                dst.display()
            )
        })?;
    }

    copy_dir_recursive(src, &dst)?;

    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::install_python_package_into;

    #[test]
    fn python_install_preserves_cli_wrapper_files() {
        let src_root = tempfile::TempDir::new().expect("temp src dir");
        let dst_root = tempfile::TempDir::new().expect("temp dst dir");
        let src = src_root.path().join("generated");
        let dst = dst_root.path().join("nautilus");

        std::fs::create_dir_all(src.join("models")).expect("create generated models dir");
        std::fs::write(src.join("__init__.py"), "from .client import Nautilus\n")
            .expect("write generated __init__.py");
        std::fs::write(src.join("client.py"), "class Nautilus: ...\n")
            .expect("write generated client.py");
        std::fs::write(src.join("py.typed"), "").expect("write generated py.typed");
        std::fs::write(src.join("models").join("user.py"), "class User: ...\n")
            .expect("write generated model");

        std::fs::create_dir_all(dst.join("models")).expect("create installed models dir");
        std::fs::write(dst.join("__main__.py"), "def main(): ...\n")
            .expect("write cli __main__.py");
        std::fs::write(dst.join("nautilus"), "binary").expect("write cli binary");
        std::fs::write(dst.join("nautilus.exe"), "binary").expect("write cli windows binary");
        std::fs::write(dst.join("__init__.py"), "old generated package\n")
            .expect("write stale generated __init__.py");
        std::fs::write(dst.join("client.py"), "old client\n").expect("write stale client.py");
        std::fs::write(dst.join("models").join("legacy.py"), "old model\n")
            .expect("write stale model");

        install_python_package_into(&src, &dst).expect("overlay install should succeed");

        assert_eq!(
            std::fs::read_to_string(dst.join("__main__.py")).expect("read cli __main__.py"),
            "def main(): ...\n"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nautilus")).expect("read cli binary"),
            "binary"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nautilus.exe")).expect("read cli windows binary"),
            "binary"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("__init__.py")).expect("read generated __init__.py"),
            "from .client import Nautilus\n"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("client.py")).expect("read generated client.py"),
            "class Nautilus: ...\n"
        );
        assert!(
            !dst.join("models").join("legacy.py").exists(),
            "stale generated model should be removed"
        );
        assert!(
            dst.join("models").join("user.py").exists(),
            "new generated model should be installed"
        );
    }

    #[test]
    fn python_install_removes_generated_entries_absent_from_new_output() {
        let src_root = tempfile::TempDir::new().expect("temp src dir");
        let dst_root = tempfile::TempDir::new().expect("temp dst dir");
        let src = src_root.path().join("generated");
        let dst = dst_root.path().join("nautilus");

        std::fs::create_dir_all(src.join("_internal")).expect("create generated runtime dir");
        std::fs::write(src.join("__init__.py"), "fresh init\n").expect("write generated init");
        std::fs::write(src.join("_internal").join("__init__.py"), "").expect("write runtime init");

        std::fs::create_dir_all(dst.join("types")).expect("create stale types dir");
        std::fs::write(dst.join("types").join("__init__.py"), "stale types\n")
            .expect("write stale types init");

        install_python_package_into(&src, &dst).expect("overlay install should succeed");

        assert!(
            !dst.join("types").exists(),
            "stale generated types dir should be removed when no longer generated"
        );
        assert!(
            dst.join("_internal").join("__init__.py").exists(),
            "fresh generated runtime files should be installed"
        );
    }
}
