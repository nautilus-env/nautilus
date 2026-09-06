//! JavaScript client and model exports with their TypeScript declarations.

use crate::GeneratedFile;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToSnakeCase};
use nautilus_schema::ir::ModelIr;
use serde::Serialize;
use std::collections::HashMap;
use tera::Context;

use super::templates::render;

/// Generate `index.js` + `index.d.ts` — the typed `Nautilus` class with model delegates.
///
/// Returns `(js_code, dts_code)`.
pub fn generate_js_client(
    models: &HashMap<String, ModelIr>,
    schema_path: &str,
) -> Result<(String, String)> {
    #[derive(Serialize)]
    struct ModelCtx {
        /// camelCase — property name on `Nautilus`, e.g. `user`.
        camel_name: String,
        /// snake_case — import file name, e.g. `user`.
        snake_name: String,
        /// PascalCase + "Delegate", e.g. `UserDelegate`.
        delegate_name: String,
    }

    let mut model_list: Vec<ModelCtx> = models
        .values()
        .map(|m| ModelCtx {
            camel_name: m.logical_name.to_lower_camel_case(),
            snake_name: m.logical_name.to_snake_case(),
            delegate_name: format!("{}Delegate", m.logical_name),
        })
        .collect();
    model_list.sort_by(|a, b| a.camel_name.cmp(&b.camel_name));

    let mut context = Context::new();
    context.insert("models", &model_list);
    context.insert("schema_path", schema_path);
    let js_code = render("client.js.tera", &context)?;
    let dts_code = render("client.d.ts.tera", &context)?;
    Ok((js_code, dts_code))
}

/// Generate `models/index.js` + `models/index.d.ts` — barrel re-exports for all model files.
///
/// `js_models` contains the `.js` model filenames. Returns `(js_code, dts_code)`.
pub fn generate_js_models_index(js_models: &[GeneratedFile]) -> Result<(String, String)> {
    let mut modules: Vec<String> = js_models
        .iter()
        .map(|(file_name, _)| file_name.trim_end_matches(".js").to_string())
        .collect();
    modules.sort();

    let mut context = Context::new();
    context.insert("model_modules", &modules);
    let js_code = render("models_index.js.tera", &context)?;
    let dts_code = render("models_index.d.ts.tera", &context)?;
    Ok((js_code, dts_code))
}
