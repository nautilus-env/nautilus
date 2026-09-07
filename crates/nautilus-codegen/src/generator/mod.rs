//! Rust model generation and template context assembly.

use crate::extension_types::ExtensionRegistry;
use crate::model_view::ModelView;
use anyhow::{Context as _, Result};
use nautilus_schema::ir::{ModelIr, SchemaIr};
use std::collections::{HashMap, HashSet};
use tera::Context;

use fields::build_scalar_fields;
use keys::{build_pk_fields, build_single_record_constraints, build_vector_fields};
use ordering::build_nested_order_by_fields;
use relations::{
    build_nested_writes, build_relation_fields, build_relations, nested_write_imports,
};
use templates::render;

mod fields;
mod files;
mod keys;
mod ordering;
mod relations;
mod templates;

pub(crate) use files::generate_model_files;
pub use templates::TEMPLATES;

/// Generate complete code for a model (struct, impls, delegate, builders).
///
/// `is_async` determines whether the generated delegate methods and internal
/// builders use `async fn`/`.await` (`true`) or blocking sync wrappers (`false`).
/// This source-only API keeps returning one self-contained model module;
/// command-based generation emits the same items across included source files.
pub fn generate_model(model: &ModelIr, ir: &SchemaIr, is_async: bool) -> Result<String> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_model_with_registry(model, ir, is_async, &extensions)
}

fn generate_model_with_registry(
    model: &ModelIr,
    ir: &SchemaIr,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> Result<String> {
    let context = model_context(model, ir, is_async, extensions);
    render("model_file.tera", &context)
        .with_context(|| format!("Failed to generate Rust model '{}'", model.logical_name))
}

fn model_context(
    model: &ModelIr,
    ir: &SchemaIr,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> Context {
    let view = ModelView::new(model, ir, extensions);
    let mut context = Context::new();
    insert_derived_names(&mut context, &view);

    context.insert("primary_key_fields", &view.primary_key_fields);

    let pk_fields_with_db = build_pk_fields(&view);
    context.insert("pk_fields_with_db", &pk_fields_with_db);

    let vector_fields = build_vector_fields(&view);
    context.insert("has_vector_fields", &!vector_fields.is_empty());
    context.insert("vector_fields", &vector_fields);
    context.insert(
        "single_record_constraints",
        &build_single_record_constraints(model, &pk_fields_with_db),
    );

    let fields = build_scalar_fields(&view, extensions);
    let reserved_order_methods: HashSet<String> = fields
        .scalar
        .iter()
        .map(|field| field.name.clone())
        .collect();
    let nested_order_by_fields =
        build_nested_order_by_fields(model, ir, extensions, &reserved_order_methods);

    context.insert("has_enums", &!view.enum_imports.is_empty());
    context.insert("enum_imports", &view.enum_imports);
    context.insert("has_relations", &!view.relation_imports.is_empty());
    context.insert("relation_imports", &view.relation_imports);
    context.insert(
        "has_composite_types",
        &!view.composite_type_imports.is_empty(),
    );
    context.insert("composite_type_imports", &view.composite_type_imports);

    context.insert("scalar_fields", &fields.scalar);
    context.insert("relation_fields", &build_relation_fields(&view, extensions));
    context.insert("relations", &build_relations(&view, ir, extensions));

    let nested_writes = build_nested_writes(&view);
    context.insert("has_nested_writes", &!nested_writes.is_empty());
    context.insert(
        "nested_write_imports",
        &nested_write_imports(&view, &nested_writes),
    );
    context.insert("nested_writes", &nested_writes);

    context.insert("create_fields", &fields.create);
    context.insert("updated_at_fields", &fields.updated_at);
    context.insert("all_scalar_fields", &fields.scalar);
    context.insert("numeric_fields", &fields.numeric);
    context.insert("orderable_fields", &fields.orderable);
    context.insert("nested_order_by_fields", &nested_order_by_fields);
    context.insert("has_numeric_fields", &!fields.numeric.is_empty());
    context.insert("has_orderable_fields", &!fields.orderable.is_empty());
    context.insert("is_async", &is_async);

    context
}

/// Insert the `{Model}Delegate` / `{Model}FindMany` / … type names the
/// templates refer to.
fn insert_derived_names(context: &mut Context, view: &ModelView<'_>) {
    let name = view.logical_name();
    context.insert("model_name", name);
    context.insert("table_name", view.db_name());
    context.insert("is_view", &view.model.is_view);
    context.insert("delegate_name", &format!("{}Delegate", name));
    context.insert("columns_name", &format!("{}Columns", name));
    context.insert("find_many_name", &format!("{}FindMany", name));
    context.insert("create_name", &format!("{}Create", name));
    context.insert("create_many_name", &format!("{}CreateMany", name));
    context.insert("entry_name", &format!("{}CreateEntry", name));
    context.insert("update_name", &format!("{}Update", name));
    context.insert("delete_name", &format!("{}Delete", name));
}

/// Generate all models from a schema IR.
///
/// `is_async` is forwarded to every [`generate_model`] call.
pub fn generate_all_models(ir: &SchemaIr, is_async: bool) -> Result<HashMap<String, String>> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_all_models_with_registry(ir, is_async, &extensions)
}

pub(crate) fn generate_all_models_with_registry(
    ir: &SchemaIr,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> Result<HashMap<String, String>> {
    let mut generated = HashMap::new();

    for (model_name, model_ir) in &ir.models {
        let code = generate_model_with_registry(model_ir, ir, is_async, extensions)?;
        generated.insert(model_name.clone(), code);
    }

    Ok(generated)
}
