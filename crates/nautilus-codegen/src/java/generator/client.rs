//! Java client, transaction client and Maven manifest.

use anyhow::Result;
use heck::ToLowerCamelCase;
use nautilus_schema::ir::ModelIr;
use serde::Serialize;
use std::collections::BTreeSet;
use tera::Context;

use super::config::{JavaConfig, JACKSON_VERSION};
use super::templates::render;

#[derive(Debug, Clone)]
pub(super) struct ModelMeta {
    name: String,
    camel: String,
    delegate_name: String,
}

#[derive(Debug, Serialize)]
struct PomContext {
    group_id: String,
    artifact_id: String,
    version: String,
    jackson_version: String,
}

#[derive(Debug, Serialize)]
struct ClientModelContext {
    camel: String,
    delegate_name: String,
}

#[derive(Debug, Serialize)]
struct TransactionClientTemplateContext {
    package_name: String,
    imports: Vec<String>,
    models: Vec<ClientModelContext>,
}

#[derive(Debug, Serialize)]
struct NautilusTemplateContext {
    package_name: String,
    imports: Vec<String>,
    schema_path_literal: String,
    models: Vec<ClientModelContext>,
    is_async: bool,
}

pub(super) fn generate_pom(config: &JavaConfig) -> Result<String> {
    let context = Context::from_serialize(&PomContext {
        group_id: config.group_id.clone(),
        artifact_id: config.artifact_id.clone(),
        version: config.version.clone(),
        jackson_version: JACKSON_VERSION.to_string(),
    })
    .expect("Java pom context should serialize");
    render("java_pom.tera", &context)
}

pub(super) fn generate_transaction_client(
    config: &JavaConfig,
    models: &[ModelMeta],
) -> Result<String> {
    let mut imports = BTreeSet::new();
    imports.insert(format!(
        "{}.internal.BaseNautilusClient",
        config.root_package
    ));
    imports.insert(format!(
        "{}.internal.BaseTransactionClient",
        config.root_package
    ));
    for model in models {
        imports.insert(format!(
            "{}.client.{}",
            config.root_package, model.delegate_name
        ));
    }

    let context = Context::from_serialize(&TransactionClientTemplateContext {
        package_name: config.root_package.clone(),
        imports: imports.into_iter().collect(),
        models: models
            .iter()
            .map(|model| ClientModelContext {
                camel: model.camel.clone(),
                delegate_name: model.delegate_name.clone(),
            })
            .collect(),
    })
    .expect("Java transaction client context should serialize");
    render("java_transaction_client.tera", &context)
}

pub(super) fn generate_nautilus_client(
    config: &JavaConfig,
    models: &[ModelMeta],
) -> Result<String> {
    let mut imports = BTreeSet::new();
    imports.insert(format!(
        "{}.internal.BaseNautilusClient",
        config.root_package
    ));
    imports.insert(format!(
        "{}.internal.GlobalNautilusRegistry",
        config.root_package
    ));
    imports.insert("com.fasterxml.jackson.databind.JsonNode".to_string());
    imports.insert("java.util.List".to_string());
    if config.is_async {
        imports.insert("java.util.concurrent.CompletableFuture".to_string());
        imports.insert("java.util.concurrent.CompletionException".to_string());
    }
    imports.insert("java.util.function.Function".to_string());
    for model in models {
        imports.insert(format!(
            "{}.client.{}",
            config.root_package, model.delegate_name
        ));
    }

    let context = Context::from_serialize(&NautilusTemplateContext {
        package_name: config.root_package.clone(),
        imports: imports.into_iter().collect(),
        schema_path_literal: format!("{:?}", config.schema_path),
        models: models
            .iter()
            .map(|model| ClientModelContext {
                camel: model.camel.clone(),
                delegate_name: model.delegate_name.clone(),
            })
            .collect(),
        is_async: config.is_async,
    })
    .expect("Java Nautilus client context should serialize");
    render("java_nautilus.tera", &context)
}

pub(super) fn sorted_model_meta<'a>(models: impl Iterator<Item = &'a ModelIr>) -> Vec<ModelMeta> {
    let mut values: Vec<ModelMeta> = models
        .map(|model| ModelMeta {
            name: model.logical_name.clone(),
            camel: model.logical_name.to_lower_camel_case(),
            delegate_name: format!("{}Delegate", model.logical_name),
        })
        .collect();
    values.sort_by(|left, right| left.name.cmp(&right.name));
    values
}
