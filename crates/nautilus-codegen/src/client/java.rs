//! Generating the Java client.

use anyhow::Result;

use nautilus_schema::ir::JavaGenerationMode;

use super::{GeneratedClient, GenerationContext};
use crate::install::Delivery;
use crate::writer;
use crate::InstallMode;

pub(super) fn generate(ctx: &GenerationContext<'_>) -> Result<GeneratedClient> {
    let java_mode = ctx
        .ir
        .generator
        .as_ref()
        .and_then(|g| g.java_mode)
        .unwrap_or(JavaGenerationMode::Maven);

    let mut warnings = Vec::new();
    if ctx.options.install == InstallMode::Always {
        warnings.push(match java_mode {
            JavaGenerationMode::Maven => "install = true is currently ignored for 'nautilus-client-java'; the generated Maven module is written only to the configured output path".to_string(),
            JavaGenerationMode::Jar => "install = true is currently ignored for 'nautilus-client-java'; generation writes the Maven module to the configured output path and the plain Java bundle to output/dist".to_string(),
        });
    }

    let output_path = ctx
        .output_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Java generation requires generator.output"))?;

    let files = crate::java::generator::generate_java_client_with_registry(
        ctx.ir,
        &ctx.embedded_schema_path(),
        ctx.is_async,
        &ctx.registry,
    )?;

    let bundle = match java_mode {
        JavaGenerationMode::Jar => Some(
            ctx.ir
                .generator
                .as_ref()
                .and_then(|g| g.java_artifact_id.as_deref())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Java bundle mode requires generator.artifact_id to build the jar name"
                    )
                })?
                .to_string(),
        ),
        JavaGenerationMode::Maven => None,
    };

    Ok(GeneratedClient {
        language: "Java",
        package: writer::java::package(&files),
        delivery: Delivery::JavaModule {
            path: output_path.to_string(),
            bundle,
        },
        warnings,
    })
}
