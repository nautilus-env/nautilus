//! Python model generation and template context assembly.

use crate::extension_types::ExtensionRegistry;
use crate::model_view::ModelView;
use crate::GeneratedFile;
use anyhow::{Context as _, Result};
use nautilus_schema::ir::{ModelIr, SchemaIr};
use serde::Serialize;
use tera::Context;

use fields::build_scalar_fields;
use relations::{build_include_fields, build_relation_fields, build_relations};
use templates::render;

mod client;
mod fields;
mod relations;
mod runtime;
mod templates;
mod types;

pub use client::{
    generate_enums_init, generate_models_init, generate_package_init, generate_python_client,
};
pub use runtime::{
    generate_errors_init, generate_events_init, generate_internal_init, generate_transaction_init,
    python_runtime_files,
};
pub use templates::PYTHON_TEMPLATES;
pub use types::{generate_python_composite_types, generate_python_enums};

/// Generate complete Python code for a model.
///
/// `is_async` determines whether delegate methods use `async def`/`await` (`true`)
/// or synchronous `def` + `asyncio.run()` wrappers (`false`).
/// `recursive_type_depth` controls the depth of generated recursive include TypedDicts.
pub fn generate_python_model(
    model: &ModelIr,
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
) -> Result<GeneratedFile> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_python_model_with_registry(model, ir, is_async, recursive_type_depth, &extensions)
}

fn generate_python_model_with_registry(
    model: &ModelIr,
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
    extensions: &ExtensionRegistry,
) -> Result<GeneratedFile> {
    let view = ModelView::new(model, ir, extensions);
    let mut context = Context::new();
    crate::template::insert_protocol_version(&mut context);
    insert_derived_names(&mut context, &view);

    context.insert("primary_key_fields", &view.primary_key_fields);

    let fields = build_scalar_fields(&view, ir, extensions);

    context.insert("has_datetime", &fields.has_datetime);
    context.insert("has_uuid", &fields.has_uuid);
    context.insert("has_decimal", &fields.has_decimal);
    context.insert("has_dict", &fields.has_dict);
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
    context.insert("has_relations", &!view.relation_imports.is_empty());
    context.insert("relation_imports", &view.relation_imports);

    context.insert("needs_typeddict", &true);
    context.insert("where_input_fields", &fields.where_input);
    context.insert("create_input_fields", &fields.create_input);
    context.insert("update_input_fields", &fields.update_input);
    context.insert("order_by_fields", &fields.order_by);
    context.insert(
        "has_dotted_order_by_fields",
        &!view.dotted_order_by.is_empty(),
    );
    context.insert("include_fields", &build_include_fields(&view));
    context.insert("has_includes", &!view.relations.is_empty());
    context.insert("numeric_fields", &fields.numeric);
    context.insert("orderable_fields", &fields.orderable);
    context.insert("object_value_db_fields", &view.object_value_db_names);
    context.insert("has_numeric_fields", &!fields.numeric.is_empty());
    context.insert("has_orderable_fields", &!fields.orderable.is_empty());
    context.insert("has_vector_fields", &!view.vector_field_names.is_empty());
    context.insert("vector_field_names", &view.vector_field_names);

    context.insert("scalar_fields", &fields.scalar);
    context.insert(
        "relation_fields",
        &build_relation_fields(&view, ir, extensions),
    );
    context.insert("create_fields", &fields.create);
    context.insert("relations", &build_relations(&view));
    context.insert("is_async", &is_async);
    context.insert("recursive_type_depth", &recursive_type_depth);

    let model_code = render("model_file.py.tera", &context)
        .with_context(|| format!("Failed to generate Python model '{}'", view.logical_name()))?;

    Ok((format!("{}.py", view.snake_name()), model_code))
}

/// Insert the `{Model}Delegate` / `{Model}FindMany` / … class names the
/// templates refer to.
fn insert_derived_names(context: &mut Context, view: &ModelView<'_>) {
    let name = view.logical_name();
    context.insert("model_name", name);
    context.insert("snake_name", &view.snake_name());
    context.insert("table_name", view.db_name());
    context.insert("is_view", &view.model.is_view);
    context.insert("delegate_name", &format!("{}Delegate", name));
    context.insert("find_many_name", &format!("{}FindMany", name));
    context.insert("create_name", &format!("{}Create", name));
    context.insert("create_many_name", &format!("{}CreateMany", name));
    context.insert("update_name", &format!("{}Update", name));
    context.insert("delete_name", &format!("{}Delete", name));
}

fn build_extension_imports(view: &ModelView<'_>) -> Vec<ExtensionImportContext> {
    view.extension_import_views()
        .into_iter()
        .map(|import| {
            let mut symbols = import.types.clone();
            symbols.extend(import.input_types.iter().cloned());
            ExtensionImportContext {
                module: import.module,
                symbols,
                types: import.types,
                input_types: import.input_types,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
struct ExtensionImportContext {
    module: String,
    symbols: Vec<String>,
    types: Vec<String>,
    input_types: Vec<String>,
}

/// Generate all Python models.
///
/// `is_async` is forwarded to every [`generate_python_model`] call.
/// `recursive_type_depth` controls the depth of generated recursive include TypedDicts.
pub fn generate_all_python_models(
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
) -> Result<Vec<GeneratedFile>> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_all_python_models_with_registry(ir, is_async, recursive_type_depth, &extensions)
}

pub(crate) fn generate_all_python_models_with_registry(
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
    extensions: &ExtensionRegistry,
) -> Result<Vec<GeneratedFile>> {
    ir.models
        .values()
        .map(|model| {
            generate_python_model_with_registry(
                model,
                ir,
                is_async,
                recursive_type_depth,
                extensions,
            )
        })
        .collect()
}
