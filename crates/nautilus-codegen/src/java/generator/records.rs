//! Java enum, model and composite records.

use crate::java::type_mapper::{composite_field_to_java_type, field_to_java_type};
use anyhow::Result;
use heck::ToLowerCamelCase;
use nautilus_schema::ir::{CompositeTypeIr, EnumIr, ModelIr};
use serde::Serialize;
use std::collections::BTreeSet;
use tera::Context;

use super::config::JavaConfig;
use super::readers::{generate_composite_field_read, generate_model_field_read};
use super::templates::render;

#[derive(Debug, Serialize)]
struct EnumTemplateContext {
    package_name: String,
    name: String,
    variants: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RecordComponentContext {
    ty: String,
    name: String,
    doc_comment: String,
}

#[derive(Debug, Serialize)]
struct RecordTemplateContext {
    package_name: String,
    imports: Vec<String>,
    name: String,
    components: Vec<RecordComponentContext>,
    reads: Vec<String>,
    writes: Vec<String>,
    ctor_args: Vec<String>,
    static_delegate: Option<String>,
    static_delegate_accessor: Option<String>,
    implements_type: String,
}

/// Per-field render artifacts collected by [`build_record_context`].
struct RecordField {
    component: RecordComponentContext,
    imports: BTreeSet<String>,
    read: String,
    ctor_arg: String,
}

pub(super) fn generate_enum_file(config: &JavaConfig, enum_ir: &EnumIr) -> Result<String> {
    let context = Context::from_serialize(&EnumTemplateContext {
        package_name: config.root_package.clone(),
        name: enum_ir.logical_name.clone(),
        variants: enum_ir.variants.clone(),
    })
    .expect("Java enum context should serialize");
    render("java_enum.tera", &context)
}

/// Shared imports every Java record template (composite, model) needs for
/// Jackson serialization helpers plus the crate-local `JsonSupport`.
fn base_record_imports(root_package: &str) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    imports.insert(format!("{root_package}.internal.JsonSupport"));
    imports.insert("com.fasterxml.jackson.databind.JsonNode".to_string());
    imports.insert("com.fasterxml.jackson.databind.node.ObjectNode".to_string());
    imports
}

/// Render the per-field `toJsonNode` write guard used by composite and model
/// records. Field name doubles as the wire key because composite and model
/// records both serialize using the logical name unchanged.
fn format_record_field_write(field_name: &str) -> String {
    format!(
        "        if (this.{field_name} != null) {{\n            node.set(\"{field_name}\", JsonSupport.toJsonNode(this.{field_name}));\n        }}\n",
    )
}

/// Assemble the Tera [`Context`] shared by `composite.java.tera` and
/// `model.java.tera`. Callers supply the record name, the extra imports
/// (beyond the Jackson/JsonSupport base set), the per-field renderer, and the
/// optional static-delegate accessor that only models emit.
fn build_record_context<I, F>(
    config: &JavaConfig,
    name: String,
    extra_imports: impl IntoIterator<Item = String>,
    fields: I,
    mut per_field: F,
    implements_type: String,
    static_delegate: Option<(String, String)>,
) -> Context
where
    I: IntoIterator,
    F: FnMut(I::Item) -> RecordField,
{
    let mut imports = base_record_imports(&config.root_package);
    imports.extend(extra_imports);

    let mut components = Vec::new();
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut ctor_args = Vec::new();
    for field in fields {
        let rendered = per_field(field);
        imports.extend(rendered.imports);
        writes.push(format_record_field_write(&rendered.component.name));
        components.push(rendered.component);
        reads.push(rendered.read);
        ctor_args.push(rendered.ctor_arg);
    }

    let (static_delegate, static_delegate_accessor) = match static_delegate {
        Some((delegate, accessor)) => (Some(delegate), Some(accessor)),
        None => (None, None),
    };

    Context::from_serialize(&RecordTemplateContext {
        package_name: config.root_package.clone(),
        imports: imports.into_iter().collect(),
        name,
        components,
        reads,
        writes,
        ctor_args,
        static_delegate,
        static_delegate_accessor,
        implements_type,
    })
    .expect("Java record context should serialize")
}

pub(super) fn generate_composite_file(
    config: &JavaConfig,
    composite: &CompositeTypeIr,
) -> Result<String> {
    let extra_imports = [format!("{}.internal.WireSerializable", config.root_package)];
    let context = build_record_context(
        config,
        composite.logical_name.clone(),
        extra_imports,
        &composite.fields,
        |field| {
            let (ty, field_imports) = composite_field_to_java_type(
                field,
                &config.root_package,
                &composite.logical_name,
                &config.extensions,
            );
            RecordField {
                component: RecordComponentContext {
                    ty,
                    name: field.logical_name.clone(),
                    doc_comment: String::new(),
                },
                imports: field_imports,
                read: generate_composite_field_read(config, field),
                ctor_arg: field.logical_name.clone(),
            }
        },
        "WireSerializable".to_string(),
        None,
    );
    render("java_composite.tera", &context)
}

pub(super) fn generate_model_file(config: &JavaConfig, model: &ModelIr) -> Result<String> {
    let extra_imports = [
        format!("{}.client.Nautilus", config.root_package),
        format!(
            "{}.client.{}Delegate",
            config.root_package, model.logical_name
        ),
        format!("{}.internal.GlobalNautilusRegistry", config.root_package),
    ];
    let context = build_record_context(
        config,
        model.logical_name.clone(),
        extra_imports,
        &model.fields,
        |field| {
            let (ty, field_imports) = field_to_java_type(
                field,
                &config.root_package,
                &model.logical_name,
                &config.extensions,
            );
            RecordField {
                component: RecordComponentContext {
                    ty,
                    name: field.logical_name.clone(),
                    doc_comment: crate::schema_docs::field_modifier_doc(model, field),
                },
                imports: field_imports,
                read: generate_model_field_read(config, model, field),
                ctor_arg: field.logical_name.clone(),
            }
        },
        "NautilusModel".to_string(),
        Some((
            format!("{}Delegate", model.logical_name),
            model.logical_name.to_lower_camel_case(),
        )),
    );
    render("java_model.tera", &context)
}
