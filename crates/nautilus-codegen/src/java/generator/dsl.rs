//! Java filter, write, aggregate and ordering builders.

use crate::java::type_mapper::{
    extension_raw_java_type, field_base_type, field_to_java_type, filter_operators_for_field,
    is_writable_on_create, is_writable_on_update,
};
use crate::model_view::ModelView;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use nautilus_schema::ir::{EnumIr, ModelIr, SchemaIr};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tera::Context;

use super::config::JavaConfig;
use super::templates::render;

#[derive(Debug, Serialize)]
struct DslFilterOpCtx {
    /// Raw wire operator suffix, e.g. `"gt"`.
    suffix: String,
    /// PascalCase suffix for the Java method name, e.g. `"Gt"`.
    suffix_pascal: String,
    /// Java type for the method parameter, e.g. `"Integer"`.
    java_type: String,
}

#[derive(Debug, Serialize)]
struct DslScalarFieldCtx {
    /// Logical field name – used as method name, ScalarField enum wire value, Select/OrderBy key.
    name: String,
    /// PascalCase variant name for the ScalarField enum.
    variant_name: String,
    /// DB column name – used as Where/CreateInput/UpdateInput wire key.
    db_name: String,
    /// Base Java type for the Where `equals` method parameter.
    java_type: String,
    /// Raw compatibility type when `java_type` is a generated extension wrapper.
    raw_java_type: String,
    filter_ops: Vec<DslFilterOpCtx>,
}

#[derive(Debug, Serialize)]
struct DslWritableFieldCtx {
    /// Logical field name – Java setter method name.
    name: String,
    /// DB column name – wire key sent to the engine.
    db_name: String,
    /// Full Java type (e.g. `"List<String>"` for arrays).
    java_type: String,
    /// Raw compatibility type when `java_type` is a generated extension wrapper.
    raw_java_type: String,
    /// PascalCase form of `name`, used to build the operator method names.
    method_suffix: String,
    /// `true` when the column's type lets an update derive the new value from
    /// the current one. Mirrors the engine's own rule.
    accepts_arithmetic: bool,
}

#[derive(Debug, Serialize)]
struct DslRelationFieldCtx {
    name: String,
    target_model: String,
}

/// Template context for the nested-write builder a relation contributes to
/// `CreateInput` and `UpdateInput`.
#[derive(Debug, Serialize)]
struct DslNestedWriteCtx {
    /// Builder method name on the create/update input.
    method_name: String,
    /// Name the engine matches the relation by.
    wire_name: String,
    target_model: String,
    /// The DSL class carrying the target model's inputs and filters.
    target_dsl: String,
    create_nested_name: String,
    update_nested_name: String,
    is_owning: bool,
}

#[derive(Debug, Serialize)]
struct DslOrderByFieldCtx {
    method_name: String,
    wire_name: String,
}

#[derive(Debug, Serialize)]
struct DslTemplateContext {
    package_name: String,
    imports: Vec<String>,
    name: String,
    scalar_fields: Vec<DslScalarFieldCtx>,
    create_fields: Vec<DslWritableFieldCtx>,
    update_fields: Vec<DslWritableFieldCtx>,
    relation_fields: Vec<DslRelationFieldCtx>,
    nested_writes: Vec<DslNestedWriteCtx>,
    numeric_field_names: Vec<String>,
    order_by_fields: Vec<DslOrderByFieldCtx>,
    orderable_field_names: Vec<String>,
    vector_field_names: Vec<String>,
    all_scalar_field_names: Vec<String>,
}

/// Reserve a DSL order-by method name, suffixing it until it stops colliding
/// with a scalar field's own order-by method.
fn unique_order_by_method(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_string();
    let mut suffix = 0usize;
    while used.contains(&name) {
        suffix += 1;
        name = if suffix == 1 {
            format!("{base}Order")
        } else {
            format!("{base}Order{suffix}")
        };
    }
    used.insert(name.clone());
    name
}

pub(super) fn generate_dsl_file(
    config: &JavaConfig,
    model: &ModelIr,
    ir: &SchemaIr,
    enums: &BTreeMap<String, EnumIr>,
) -> Result<String> {
    let dsl_name = format!("{}Dsl", model.logical_name);
    let pk_fields = model.primary_key.fields();

    let mut imports = BTreeSet::new();
    imports.insert(format!("{}.internal.JsonSupport", config.root_package));
    imports.insert(format!("{}.internal.WireSerializable", config.root_package));
    imports.insert("com.fasterxml.jackson.databind.JsonNode".to_string());
    imports.insert("com.fasterxml.jackson.databind.node.ArrayNode".to_string());
    imports.insert("com.fasterxml.jackson.databind.node.ObjectNode".to_string());
    imports.insert("java.util.List".to_string());
    imports.insert("java.util.function.Consumer".to_string());

    for field in &model.fields {
        let (_, field_imports) = field_to_java_type(
            field,
            &config.root_package,
            &model.logical_name,
            &config.extensions,
        );
        imports.extend(field_imports);
    }

    let view = ModelView::new(model, ir, &config.extensions);

    let mut scalar_fields: Vec<DslScalarFieldCtx> = Vec::new();
    let mut create_fields: Vec<DslWritableFieldCtx> = Vec::new();
    let mut update_fields: Vec<DslWritableFieldCtx> = Vec::new();
    let mut numeric_field_names: Vec<String> = Vec::new();
    let mut order_by_fields: Vec<DslOrderByFieldCtx> = Vec::new();
    let mut orderable_field_names: Vec<String> = Vec::new();
    let mut all_scalar_field_names: Vec<String> = Vec::new();
    let mut used_order_by_methods: BTreeSet<String> = BTreeSet::new();

    for scalar in &view.scalars {
        let field = scalar.field;
        let (base_type, _) = field_base_type(
            field,
            &config.root_package,
            &model.logical_name,
            &config.extensions,
        );
        let raw_java_type = extension_raw_java_type(field, &config.extensions)
            .filter(|_| !field.is_array)
            .map(|raw| raw.to_string())
            .unwrap_or_default();
        let filter_ops: Vec<DslFilterOpCtx> = filter_operators_for_field(field, enums)
            .into_iter()
            .map(|(suffix, java_type)| DslFilterOpCtx {
                suffix_pascal: suffix.to_upper_camel_case(),
                suffix,
                java_type,
            })
            .collect();

        scalar_fields.push(DslScalarFieldCtx {
            variant_name: field.logical_name.to_upper_camel_case(),
            name: field.logical_name.clone(),
            db_name: field.db_name.clone(),
            java_type: base_type,
            raw_java_type: raw_java_type.clone(),
            filter_ops,
        });

        all_scalar_field_names.push(field.logical_name.clone());

        if scalar.numeric_scalar().is_some() {
            numeric_field_names.push(field.logical_name.clone());
        }
        if scalar.is_orderable() {
            order_by_fields.push(DslOrderByFieldCtx {
                method_name: field.logical_name.clone(),
                wire_name: field.logical_name.clone(),
            });
            used_order_by_methods.insert(field.logical_name.clone());
            orderable_field_names.push(field.logical_name.clone());
        }

        if is_writable_on_create(field) {
            let (ty, _) = field_to_java_type(field, "", &model.logical_name, &config.extensions);
            create_fields.push(DslWritableFieldCtx {
                name: field.logical_name.clone(),
                db_name: field.db_name.clone(),
                java_type: ty,
                raw_java_type: raw_java_type.clone(),
                method_suffix: field.logical_name.to_upper_camel_case(),
                accepts_arithmetic: false,
            });
        }

        if is_writable_on_update(field, &pk_fields) {
            let (ty, _) = field_to_java_type(field, "", &model.logical_name, &config.extensions);
            update_fields.push(DslWritableFieldCtx {
                name: field.logical_name.clone(),
                db_name: field.db_name.clone(),
                java_type: ty,
                raw_java_type: raw_java_type.clone(),
                method_suffix: field.logical_name.to_upper_camel_case(),
                accepts_arithmetic: scalar.accepts_arithmetic(),
            });
        }
    }

    for dotted in &view.dotted_order_by {
        order_by_fields.push(DslOrderByFieldCtx {
            method_name: unique_order_by_method(
                &format!("{}_{}", dotted.parent, dotted.child).to_lower_camel_case(),
                &mut used_order_by_methods,
            ),
            wire_name: dotted.path(),
        });
    }

    let relation_fields: Vec<DslRelationFieldCtx> = view
        .resolved_relations()
        .map(|(relation, _)| DslRelationFieldCtx {
            name: relation.logical_name().to_string(),
            target_model: relation.target_model_name().to_string(),
        })
        .collect();

    let nested_writes: Vec<DslNestedWriteCtx> = view
        .resolved_relations()
        .map(|(relation, target)| {
            let relation_pascal = relation.logical_name().to_upper_camel_case();
            DslNestedWriteCtx {
                method_name: relation.logical_name().to_lower_camel_case(),
                wire_name: relation.logical_name().to_string(),
                target_model: target.logical_name.clone(),
                target_dsl: format!("{}Dsl", target.logical_name),
                create_nested_name: format!("{relation_pascal}CreateNested"),
                update_nested_name: format!("{relation_pascal}UpdateNested"),
                is_owning: relation.is_owning(),
            }
        })
        .collect();

    let mut context = Context::from_serialize(&DslTemplateContext {
        package_name: config.root_package.clone(),
        imports: imports.into_iter().collect(),
        name: dsl_name,
        scalar_fields,
        create_fields,
        update_fields,
        relation_fields,
        nested_writes,
        numeric_field_names,
        order_by_fields,
        orderable_field_names,
        vector_field_names: view.vector_field_names,
        all_scalar_field_names,
    })
    .expect("Java DSL context should serialize");
    for (flag, value) in config.extensions.template_flags() {
        context.insert(&flag, &value);
    }
    render("java_dsl.tera", &context)
}
