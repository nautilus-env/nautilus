//! Generating the Rust client.

use anyhow::Result;

use super::{GeneratedClient, GenerationContext};
use crate::composite_type_gen::generate_all_composite_types_with_registry;
use crate::enum_gen::generate_all_enums;
use crate::extension_types::generate_rust_extension_files;
use crate::generator::generate_all_models_with_registry;
use crate::install::Delivery;
use crate::writer;
use crate::InstallMode;

pub(super) fn generate(ctx: &GenerationContext<'_>) -> Result<GeneratedClient> {
    let models = generate_all_models_with_registry(ctx.ir, ctx.is_async, &ctx.registry)?;
    let enums_code = (!ctx.ir.enums.is_empty())
        .then(|| generate_all_enums(&ctx.ir.enums))
        .transpose()?;
    let composite_types_code = generate_all_composite_types_with_registry(ctx.ir, &ctx.registry)?;
    let extension_files = generate_rust_extension_files(&ctx.registry)?;

    // Rust integration always needs a persistent output path because
    // integrating adds a Cargo path-dependency pointing to the generated crate
    // on disk.
    let output_path = ctx
        .output_path
        .as_deref()
        .unwrap_or("./generated")
        .to_string();

    let package = writer::rust::package(
        &output_path,
        &models,
        enums_code,
        composite_types_code,
        &extension_files,
        ctx.source,
        ctx.options.standalone,
    )?;

    Ok(GeneratedClient {
        language: "Rust",
        package,
        delivery: Delivery::RustCrate {
            path: output_path,
            integrate: ctx.options.install != InstallMode::Never,
        },
        warnings: Vec::new(),
    })
}
