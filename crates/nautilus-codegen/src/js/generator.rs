//! JavaScript/TypeScript code generator for Nautilus models, delegates, and input types.

use anyhow::{Context as _, Result};
use heck::{ToLowerCamelCase, ToSnakeCase};
use nautilus_schema::ir::{
    CompositeTypeIr, EnumIr, FieldIr, ModelIr, ResolvedFieldType, ScalarType, SchemaIr,
};
use serde::Serialize;
use std::collections::HashMap;
use tera::{Context, Tera};

use crate::extension_types::{ts_input_type_for_extension, ExtensionRegistry, ExtensionType};
use crate::js::type_mapper::{
    get_base_ts_type, get_filter_operators_for_field, get_ts_default_value, is_auto_generated,
    scalar_to_ts_type,
};
use crate::model_view::ModelView;

/// JS/TS template registry — loaded once at first use.
pub static JS_TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "model.js.tera",
            include_str!("../../templates/js/model.js.tera"),
        ),
        (
            "model.d.ts.tera",
            include_str!("../../templates/js/model.d.ts.tera"),
        ),
        (
            "enums.js.tera",
            include_str!("../../templates/js/enums.js.tera"),
        ),
        (
            "enums.d.ts.tera",
            include_str!("../../templates/js/enums.d.ts.tera"),
        ),
        (
            "client.js.tera",
            include_str!("../../templates/js/client.js.tera"),
        ),
        (
            "client.d.ts.tera",
            include_str!("../../templates/js/client.d.ts.tera"),
        ),
        (
            "models_index.js.tera",
            include_str!("../../templates/js/models_index.js.tera"),
        ),
        (
            "models_index.d.ts.tera",
            include_str!("../../templates/js/models_index.d.ts.tera"),
        ),
        (
            "composite_types.d.ts.tera",
            include_str!("../../templates/js/composite_types.d.ts.tera"),
        ),
    ])
    .expect("embedded JS templates must parse");
    tera
});

fn render(template: &str, ctx: &Context) -> Result<String> {
    crate::template::render(&JS_TEMPLATES, template, ctx)
}

#[derive(Debug, Clone, Serialize)]
struct JsFieldContext {
    /// Logical JS field name (camelCase, same as schema logical name).
    name: String,
    /// Logical name from the schema IR (may differ from `name` after `@map`).
    logical_name: String,
    /// Database column name.
    db_name: String,
    /// Full TypeScript type, e.g. `string | null`, `number[]`.
    ts_type: String,
    input_ts_type: String,
    /// Inner base type without wrappers, e.g. `string`, `number`, `Date`.
    base_type: String,
    raw_base_type: String,
    extension_coercer: String,
    extension_input_serializer: String,
    is_optional: bool,
    is_array: bool,
    is_enum: bool,
    has_default: bool,
    default: String,
    is_pk: bool,
    doc_comment: String,
    index: usize,
}

#[derive(Debug, Clone, Serialize)]
struct JsFilterOperatorContext {
    suffix: String,
    ts_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct JsWhereInputFieldContext {
    name: String,
    /// Base TS type used by the template to pick the right filter interface.
    base_type: String,
    ts_type: String,
    where_ts_type: String,
    is_nullable: bool,
    is_vector: bool,
    operators: Vec<JsFilterOperatorContext>,
}

#[derive(Debug, Clone, Serialize)]
struct JsCreateInputFieldContext {
    name: String,
    ts_type: String,
    is_required: bool,
}

#[derive(Debug, Clone, Serialize)]
struct JsUpdateInputFieldContext {
    name: String,
    ts_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct JsOrderByFieldContext {
    name: String,
    is_dotted: bool,
}

#[derive(Debug, Clone, Serialize)]
struct JsIncludeFieldContext {
    name: String,
    target_model: String,
    target_snake: String,
    /// camelCase — property name on the generated Nautilus class.
    target_camel: String,
    is_array: bool,
}

#[derive(Debug, Clone, Serialize)]
struct JsAggregateFieldContext {
    name: String,
    ts_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct JsExtensionImportContext {
    module: String,
    types: Vec<String>,
    input_types: Vec<String>,
}

fn output_base_ts_type(
    field: &nautilus_schema::ir::FieldIr,
    enums: &HashMap<String, EnumIr>,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return ty.type_name.to_string();
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_ts_type(scalar).to_string(),
        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => {
            if enums.contains_key(&rel.target_model) {
                rel.target_model.clone()
            } else {
                format!("{}Model", rel.target_model)
            }
        }
    }
}

fn input_base_ts_type(
    field: &nautilus_schema::ir::FieldIr,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return ts_input_type_for_extension(ty);
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_ts_type(scalar).to_string(),
        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => format!("{}Model", rel.target_model),
    }
}

fn exact_output_ts_type(field: &nautilus_schema::ir::FieldIr, base_type: String) -> String {
    if field.is_array {
        format!("{}[]", base_type)
    } else if !field.is_required {
        format!("{} | null", base_type)
    } else {
        base_type
    }
}

fn exact_input_ts_type(field: &nautilus_schema::ir::FieldIr, base_type: String) -> String {
    if field.is_array {
        format!("{}[]", base_type)
    } else if !field.is_required {
        format!("{} | null", base_type)
    } else {
        base_type
    }
}

/// Generate JavaScript + declaration code for a single model.
///
/// Returns `((js_filename, js_code), (dts_filename, dts_code))`.
pub fn generate_js_model(
    model: &ModelIr,
    ir: &SchemaIr,
) -> Result<((String, String), (String, String))> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_js_model_with_registry(model, ir, &extensions)
}

fn generate_js_model_with_registry(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Result<((String, String), (String, String))> {
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

/// The per-field template contexts a model needs, collected in a single pass
/// over the shared [`ModelView`].
#[derive(Default)]
struct JsFieldSets {
    scalar: Vec<JsFieldContext>,
    where_input: Vec<JsWhereInputFieldContext>,
    create_input: Vec<JsCreateInputFieldContext>,
    update_input: Vec<JsUpdateInputFieldContext>,
    order_by: Vec<JsOrderByFieldContext>,
    numeric: Vec<JsAggregateFieldContext>,
    orderable: Vec<JsAggregateFieldContext>,
}

fn build_scalar_fields(
    view: &ModelView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> JsFieldSets {
    let mut sets = JsFieldSets::default();

    for scalar in &view.scalars {
        let field = scalar.field;
        let extension_type = scalar.extension_type;

        let base_type = output_base_ts_type(field, &ir.enums, extensions);
        let ts_type = exact_output_ts_type(field, base_type.clone());
        let input_ts_type = exact_input_ts_type(field, input_base_ts_type(field, extensions));
        let raw_base_type = get_base_ts_type(field, &ir.enums);
        let auto_generated = is_auto_generated(field);
        let default_val = get_ts_default_value(field);

        sets.scalar.push(JsFieldContext {
            name: field.logical_name.clone(),
            logical_name: field.logical_name.clone(),
            db_name: field.db_name.clone(),
            ts_type: ts_type.clone(),
            input_ts_type: input_ts_type.clone(),
            base_type: base_type.clone(),
            raw_base_type: raw_base_type.clone(),
            extension_coercer: extension_wire_adapter(field, extension_type, WireAdapter::From),
            extension_input_serializer: extension_wire_adapter(
                field,
                extension_type,
                WireAdapter::ToInput,
            ),
            is_optional: !field.is_required,
            is_array: field.is_array,
            is_enum: scalar.is_enum(),
            has_default: default_val.is_some(),
            default: default_val.unwrap_or_default(),
            is_pk: scalar.is_pk,
            doc_comment: scalar.doc_comment.clone(),
            index: scalar.index,
        });

        sets.where_input.push(where_input_field(
            field,
            ir,
            extension_type,
            &raw_base_type,
            &ts_type,
        ));

        if !auto_generated {
            sets.create_input.push(JsCreateInputFieldContext {
                name: field.logical_name.clone(),
                ts_type: input_ts_type.clone(),
                is_required: field.is_required
                    && field.default_value.is_none()
                    && !field.is_updated_at,
            });
        }

        // A database-generated integer primary key is never writable, so it
        // stays out of the update input.
        let is_auto_pk = auto_generated
            && scalar.is_pk
            && matches!(
                field.field_type,
                ResolvedFieldType::Scalar(ScalarType::Int | ScalarType::BigInt)
            );
        if !is_auto_pk {
            sets.update_input.push(JsUpdateInputFieldContext {
                name: field.logical_name.clone(),
                ts_type: input_ts_type,
            });
        }

        if let Some(scalar_type) = scalar.numeric_scalar() {
            sets.numeric.push(JsAggregateFieldContext {
                name: field.logical_name.clone(),
                ts_type: scalar_to_ts_type(scalar_type).to_string(),
            });
        }

        if scalar.is_orderable() {
            sets.order_by.push(JsOrderByFieldContext {
                name: field.logical_name.clone(),
                is_dotted: false,
            });
            sets.orderable.push(JsAggregateFieldContext {
                name: field.logical_name.clone(),
                ts_type: base_type,
            });
        }
    }

    sets.order_by.extend(
        view.dotted_order_by
            .iter()
            .map(|dotted| JsOrderByFieldContext {
                name: dotted.path(),
                is_dotted: true,
            }),
    );
    sets
}

enum WireAdapter {
    From,
    ToInput,
}

/// The JavaScript expression that converts a field between its wire form and
/// its extension type, mapping over the elements of an array field.
fn extension_wire_adapter(
    field: &FieldIr,
    extension_type: Option<ExtensionType>,
    adapter: WireAdapter,
) -> String {
    let Some(ty) = extension_type else {
        return String::new();
    };
    let method = match adapter {
        WireAdapter::From => "from",
        WireAdapter::ToInput => "toWireInput",
    };
    if field.is_array {
        format!(
            "(value) => Array.isArray(value) ? value.map(item => {}.{}(item)) : value",
            ty.type_name, method
        )
    } else {
        format!("{}.{}", ty.type_name, method)
    }
}

fn where_input_field(
    field: &FieldIr,
    ir: &SchemaIr,
    extension_type: Option<ExtensionType>,
    raw_base_type: &str,
    ts_type: &str,
) -> JsWhereInputFieldContext {
    let is_nullable = !field.is_required && !field.is_array;
    JsWhereInputFieldContext {
        name: field.logical_name.clone(),
        base_type: raw_base_type.to_string(),
        ts_type: ts_type.to_string(),
        where_ts_type: extension_type
            .map(|ty| {
                let type_expr = ty.ts_filter_input();
                if is_nullable {
                    format!("{type_expr} | null")
                } else {
                    type_expr
                }
            })
            .unwrap_or_default(),
        is_nullable,
        is_vector: field.is_vector(),
        operators: get_filter_operators_for_field(field, &ir.enums)
            .into_iter()
            .map(|op| JsFilterOperatorContext {
                suffix: op.suffix,
                ts_type: op.type_name,
            })
            .collect(),
    }
}

/// Relation fields are hydrated separately, so they carry no column metadata
/// and always default to empty.
fn build_relation_fields(view: &ModelView<'_>) -> Vec<JsFieldContext> {
    view.relations
        .iter()
        .map(|relation| {
            let target = relation.target_model_name();
            let (ts_type, base_type) = if relation.is_array() {
                (format!("{}Model[]", target), format!("{}Model", target))
            } else {
                (
                    format!("{}Model | null", target),
                    format!("{}Model", target),
                )
            };

            JsFieldContext {
                name: relation.logical_name().to_string(),
                logical_name: relation.logical_name().to_string(),
                db_name: relation.field.db_name.clone(),
                input_ts_type: ts_type.clone(),
                ts_type,
                raw_base_type: base_type.clone(),
                base_type,
                extension_coercer: String::new(),
                extension_input_serializer: String::new(),
                is_optional: true,
                is_array: relation.is_array(),
                is_enum: false,
                has_default: true,
                default: if relation.is_array() {
                    "[]".to_string()
                } else {
                    "null".to_string()
                },
                is_pk: false,
                doc_comment: crate::schema_docs::field_modifier_doc(view.model, relation.field),
                index: relation.index,
            }
        })
        .collect()
}

fn build_include_fields(view: &ModelView<'_>) -> Vec<JsIncludeFieldContext> {
    view.relations
        .iter()
        .map(|relation| JsIncludeFieldContext {
            name: relation.logical_name().to_string(),
            target_model: relation.target_model_name().to_string(),
            target_snake: relation.target_model_name().to_snake_case(),
            target_camel: relation.target_model_name().to_lower_camel_case(),
            is_array: relation.is_array(),
        })
        .collect()
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

/// Generate JavaScript + declaration code for all models in the schema.
///
/// Returns `(js_models, dts_models)`, each sorted by filename.
#[allow(clippy::type_complexity)]
pub fn generate_all_js_models(
    ir: &SchemaIr,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_all_js_models_with_registry(ir, &extensions)
}

#[allow(clippy::type_complexity)]
pub(crate) fn generate_all_js_models_with_registry(
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
    let pairs: Vec<((String, String), (String, String))> = ir
        .models
        .values()
        .map(|model| generate_js_model_with_registry(model, ir, extensions))
        .collect::<Result<Vec<_>>>()?;

    let mut js_models: Vec<(String, String)> = pairs.iter().map(|(js, _)| js.clone()).collect();
    let mut dts_models: Vec<(String, String)> = pairs.iter().map(|(_, dts)| dts.clone()).collect();

    js_models.sort_by(|a, b| a.0.cmp(&b.0));
    dts_models.sort_by(|a, b| a.0.cmp(&b.0));

    Ok((js_models, dts_models))
}

/// Generate `types.d.ts` — TypeScript interfaces for all composite types.
///
/// Returns `None` when there are no composite types.
pub fn generate_js_composite_types(
    composite_types: &HashMap<String, CompositeTypeIr>,
) -> Result<Option<String>> {
    if composite_types.is_empty() {
        return Ok(None);
    }

    #[derive(Serialize)]
    struct CompositeFieldCtx {
        name: String,
        ts_type: String,
    }

    #[derive(Serialize)]
    struct CompositeTypeCtx {
        name: String,
        fields: Vec<CompositeFieldCtx>,
    }

    let mut type_list: Vec<CompositeTypeCtx> = composite_types
        .values()
        .map(|ct| {
            let fields = ct
                .fields
                .iter()
                .map(|f| {
                    let base = match &f.field_type {
                        ResolvedFieldType::Scalar(s) => scalar_to_ts_type(s).to_string(),
                        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
                        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
                        ResolvedFieldType::Relation(_) => "unknown".to_string(),
                    };
                    let ts_type = if f.is_array {
                        format!("{}[]", base)
                    } else if !f.is_required {
                        format!("{} | null", base)
                    } else {
                        base
                    };
                    CompositeFieldCtx {
                        name: f.logical_name.clone(),
                        ts_type,
                    }
                })
                .collect();
            CompositeTypeCtx {
                name: ct.logical_name.clone(),
                fields,
            }
        })
        .collect();
    type_list.sort_by(|a, b| a.name.cmp(&b.name));

    let mut context = Context::new();
    context.insert("composite_types", &type_list);

    Ok(Some(render("composite_types.d.ts.tera", &context)?))
}

/// Generate `enums.js` + `enums.d.ts` for all enum definitions.
///
/// Returns `(js_code, dts_code)`.
pub fn generate_js_enums(enums: &HashMap<String, EnumIr>) -> Result<(String, String)> {
    #[derive(Serialize)]
    struct EnumCtx {
        name: String,
        variants: Vec<String>,
    }

    let mut enum_list: Vec<EnumCtx> = enums
        .values()
        .map(|e| EnumCtx {
            name: e.logical_name.clone(),
            variants: e.variants.clone(),
        })
        .collect();
    enum_list.sort_by(|a, b| a.name.cmp(&b.name));

    let mut context = Context::new();
    context.insert("enums", &enum_list);
    let js_code = render("enums.js.tera", &context)?;
    let dts_code = render("enums.d.ts.tera", &context)?;
    Ok((js_code, dts_code))
}

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
pub fn generate_js_models_index(js_models: &[(String, String)]) -> Result<(String, String)> {
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

/// Static JavaScript + declaration runtime files embedded at compile time.
/// Returns `Vec<(filename, content)>` containing both `.js` and `.d.ts` pairs.
pub fn js_runtime_files() -> Vec<(String, String)> {
    let protocol_version = nautilus_protocol::PROTOCOL_VERSION.to_string();
    vec![
        (
            "_errors.js".to_string(),
            include_str!("../../templates/js/runtime/_errors.js").to_string(),
        ),
        (
            "_errors.d.ts".to_string(),
            include_str!("../../templates/js/runtime/_errors.d.ts").to_string(),
        ),
        (
            "_protocol.js".to_string(),
            include_str!("../../templates/js/runtime/_protocol.js")
                .replace("{{ protocol_version }}", &protocol_version),
        ),
        (
            "_protocol.d.ts".to_string(),
            include_str!("../../templates/js/runtime/_protocol.d.ts")
                .replace("{{ protocol_version }}", &protocol_version),
        ),
        (
            "_engine.js".to_string(),
            include_str!("../../templates/js/runtime/_engine.js").to_string(),
        ),
        (
            "_engine.d.ts".to_string(),
            include_str!("../../templates/js/runtime/_engine.d.ts").to_string(),
        ),
        (
            "_client.js".to_string(),
            include_str!("../../templates/js/runtime/_client.js").to_string(),
        ),
        (
            "_client.d.ts".to_string(),
            include_str!("../../templates/js/runtime/_client.d.ts").to_string(),
        ),
        (
            "_transaction.js".to_string(),
            include_str!("../../templates/js/runtime/_transaction.js").to_string(),
        ),
        (
            "_transaction.d.ts".to_string(),
            include_str!("../../templates/js/runtime/_transaction.d.ts").to_string(),
        ),
        (
            "_events.js".to_string(),
            include_str!("../../templates/js/runtime/_events.js").to_string(),
        ),
        (
            "_events.d.ts".to_string(),
            include_str!("../../templates/js/runtime/_events.d.ts").to_string(),
        ),
    ]
}
