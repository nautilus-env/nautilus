//! How a generated Rust client is laid out.

use anyhow::{Context, Result};
use heck::ToSnakeCase;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tera::Context as TeraContext;

use crate::generator::TEMPLATES;
use crate::package::GeneratedPackage;
use crate::GeneratedFile;

/// Lay out the generated Rust client.
///
/// Produces:
/// - `src/lib.rs`           — module declarations and re-exports
/// - `src/{model_snake}.rs` — model code for each model
/// - `src/enums.rs`         — all enum types (if any)
/// - `src/types.rs`         — composite types (if any)
/// - `src/runtime.rs`, `src/events.rs` — the runtime the models call into
/// - `Cargo.toml`           — **only** when `standalone == true`
///
/// When `standalone` is `false` (the default) the output is a plain directory
/// of `.rs` source files ready to be included in an existing Cargo workspace
/// without any generated `Cargo.toml`. `output_path` is where the package is
/// headed, which the standalone manifest needs to point back at the workspace.
pub(crate) fn package(
    output_path: &str,
    models: &HashMap<String, String>,
    enums_code: Option<String>,
    composite_types_code: Option<String>,
    extension_files: &[GeneratedFile],
    schema_source: &str,
    standalone: bool,
) -> Result<GeneratedPackage> {
    let mut package = GeneratedPackage::default();

    for (model_name, code) in models {
        package.add(format!("src/{}.rs", model_name.to_snake_case()), code);
    }

    let has_enums = enums_code.is_some();
    if let Some(enums_code) = enums_code {
        package.add("src/enums.rs", enums_code);
    }

    let has_composite_types = composite_types_code.is_some();
    if let Some(types_code) = composite_types_code {
        package.add("src/types.rs", types_code);
    }

    package.add_all("src", extension_files);

    package.add(
        "src/lib.rs",
        generate_lib_rs(
            models,
            has_enums,
            has_composite_types,
            !extension_files.is_empty(),
            schema_source,
        )?,
    );
    package.add(
        "src/runtime.rs",
        include_str!("../../templates/rust/runtime.rs.tpl"),
    );
    package.add(
        "src/events.rs",
        include_str!("../../templates/rust/events.rs.tpl"),
    );

    if standalone {
        package.add(
            "Cargo.toml",
            generate_rust_cargo_toml(&workspace_root_path(output_path)?),
        );
    }

    Ok(package.sorted())
}

/// How many `..` hops separate the output directory from the workspace root,
/// as the generated `Cargo.toml` has to spell them.
fn workspace_root_path(output_path: &str) -> Result<String> {
    let output = Path::new(output_path);
    let absolute = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Failed to get current directory")?
            .join(output)
    };

    let mut hops = PathBuf::new();
    let mut candidate = absolute.clone();

    if let Some(workspace_toml) = crate::find_workspace_cargo_toml(&absolute) {
        let workspace_dir = workspace_toml.parent().unwrap();
        while candidate != workspace_dir {
            hops.push("..");
            match candidate.parent() {
                Some(parent) => candidate = parent.to_path_buf(),
                None => break,
            }
        }
    } else {
        // Fallback: legacy upward walk (shouldn't normally be reached).
        loop {
            candidate = match candidate.parent() {
                Some(parent) => parent.to_path_buf(),
                None => break,
            };
            hops.push("..");
            let Ok(manifest) = std::fs::read_to_string(candidate.join("Cargo.toml")) else {
                continue;
            };
            if manifest.contains("[workspace]") {
                break;
            }
        }
    }

    Ok(hops.to_string_lossy().replace('\\', "/"))
}

/// Generate Cargo.toml for the generated Rust package.
///
/// `workspace_root_path` is the relative path from the output directory back
/// to the Cargo workspace root, e.g. `"../../../.."` when the output sits
/// four directory levels below the workspace root.
fn generate_rust_cargo_toml(workspace_root_path: &str) -> String {
    include_str!("../../templates/rust/Cargo.toml.tpl")
        .replace("{{ workspace_root_path }}", workspace_root_path)
        .replace("{{ rust_version }}", env!("CARGO_PKG_RUST_VERSION"))
}

/// Generate the lib.rs file content with module declarations and re-exports.
fn generate_lib_rs(
    models: &HashMap<String, String>,
    has_enums: bool,
    has_composite_types: bool,
    has_extensions: bool,
    schema_source: &str,
) -> Result<String> {
    let mut model_names: Vec<_> = models.keys().cloned().collect();
    model_names.sort();

    let model_modules: Vec<String> = model_names
        .iter()
        .map(|model_name| model_name.to_snake_case())
        .collect();

    let mut context = TeraContext::new();
    context.insert("has_enums", &has_enums);
    context.insert("has_composite_types", &has_composite_types);
    context.insert("has_extensions", &has_extensions);
    context.insert("model_modules", &model_modules);
    context.insert("schema_source_literal", &format!("{:?}", schema_source));

    TEMPLATES
        .render("lib_rs.tera", &context)
        .context("Failed to render lib.rs template")
}
