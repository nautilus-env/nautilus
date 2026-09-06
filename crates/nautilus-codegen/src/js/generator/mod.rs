//! JavaScript model generation and declaration context assembly.

use crate::extension_types::ExtensionRegistry;
use crate::model_view::ModelView;
use crate::{GeneratedFile, GeneratedJsFiles};
use anyhow::{Context as _, Result};
use nautilus_schema::ir::{ModelIr, SchemaIr};
use serde::Serialize;
use tera::Context;

use fields::build_scalar_fields;
use relations::{build_include_fields, build_relation_fields};
use templates::render;

mod client;
mod fields;
mod relations;
mod runtime;
mod templates;
mod types;

pub use client::{generate_js_client, generate_js_models_index};
pub use runtime::js_runtime_files;
pub use templates::JS_TEMPLATES;
pub use types::{generate_js_composite_types, generate_js_enums};

/// Generate JavaScript + declaration code for a single model.
///
/// Returns `((js_filename, js_code), (dts_filename, dts_code))`.
pub fn generate_js_model(model: &ModelIr, ir: &SchemaIr) -> Result<(GeneratedFile, GeneratedFile)> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_js_model_with_registry(model, ir, &extensions)
}

fn generate_js_model_with_registry(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Result<(GeneratedFile, GeneratedFile)> {
    let view = ModelView::new(model, ir, extensions);
    let mut context = Context::new();
    crate::template::insert_protocol_version(&mut context);

    context.insert("model_name", view.logical_name());
    context.insert("snake_name", &view.snake_name());
    context.insert("table_name", view.db_name());
    context.insert("is_view", &view.model.is_view);
    context.insert("delegate_name", &format!("{}Delegate", view.logical_name()));
    context.insert("primary_key_fields", &view.primary_key_fields);

    let fields = build_scalar_fields(&view, ir, extensions);

    context.insert("scalar_fields", &fields.scalar);
    context.insert("relation_fields", &build_relation_fields(&view));
    context.insert("where_input_fields", &fields.where_input);
    context.insert("create_input_fields", &fields.create_input);
    context.insert("update_input_fields", &fields.update_input);
    context.insert("order_by_fields", &fields.order_by);
    context.insert("include_fields", &build_include_fields(&view));
    context.insert("has_includes", &!view.relations.is_empty());
    context.insert("numeric_fields", &fields.numeric);
    context.insert("orderable_fields", &fields.orderable);
    context.insert("object_value_db_fields", &view.object_value_db_names);
    context.insert("has_numeric_fields", &!fields.numeric.is_empty());
    context.insert("has_vector_fields", &!view.vector_field_names.is_empty());
    context.insert("vector_field_names", &view.vector_field_names);
    for (flag, value) in extensions.template_flags() {
        context.insert(&flag, &value);
    }
    context.insert("has_enums", &!view.enum_imports.is_empty());
    context.insert("enum_imports", &view.enum_imports);
    context.insert(
        "has_composite_types",
        &!view.composite_type_imports.is_empty(),
    );
    context.insert("composite_type_imports", &view.composite_type_imports);

    let extension_imports = build_extension_imports(&view);
    context.insert("has_extension_types", &!extension_imports.is_empty());
    context.insert("extension_imports", &extension_imports);

    let snake = view.snake_name();
    let describe = || {
        format!(
            "Failed to generate JavaScript model '{}'",
            view.logical_name()
        )
    };
    let js_code = render("model.js.tera", &context).with_context(describe)?;
    let dts_code = render("model.d.ts.tera", &context).with_context(describe)?;

    Ok((
        (format!("{}.js", snake), js_code),
        (format!("{}.d.ts", snake), dts_code),
    ))
}

fn build_extension_imports(view: &ModelView<'_>) -> Vec<JsExtensionImportContext> {
    view.extension_import_views()
        .into_iter()
        .map(|import| JsExtensionImportContext {
            module: import.module,
            types: import.types,
            input_types: import.input_types,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
struct JsExtensionImportContext {
    module: String,
    types: Vec<String>,
    input_types: Vec<String>,
}

/// Generate JavaScript + declaration code for all models in the schema.
///
/// Returns `(js_models, dts_models)`, each sorted by filename.
pub fn generate_all_js_models(ir: &SchemaIr) -> Result<GeneratedJsFiles> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_all_js_models_with_registry(ir, &extensions)
}

pub(crate) fn generate_all_js_models_with_registry(
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Result<GeneratedJsFiles> {
    let pairs: Vec<(GeneratedFile, GeneratedFile)> = ir
        .models
        .values()
        .map(|model| generate_js_model_with_registry(model, ir, extensions))
        .collect::<Result<Vec<_>>>()?;

    let mut js_models: Vec<GeneratedFile> = pairs.iter().map(|(js, _)| js.clone()).collect();
    let mut dts_models: Vec<GeneratedFile> = pairs.iter().map(|(_, dts)| dts.clone()).collect();

    js_models.sort_by(|a, b| a.0.cmp(&b.0));
    dts_models.sort_by(|a, b| a.0.cmp(&b.0));

    Ok((js_models, dts_models))
}
