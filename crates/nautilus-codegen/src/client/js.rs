//! Generating the JavaScript client.

use anyhow::Result;

use super::{GeneratedClient, GenerationContext};
use crate::extension_types::generate_js_extension_files;
use crate::install::{Delivery, InstallTarget};
use crate::js::{
    generate_js_client, generate_js_composite_types, generate_js_enums, generate_js_models_index,
    js_runtime_files,
};
use crate::writer::{self, JsOutput};

pub(super) fn generate(ctx: &GenerationContext<'_>) -> Result<GeneratedClient> {
    let (js_models, dts_models) =
        crate::js::generator::generate_all_js_models_with_registry(ctx.ir, &ctx.registry)?;
    let (js_enums, dts_enums) = if !ctx.ir.enums.is_empty() {
        let (js, dts) = generate_js_enums(&ctx.ir.enums)?;
        (Some(js), Some(dts))
    } else {
        (None, None)
    };
    let dts_composite_types = generate_js_composite_types(&ctx.ir.composite_types)?;
    let (js_extension_files, dts_extension_files) = generate_js_extension_files(&ctx.registry)?;
    let (js_client, dts_client) = generate_js_client(&ctx.ir.models, &ctx.embedded_schema_path())?;
    let (js_models_index, dts_models_index) = generate_js_models_index(&js_models)?;
    let runtime = js_runtime_files();

    let package = writer::js::package(JsOutput {
        js_models: &js_models,
        dts_models: &dts_models,
        js_enums,
        dts_enums,
        dts_composite_types,
        js_extension_files: &js_extension_files,
        dts_extension_files: &dts_extension_files,
        js_client: Some(js_client),
        dts_client: Some(dts_client),
        js_models_index: Some(js_models_index),
        dts_models_index: Some(dts_models_index),
        runtime_files: &runtime,
    });

    Ok(GeneratedClient {
        language: "JavaScript",
        package,
        delivery: Delivery::Installable {
            target: InstallTarget::JavaScript,
            path: ctx.output_path.clone(),
            install: ctx.options.install,
        },
        warnings: Vec::new(),
    })
}
