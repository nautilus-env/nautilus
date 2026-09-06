//! Java generation entry points and source layout.

use crate::extension_types::{generate_java_extension_files, ExtensionRegistry};
use crate::GeneratedFile;
use anyhow::{anyhow, Context as _, Result};
use nautilus_schema::ir::{EnumIr, SchemaIr};
use std::collections::BTreeMap;

use client::{
    generate_nautilus_client, generate_pom, generate_transaction_client, sorted_model_meta,
};
use config::{JavaConfig, DEFAULT_MAVEN_VERSION};
use delegate::generate_delegate_file;
use dsl::generate_dsl_file;
use projections::generate_projection_file;
use records::{generate_composite_file, generate_enum_file, generate_model_file};
use runtime::java_event_files;
use templates::render_pkg;

mod client;
mod config;
mod delegate;
mod dsl;
mod projections;
mod readers;
mod records;
mod runtime;
mod templates;

pub(crate) use config::JACKSON_VERSION;
pub use runtime::java_runtime_files;

/// Public entry point: generate all Java source files for the given schema.
pub fn generate_java_client(
    ir: &SchemaIr,
    schema_path: &str,
    is_async: bool,
) -> Result<Vec<GeneratedFile>> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_java_client_with_registry(ir, schema_path, is_async, &extensions)
}

pub(crate) fn generate_java_client_with_registry(
    ir: &SchemaIr,
    schema_path: &str,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> Result<Vec<GeneratedFile>> {
    let generator = ir
        .generator
        .as_ref()
        .ok_or_else(|| anyhow!("Java generation requires a generator block"))?;

    let config = JavaConfig {
        root_package: generator
            .java_package
            .clone()
            .ok_or_else(|| anyhow!("Java generation requires generator.package"))?,
        group_id: generator
            .java_group_id
            .clone()
            .ok_or_else(|| anyhow!("Java generation requires generator.group_id"))?,
        artifact_id: generator
            .java_artifact_id
            .clone()
            .ok_or_else(|| anyhow!("Java generation requires generator.artifact_id"))?,
        version: DEFAULT_MAVEN_VERSION.to_string(),
        schema_path: schema_path.to_string(),
        is_async,
        extensions: extensions.clone(),
    };

    let models = sorted_model_meta(ir.models.values());
    let enums_map: BTreeMap<String, EnumIr> = ir
        .enums
        .iter()
        .map(|(name, item)| (name.clone(), item.clone()))
        .collect();

    let mut files = Vec::new();
    files.push(("pom.xml".to_string(), generate_pom(&config)?));
    files.extend(java_runtime_files(&config.root_package)?);
    files.push((
        java_source_path(&config.root_package, "model", "NautilusModel.java"),
        render_pkg("java_nautilus_model.tera", &config.root_package)?,
    ));
    files.push((
        java_source_path(&config.root_package, "dsl", "SortOrder.java"),
        render_pkg("java_sort_order.tera", &config.root_package)?,
    ));
    files.push((
        java_source_path(&config.root_package, "dsl", "Filters.java"),
        render_pkg("java_filters.tera", &config.root_package)?,
    ));
    files.push((
        java_source_path(&config.root_package, "client", "NautilusOptions.java"),
        render_pkg("java_nautilus_options.tera", &config.root_package)?,
    ));
    files.push((
        java_source_path(&config.root_package, "client", "IsolationLevel.java"),
        render_pkg("java_isolation_level.tera", &config.root_package)?,
    ));
    files.push((
        java_source_path(&config.root_package, "client", "TransactionOptions.java"),
        render_pkg("java_transaction_options.tera", &config.root_package)?,
    ));
    files.push((
        java_source_path(
            &config.root_package,
            "client",
            "TransactionBatchOperation.java",
        ),
        render_pkg("java_transaction_batch_op.tera", &config.root_package)?,
    ));
    files.extend(java_event_files(&config.root_package)?);
    files.extend(generate_java_extension_files(
        &config.extensions,
        &config.root_package,
    )?);

    for enum_ir in sorted_named(ir.enums.values(), |item| item.logical_name.clone()) {
        files.push((
            java_source_path(
                &config.root_package,
                "enums",
                &format!("{}.java", enum_ir.logical_name),
            ),
            generate_enum_file(&config, enum_ir)?,
        ));
    }

    for composite in sorted_named(ir.composite_types.values(), |item| {
        item.logical_name.clone()
    }) {
        files.push((
            java_source_path(
                &config.root_package,
                "types",
                &format!("{}.java", composite.logical_name),
            ),
            generate_composite_file(&config, composite)?,
        ));
    }

    for model in sorted_named(ir.models.values(), |item| item.logical_name.clone()) {
        files.push((
            java_source_path(
                &config.root_package,
                "dsl",
                &format!("{}Dsl.java", model.logical_name),
            ),
            generate_dsl_file(&config, model, ir, &enums_map)?,
        ));
        files.push((
            java_source_path(
                &config.root_package,
                "client",
                &format!("{}Delegate.java", model.logical_name),
            ),
            generate_delegate_file(&config, model)?,
        ));
        files.push((
            java_source_path(
                &config.root_package,
                "model",
                &format!("{}.java", model.logical_name),
            ),
            generate_model_file(&config, model).with_context(|| {
                format!("Failed to generate Java model '{}'", model.logical_name)
            })?,
        ));
        files.push((
            java_source_path(
                &config.root_package,
                "model",
                &format!("{}Projection.java", model.logical_name),
            ),
            generate_projection_file(&config, model)?,
        ));
    }

    files.push((
        java_source_path(&config.root_package, "client", "TransactionClient.java"),
        generate_transaction_client(&config, &models)?,
    ));
    files.push((
        java_source_path(&config.root_package, "client", "Nautilus.java"),
        generate_nautilus_client(&config, &models)?,
    ));

    Ok(files)
}

fn sorted_named<T, F>(items: impl Iterator<Item = T>, key: F) -> Vec<T>
where
    T: Clone,
    F: Fn(&T) -> String,
{
    let mut values: Vec<T> = items.collect();
    values.sort_by_key(|item| key(item));
    values
}

pub(super) fn java_source_path(root_package: &str, subpackage: &str, file_name: &str) -> String {
    let package_path = root_package.replace('.', "/");
    format!(
        "src/main/java/{}/{}/{}",
        package_path, subpackage, file_name
    )
}
