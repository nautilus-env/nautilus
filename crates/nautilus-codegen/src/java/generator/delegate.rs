//! Java model delegates and their imports.

use anyhow::Result;
use nautilus_schema::ir::ModelIr;
use serde::Serialize;
use std::collections::BTreeSet;
use tera::Context;

use super::config::JavaConfig;
use super::templates::render;

#[derive(Debug, Serialize)]
struct DelegateTemplateContext {
    package_name: String,
    imports: Vec<String>,
    name: String,
    model_name: String,
    projection_name: String,
    dsl_name: String,
    is_async: bool,
    is_view: bool,
}

pub(super) fn generate_delegate_file(config: &JavaConfig, model: &ModelIr) -> Result<String> {
    let delegate_name = format!("{}Delegate", model.logical_name);
    let dsl_name = format!("{}Dsl", model.logical_name);
    let projection_name = format!("{}Projection", model.logical_name);

    let mut imports = BTreeSet::new();
    imports.insert(format!("{}.dsl.{}", config.root_package, dsl_name));
    imports.insert(format!("{}.internal.AbstractDelegate", config.root_package));
    imports.insert(format!("{}.internal.JsonSupport", config.root_package));
    imports.insert(format!(
        "{}.internal.NotFoundException",
        config.root_package
    ));
    imports.insert(format!(
        "{}.internal.NautilusException",
        config.root_package
    ));
    imports.insert(format!("{}.internal.RpcCaller", config.root_package));
    imports.insert(format!("{}.events.CrudEventContext", config.root_package));
    imports.insert(format!("{}.events.EventPhase", config.root_package));
    imports.insert(format!("{}.events.StopPropagation", config.root_package));
    imports.insert(format!(
        "{}.model.{}",
        config.root_package, model.logical_name
    ));
    imports.insert(format!("{}.model.{}", config.root_package, projection_name));
    imports.insert("com.fasterxml.jackson.databind.JsonNode".to_string());
    imports.insert("com.fasterxml.jackson.databind.node.ArrayNode".to_string());
    imports.insert("com.fasterxml.jackson.databind.node.ObjectNode".to_string());
    imports.insert("java.util.HashMap".to_string());
    imports.insert("java.util.List".to_string());
    imports.insert("java.util.Map".to_string());
    imports.insert("java.util.Objects".to_string());
    imports.insert("java.util.stream.Stream".to_string());
    if config.is_async {
        imports.insert("java.util.concurrent.CompletableFuture".to_string());
    }
    imports.insert("java.util.function.Consumer".to_string());
    imports.insert("java.util.function.Function".to_string());

    let context = Context::from_serialize(&DelegateTemplateContext {
        package_name: config.root_package.clone(),
        imports: imports.into_iter().collect(),
        name: delegate_name,
        model_name: model.logical_name.clone(),
        projection_name,
        dsl_name,
        is_async: config.is_async,
        is_view: model.is_view,
    })
    .expect("Java delegate context should serialize");
    render("java_delegate.tera", &context)
}
