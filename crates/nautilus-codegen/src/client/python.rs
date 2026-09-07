//! Generating the Python client.

use anyhow::Result;

use super::{GeneratedClient, GenerationContext};
use crate::extension_types::generate_python_extension_files;
use crate::install::{Delivery, InstallTarget};
use crate::python::{
    generate_python_client, generate_python_composite_types, generate_python_enums,
    python_runtime_files,
};
use crate::writer;

pub(super) fn generate(ctx: &GenerationContext<'_>) -> Result<GeneratedClient> {
    let models = crate::python::generator::generate_python_model_files(
        ctx.ir,
        ctx.is_async,
        ctx.recursive_type_depth,
        &ctx.registry,
    )?;
    let enums_code = (!ctx.ir.enums.is_empty())
        .then(|| generate_python_enums(&ctx.ir.enums))
        .transpose()?;
    let composite_types_code = generate_python_composite_types(&ctx.ir.composite_types)?;
    let extension_files = generate_python_extension_files(&ctx.registry)?;
    let client_code =
        generate_python_client(&ctx.ir.models, &ctx.embedded_schema_path(), ctx.is_async)?;

    let mut package = writer::python::package(
        &models.facades,
        enums_code,
        composite_types_code,
        &extension_files,
        Some(client_code),
        &python_runtime_files(),
    )?;
    package.add_all("models", &models.parts);

    Ok(GeneratedClient {
        language: "Python",
        package: package.sorted(),
        delivery: Delivery::Installable {
            target: InstallTarget::Python,
            path: ctx.output_path.clone(),
            install: ctx.options.install,
        },
        warnings: Vec::new(),
    })
}
