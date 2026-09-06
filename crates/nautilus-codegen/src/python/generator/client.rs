//! Python client and package import surfaces.

use crate::GeneratedFile;
use anyhow::Result;
use heck::{ToPascalCase, ToSnakeCase};
use nautilus_schema::ir::ModelIr;
use serde::Serialize;
use std::collections::HashMap;
use tera::Context;

use super::templates::render;

/// Generate Python client file with model delegates.
///
/// `is_async` determines whether the generated `Nautilus` class exposes an async
/// context manager (`async with Nautilus(...) as db`) or a sync one (`with Nautilus(...) as db`).
pub fn generate_python_client(
    models: &HashMap<String, ModelIr>,
    schema_path: &str,
    is_async: bool,
) -> Result<String> {
    let mut context = Context::new();

    #[derive(Serialize)]
    struct ModelContext {
        snake_name: String,
        delegate_name: String,
    }

    let mut model_contexts: Vec<ModelContext> = models
        .values()
        .map(|m| ModelContext {
            snake_name: m.logical_name.to_snake_case(),
            delegate_name: format!("{}Delegate", m.logical_name),
        })
        .collect();
    model_contexts.sort_by(|a, b| a.snake_name.cmp(&b.snake_name));

    context.insert("models", &model_contexts);
    context.insert("schema_path", schema_path);
    context.insert("is_async", &is_async);

    render("client.py.tera", &context)
}

/// Generate package __init__.py
pub fn generate_package_init(has_enums: bool) -> Result<String> {
    let mut context = Context::new();
    context.insert("has_enums", &has_enums);

    render("package_init.py.tera", &context)
}

/// Generate models/__init__.py
pub fn generate_models_init(models: &[GeneratedFile]) -> Result<String> {
    let mut context = Context::new();

    let mut model_modules: Vec<String> = models
        .iter()
        .map(|(file_name, _)| file_name.trim_end_matches(".py").to_string())
        .collect();
    model_modules.sort();

    let mut model_classes: Vec<String> = model_modules.iter().map(|m| m.to_pascal_case()).collect();
    model_classes.sort();

    context.insert("model_modules", &model_modules);
    context.insert("model_classes", &model_classes);

    render("models_init.py.tera", &context)
}

/// Generate enums/__init__.py
pub fn generate_enums_init(has_enums: bool) -> Result<String> {
    let mut context = Context::new();
    context.insert("has_enums", &has_enums);

    render("enums_init.py.tera", &context)
}
