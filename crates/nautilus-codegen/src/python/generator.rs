//! Python code generator for Nautilus models, delegates, and builders.

use heck::{ToPascalCase, ToSnakeCase};
use nautilus_schema::ir::{CompositeTypeIr, EnumIr, ModelIr, ResolvedFieldType, SchemaIr};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tera::{Context, Tera};

use crate::backend::LanguageBackend;
use crate::extension_types::{
    python_input_type_for_extension, ExtensionRegistry, ExtensionType, ExtensionWireKind,
};
use crate::python::backend::PythonBackend;
use crate::python::type_mapper::{
    get_base_python_type, get_default_value, get_filter_operators_for_field, is_auto_generated,
};
use crate::type_helpers::{is_orderable_composite_field, is_orderable_model_field};

/// Python template registry — loaded once at first use.
pub static PYTHON_TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "composite_types.py.tera",
            include_str!("../../templates/python/composite_types.py.tera"),
        ),
        (
            "model_file.py.tera",
            include_str!("../../templates/python/model_file.py.tera"),
        ),
        (
            "input_types.py.tera",
            include_str!("../../templates/python/input_types.py.tera"),
        ),
        (
            "enums.py.tera",
            include_str!("../../templates/python/enums.py.tera"),
        ),
        (
            "client.py.tera",
            include_str!("../../templates/python/client.py.tera"),
        ),
        (
            "package_init.py.tera",
            include_str!("../../templates/python/package_init.py.tera"),
        ),
        (
            "models_init.py.tera",
            include_str!("../../templates/python/models_init.py.tera"),
        ),
        (
            "enums_init.py.tera",
            include_str!("../../templates/python/enums_init.py.tera"),
        ),
        (
            "errors_init.py.tera",
            include_str!("../../templates/python/errors_init.py.tera"),
        ),
        (
            "internal_init.py.tera",
            include_str!("../../templates/python/internal_init.py.tera"),
        ),
        (
            "transaction_init.py.tera",
            include_str!("../../templates/python/transaction_init.py.tera"),
        ),
        (
            "events.py.tera",
            include_str!("../../templates/python/events.py.tera"),
        ),
    ])
    .expect("embedded Python templates must parse");
    tera
});

fn render(template: &str, ctx: &Context) -> String {
    crate::template::render(&PYTHON_TEMPLATES, template, ctx)
}

/// Template context for a single model field in the Python codegen backend.
///
/// This struct is intentionally separate from `FieldContext` in
/// `generator.rs`: Python needs additional template variables
/// (`logical_name`, `python_type`, `base_type`, `is_enum`, `has_default`,
/// `default`) that have no counterpart in the Rust backend, and the two are
/// expected to evolve independently.
#[derive(Debug, Clone, Serialize)]
struct PythonFieldContext {
    name: String,
    logical_name: String,
    db_name: String,
    python_type: String,
    input_python_type: String,
    model_python_type: String,
    base_type: String,
    raw_base_type: String,
    extension_coercer: String,
    extension_input_serializer: String,
    is_optional: bool,
    is_array: bool,
    is_enum: bool,
    has_default: bool,
    default: String,
    model_has_default: bool,
    model_default: String,
    is_pk: bool,
    doc_comment: String,
    index: usize,
}

#[derive(Debug, Clone, Serialize)]
struct PythonRelationContext {
    field_name: String,
    target_model: String,
    target_table: String,
    is_array: bool,
    fields: Vec<String>,
    references: Vec<String>,
    fields_db: Vec<String>,
    references_db: Vec<String>,
}

fn resolve_inverse_relation_fields(
    source_model_name: &str,
    relation_name: Option<&str>,
    target_model: &ModelIr,
) -> (Vec<String>, Vec<String>) {
    let inverse = target_model.relation_fields().find(|field| {
        if let ResolvedFieldType::Relation(inv_rel) = &field.field_type {
            if inv_rel.target_model != source_model_name {
                return false;
            }

            match (relation_name, inv_rel.name.as_deref()) {
                (Some(expected), Some(actual)) => actual == expected,
                (Some(_), None) => false,
                (None, Some(_)) => false,
                (None, None) => true,
            }
        } else {
            false
        }
    });

    if let Some(inverse_field) = inverse {
        if let ResolvedFieldType::Relation(inv_rel) = &inverse_field.field_type {
            return (inv_rel.references.clone(), inv_rel.fields.clone());
        }
    }

    (vec![], vec![])
}

#[derive(Debug, Clone, Serialize)]
struct FilterOperatorContext {
    suffix: String,
    python_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct WhereInputFieldContext {
    name: String,
    python_type: String,
    where_python_type: String,
    is_nullable: bool,
    is_vector: bool,
    operators: Vec<FilterOperatorContext>,
}

#[derive(Debug, Clone, Serialize)]
struct CreateInputFieldContext {
    name: String,
    python_type: String,
    is_required: bool,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateInputFieldContext {
    name: String,
    python_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct OrderByFieldContext {
    name: String,
    is_dotted: bool,
}

#[derive(Debug, Clone, Serialize)]
struct IncludeFieldContext {
    name: String,
    logical_name: String,
    target_model: String,
    /// snake_case module name of the target model (e.g. "post" for Post)
    target_snake: String,
    /// true if this is a one-to-many relation (List/array)
    is_array: bool,
}

/// Context for a scalar field used in aggregate input types (avg/sum/min/max).
#[derive(Debug, Clone, Serialize)]
struct AggregateFieldContext {
    name: String,
    python_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct ExtensionImportContext {
    module: String,
    symbols: Vec<String>,
    types: Vec<String>,
    input_types: Vec<String>,
}

fn output_base_python_type(
    field: &nautilus_schema::ir::FieldIr,
    enums: &HashMap<String, EnumIr>,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return ty.type_name.to_string();
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => {
            crate::python::type_mapper::scalar_to_python_type(scalar).to_string()
        }
        ResolvedFieldType::Enum { enum_name } => {
            if enums.contains_key(enum_name) {
                enum_name.clone()
            } else {
                "str".to_string()
            }
        }
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => rel.target_model.clone(),
    }
}

fn exact_output_python_type(field: &nautilus_schema::ir::FieldIr, base_type: String) -> String {
    if field.is_array {
        format!("List[{}]", base_type)
    } else if !field.is_required {
        format!("Optional[{}]", base_type)
    } else {
        base_type
    }
}

fn exact_input_python_type(field: &nautilus_schema::ir::FieldIr, base_type: String) -> String {
    if field.is_array {
        format!("List[{}]", base_type)
    } else if !field.is_required {
        format!("Optional[{}]", base_type)
    } else {
        base_type
    }
}

fn add_none_to_python_union(type_expr: String) -> String {
    let trimmed = type_expr.trim();
    if let Some(inner) = trimmed
        .strip_prefix("Union[")
        .and_then(|value| value.strip_suffix(']'))
    {
        format!("Union[{inner}, None]")
    } else {
        format!("Optional[{trimmed}]")
    }
}

fn input_base_python_type(
    field: &nautilus_schema::ir::FieldIr,
    enums: &HashMap<String, EnumIr>,
    extensions: &ExtensionRegistry,
) -> String {
    if let Some(ty) = extensions.type_for_field(field) {
        return python_input_type_for_extension(ty);
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => {
            crate::python::type_mapper::scalar_to_python_type(scalar).to_string()
        }
        ResolvedFieldType::Enum { enum_name } => {
            if enums.contains_key(enum_name) {
                enum_name.clone()
            } else {
                "str".to_string()
            }
        }
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        ResolvedFieldType::Relation(rel) => rel.target_model.clone(),
    }
}

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
) -> (String, String) {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_python_model_with_registry(model, ir, is_async, recursive_type_depth, &extensions)
}

fn generate_python_model_with_registry(
    model: &ModelIr,
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
    extensions: &ExtensionRegistry,
) -> (String, String) {
    let mut context = Context::new();
    crate::template::insert_protocol_version(&mut context);
    insert_derived_names(&mut context, model);

    let pk_field_names = model.primary_key.fields();
    context.insert("primary_key_fields", &pk_field_names);

    let mut fields = build_scalar_fields(model, ir, extensions, &pk_field_names);
    fields.order_by.extend(composite_order_by_fields(model, ir));

    let relation_imports: Vec<String> = model
        .relation_fields()
        .filter_map(|field| match &field.field_type {
            ResolvedFieldType::Relation(rel) => Some(rel.target_model.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    context.insert("has_datetime", &fields.has_datetime);
    context.insert("has_uuid", &fields.has_uuid);
    context.insert("has_decimal", &fields.has_decimal);
    context.insert("has_dict", &fields.has_dict);
    for (flag, value) in extensions.template_flags() {
        context.insert(&flag, &value);
    }
    context.insert("has_enums", &!fields.enum_imports.is_empty());
    context.insert("enum_imports", &fields.enum_imports);
    context.insert(
        "has_composite_types",
        &!fields.composite_type_imports.is_empty(),
    );
    context.insert("composite_type_imports", &fields.composite_type_imports);

    let extension_import_contexts = build_extension_imports(fields.extension_imports);
    context.insert(
        "has_extension_types",
        &!extension_import_contexts.is_empty(),
    );
    context.insert("extension_imports", &extension_import_contexts);
    context.insert("has_relations", &!relation_imports.is_empty());
    context.insert("relation_imports", &relation_imports);

    let relation_fields: Vec<PythonFieldContext> = model
        .relation_fields()
        .enumerate()
        .map(|(idx, field)| relation_field_context(model, field, idx, ir, extensions))
        .collect();
    let include_fields = build_include_fields(model);

    context.insert("needs_typeddict", &true);
    context.insert("where_input_fields", &fields.where_input);
    context.insert("create_input_fields", &fields.create_input);
    context.insert("update_input_fields", &fields.update_input);
    context.insert("order_by_fields", &fields.order_by);
    context.insert(
        "has_dotted_order_by_fields",
        &fields.order_by.iter().any(|field| field.is_dotted),
    );
    context.insert("include_fields", &include_fields);
    context.insert("has_includes", &!include_fields.is_empty());
    context.insert("numeric_fields", &fields.numeric);
    context.insert("orderable_fields", &fields.orderable);
    context.insert("object_value_db_fields", &fields.object_value_db_names);
    context.insert("has_numeric_fields", &!fields.numeric.is_empty());
    context.insert("has_orderable_fields", &!fields.orderable.is_empty());
    context.insert("has_vector_fields", &!fields.vector_names.is_empty());
    context.insert("vector_field_names", &fields.vector_names);

    context.insert("scalar_fields", &fields.scalar);
    context.insert("relation_fields", &relation_fields);
    context.insert("create_fields", &fields.create);
    context.insert("relations", &build_relations(model, ir));
    context.insert("is_async", &is_async);
    context.insert("recursive_type_depth", &recursive_type_depth);

    let model_code = render("model_file.py.tera", &context);

    (
        format!("{}.py", model.logical_name.to_snake_case()),
        model_code,
    )
}

/// Insert the `{Model}Delegate` / `{Model}FindMany` / … class names the
/// templates refer to.
fn insert_derived_names(context: &mut Context, model: &ModelIr) {
    let name = &model.logical_name;
    context.insert("model_name", name);
    context.insert("snake_name", &name.to_snake_case());
    context.insert("table_name", &model.db_name);
    context.insert("delegate_name", &format!("{}Delegate", name));
    context.insert("find_many_name", &format!("{}FindMany", name));
    context.insert("create_name", &format!("{}Create", name));
    context.insert("create_many_name", &format!("{}CreateMany", name));
    context.insert("update_name", &format!("{}Update", name));
    context.insert("delete_name", &format!("{}Delete", name));
}

/// The per-field template contexts and import flags a model needs, collected in
/// a single pass over its scalar fields.
#[derive(Default)]
struct PythonFieldSets {
    scalar: Vec<PythonFieldContext>,
    create: Vec<PythonFieldContext>,
    where_input: Vec<WhereInputFieldContext>,
    create_input: Vec<CreateInputFieldContext>,
    update_input: Vec<UpdateInputFieldContext>,
    order_by: Vec<OrderByFieldContext>,
    numeric: Vec<AggregateFieldContext>,
    orderable: Vec<AggregateFieldContext>,
    object_value_db_names: Vec<String>,
    vector_names: Vec<String>,
    enum_imports: Vec<String>,
    composite_type_imports: Vec<String>,
    extension_imports: BTreeMap<String, BTreeSet<String>>,
    has_datetime: bool,
    has_uuid: bool,
    has_decimal: bool,
    has_dict: bool,
}

fn build_scalar_fields(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
    pk_field_names: &[&str],
) -> PythonFieldSets {
    use nautilus_schema::ir::ScalarType;

    let mut sets = PythonFieldSets::default();
    let mut enum_imports = HashSet::new();
    let mut composite_type_imports = HashSet::new();

    for (idx, field) in model.scalar_fields().enumerate() {
        match &field.field_type {
            ResolvedFieldType::Enum { enum_name } if ir.enums.contains_key(enum_name) => {
                enum_imports.insert(enum_name.clone());
            }
            ResolvedFieldType::CompositeType { type_name, .. }
                if ir.composite_types.contains_key(type_name) =>
            {
                composite_type_imports.insert(type_name.clone());
            }
            ResolvedFieldType::Scalar(scalar) => match scalar {
                ScalarType::DateTime => sets.has_datetime = true,
                ScalarType::Uuid => sets.has_uuid = true,
                ScalarType::Decimal { .. } => sets.has_decimal = true,
                ScalarType::Json | ScalarType::Jsonb | ScalarType::Hstore => sets.has_dict = true,
                _ => {}
            },
            _ => {}
        }

        let extension_type = extensions.type_for_field(field);
        if let Some(ty) = extension_type {
            sets.extension_imports
                .entry(ty.extension.to_string())
                .or_default()
                .insert(ty.type_name.to_string());
            if ty.wire_kind == ExtensionWireKind::Hstore {
                sets.has_dict = true;
            }
        }

        let input_python_type =
            exact_input_python_type(field, input_base_python_type(field, &ir.enums, extensions));
        let base_python_type = get_base_python_type(field, &ir.enums);
        let is_pk = pk_field_names.contains(&field.logical_name.as_str());

        let field_ctx = scalar_field_context(model, field, idx, is_pk, ir, extensions);
        sets.create.push(field_ctx.clone());
        sets.scalar.push(field_ctx);

        if field.is_vector() {
            sets.vector_names.push(field.logical_name.clone());
        }
        sets.where_input.push(where_input_field(
            field,
            ir,
            extension_type,
            &base_python_type,
        ));

        if matches!(
            field.field_type,
            ResolvedFieldType::Scalar(ScalarType::Json | ScalarType::Jsonb | ScalarType::Hstore)
        ) && !field.is_array
        {
            sets.object_value_db_names.push(field.db_name.clone());
        }

        sets.create_input.push(CreateInputFieldContext {
            name: field.logical_name.clone(),
            python_type: input_python_type.clone(),
            is_required: field.is_required
                && field.default_value.is_none()
                && !field.is_updated_at
                && field.computed.is_none(),
        });

        // An auto-generated primary key is never writable, so it stays out of
        // the update input.
        let is_auto_pk = is_auto_generated(field) && is_pk;
        if !is_auto_pk {
            sets.update_input.push(UpdateInputFieldContext {
                name: field.logical_name.clone(),
                python_type: input_python_type,
            });
        }

        let is_numeric = matches!(
            &field.field_type,
            ResolvedFieldType::Scalar(
                ScalarType::Int
                    | ScalarType::BigInt
                    | ScalarType::Float
                    | ScalarType::Decimal { .. }
            )
        );
        if is_numeric {
            sets.numeric.push(AggregateFieldContext {
                name: field.logical_name.clone(),
                python_type: base_python_type.clone(),
            });
        }

        if is_orderable_model_field(field) {
            sets.order_by.push(OrderByFieldContext {
                name: field.logical_name.clone(),
                is_dotted: false,
            });
            sets.orderable.push(AggregateFieldContext {
                name: field.logical_name.clone(),
                python_type: base_python_type,
            });
        }
    }

    sets.enum_imports = enum_imports.into_iter().collect();
    sets.composite_type_imports = composite_type_imports.into_iter().collect();
    sets
}

fn scalar_field_context(
    model: &ModelIr,
    field: &nautilus_schema::ir::FieldIr,
    index: usize,
    is_pk: bool,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> PythonFieldContext {
    let extension_type = extensions.type_for_field(field);
    let output_base_type = output_base_python_type(field, &ir.enums, extensions);
    let python_type = exact_output_python_type(field, output_base_type.clone());
    let input_python_type =
        exact_input_python_type(field, input_base_python_type(field, &ir.enums, extensions));
    let raw_base_type = match &field.field_type {
        ResolvedFieldType::Scalar(s) => {
            crate::python::type_mapper::scalar_to_python_type(s).to_string()
        }
        ResolvedFieldType::Enum { enum_name } => enum_name.clone(),
        _ => "Any".to_string(),
    };

    // Render enum defaults as `EnumName.VARIANT`.
    let mut default_val = get_default_value(field);
    if let Some(ref def) = default_val {
        if let ResolvedFieldType::Enum { enum_name } = &field.field_type {
            if !def.contains('.') && !def.contains('(') && def != "None" {
                default_val = Some(format!("{}.{}", enum_name, def));
            }
        }
    }

    let (model_has_default, model_default) = if field.is_array {
        (true, "Field(default_factory=list)".to_string())
    } else if !field.is_required {
        (true, "None".to_string())
    } else {
        (false, String::new())
    };

    PythonFieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        model_python_type: python_type.clone(),
        python_type,
        input_python_type,
        base_type: output_base_type,
        raw_base_type,
        extension_coercer: extension_wire_adapter(field, extension_type, "from_wire"),
        extension_input_serializer: extension_wire_adapter(field, extension_type, "to_wire_input"),
        is_optional: !field.is_required,
        is_array: field.is_array,
        is_enum: matches!(field.field_type, ResolvedFieldType::Enum { .. }),
        model_has_default,
        model_default,
        is_pk,
        doc_comment: crate::schema_docs::field_modifier_doc(model, field),
        has_default: default_val.is_some(),
        default: default_val.unwrap_or_default(),
        index,
    }
}

/// The Python expression that converts a field between its wire form and its
/// extension type, mapping over the elements of an array field.
fn extension_wire_adapter(
    field: &nautilus_schema::ir::FieldIr,
    extension_type: Option<ExtensionType>,
    method: &str,
) -> String {
    extension_type
        .map(|ty| {
            if field.is_array {
                format!(
                    "lambda v: [{}.{}(item) for item in v] if isinstance(v, list) else v",
                    ty.type_name, method
                )
            } else {
                format!("{}.{}", ty.type_name, method)
            }
        })
        .unwrap_or_default()
}

fn where_input_field(
    field: &nautilus_schema::ir::FieldIr,
    ir: &SchemaIr,
    extension_type: Option<ExtensionType>,
    base_python_type: &str,
) -> WhereInputFieldContext {
    let is_nullable = !field.is_required && !field.is_array;
    WhereInputFieldContext {
        name: field.logical_name.clone(),
        python_type: base_python_type.to_string(),
        where_python_type: extension_type
            .map(|ty| {
                let type_expr = ty.python_filter_input();
                if is_nullable {
                    add_none_to_python_union(type_expr)
                } else {
                    type_expr
                }
            })
            .unwrap_or_default(),
        is_nullable,
        is_vector: field.is_vector(),
        operators: get_filter_operators_for_field(field, &ir.enums)
            .into_iter()
            .map(|op| FilterOperatorContext {
                suffix: op.suffix,
                python_type: op.type_name,
            })
            .collect(),
    }
}

/// Dotted `parent.child` order-by entries for the orderable fields of every
/// non-array composite type column.
fn composite_order_by_fields(model: &ModelIr, ir: &SchemaIr) -> Vec<OrderByFieldContext> {
    let mut fields = Vec::new();
    for parent in model.scalar_fields().filter(|field| !field.is_array) {
        let ResolvedFieldType::CompositeType { type_name, .. } = &parent.field_type else {
            continue;
        };
        let Some(composite) = ir.composite_types.get(type_name) else {
            continue;
        };
        for nested in &composite.fields {
            if is_orderable_composite_field(nested) {
                fields.push(OrderByFieldContext {
                    name: format!("{}.{}", parent.logical_name, nested.logical_name),
                    is_dotted: true,
                });
            }
        }
    }
    fields
}

fn build_extension_imports(
    extension_imports: BTreeMap<String, BTreeSet<String>>,
) -> Vec<ExtensionImportContext> {
    extension_imports
        .into_iter()
        .map(|(module, types)| {
            let types: Vec<String> = types.into_iter().collect();
            let input_types: Vec<String> =
                types.iter().map(|name| format!("{name}Input")).collect();
            let mut symbols = types.clone();
            symbols.extend(input_types.clone());
            ExtensionImportContext {
                module,
                symbols,
                types,
                input_types,
            }
        })
        .collect()
}

/// Relation fields are hydrated separately, so they carry no column metadata
/// and always default to empty.
fn relation_field_context(
    model: &ModelIr,
    field: &nautilus_schema::ir::FieldIr,
    index: usize,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> PythonFieldContext {
    let python_type =
        PythonBackend.wrap_field_type(field, output_base_python_type(field, &ir.enums, extensions));
    let default_val = if field.is_array {
        "Field(default_factory=list)".to_string()
    } else {
        "None".to_string()
    };

    PythonFieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        input_python_type: python_type.clone(),
        model_python_type: python_type.clone(),
        python_type,
        base_type: String::new(),
        raw_base_type: String::new(),
        extension_coercer: String::new(),
        extension_input_serializer: String::new(),
        is_optional: true,
        is_array: field.is_array,
        is_enum: false,
        has_default: true,
        default: default_val,
        model_has_default: true,
        model_default: "None".to_string(),
        is_pk: false,
        doc_comment: crate::schema_docs::field_modifier_doc(model, field),
        index,
    }
}

fn build_relations(model: &ModelIr, ir: &SchemaIr) -> Vec<PythonRelationContext> {
    model
        .relation_fields()
        .filter_map(|field| {
            let ResolvedFieldType::Relation(rel) = &field.field_type else {
                return None;
            };
            let target_model = ir.models.get(&rel.target_model)?;
            let (fields, references) = if rel.fields.is_empty() {
                resolve_inverse_relation_fields(
                    &model.logical_name,
                    rel.name.as_deref(),
                    target_model,
                )
            } else {
                (rel.fields.clone(), rel.references.clone())
            };

            Some(PythonRelationContext {
                field_name: field.logical_name.to_snake_case(),
                target_model: rel.target_model.clone(),
                target_table: target_model.db_name.clone(),
                is_array: field.is_array,
                fields_db: db_names_for(model, &fields),
                references_db: db_names_for(target_model, &references),
                fields,
                references,
            })
        })
        .collect()
}

fn db_names_for(model: &ModelIr, logical_names: &[String]) -> Vec<String> {
    logical_names
        .iter()
        .filter_map(|logical_name| {
            model
                .fields
                .iter()
                .find(|f| &f.logical_name == logical_name)
                .map(|f| f.db_name.clone())
        })
        .collect()
}

fn build_include_fields(model: &ModelIr) -> Vec<IncludeFieldContext> {
    model
        .relation_fields()
        .filter_map(|field| {
            let ResolvedFieldType::Relation(rel) = &field.field_type else {
                return None;
            };
            Some(IncludeFieldContext {
                name: field.logical_name.to_snake_case(),
                logical_name: field.logical_name.clone(),
                target_model: rel.target_model.clone(),
                target_snake: rel.target_model.to_snake_case(),
                is_array: field.is_array,
            })
        })
        .collect()
}

/// Generate all Python models.
///
/// `is_async` is forwarded to every [`generate_python_model`] call.
/// `recursive_type_depth` controls the depth of generated recursive include TypedDicts.
pub fn generate_all_python_models(
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
) -> Vec<(String, String)> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_all_python_models_with_registry(ir, is_async, recursive_type_depth, &extensions)
}

pub(crate) fn generate_all_python_models_with_registry(
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
    extensions: &ExtensionRegistry,
) -> Vec<(String, String)> {
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

/// Generate `types/types.py` — TypedDict declarations for all composite types.
///
/// Returns `None` when there are no composite types.
pub fn generate_python_composite_types(
    composite_types: &HashMap<String, CompositeTypeIr>,
) -> Option<String> {
    if composite_types.is_empty() {
        return None;
    }

    #[derive(Serialize)]
    struct CompositeFieldCtx {
        name: String,
        python_type: String,
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
                        ResolvedFieldType::Scalar(s) => {
                            crate::python::type_mapper::scalar_to_python_type(s).to_string()
                        }
                        ResolvedFieldType::Enum { enum_name } => enum_name.clone(),
                        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
                        ResolvedFieldType::Relation(_) => "Any".to_string(),
                    };
                    let python_type = if f.is_array {
                        format!("List[{}]", base)
                    } else if !f.is_required {
                        format!("Optional[{}]", base)
                    } else {
                        base
                    };
                    CompositeFieldCtx {
                        name: f.logical_name.to_snake_case(),
                        python_type,
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

    Some(render("composite_types.py.tera", &context))
}

/// Generate Python enums file.
pub fn generate_python_enums(enums: &HashMap<String, EnumIr>) -> String {
    let mut context = Context::new();

    #[derive(Serialize)]
    struct EnumContext {
        name: String,
        variants: Vec<String>,
    }

    let enum_contexts: Vec<EnumContext> = enums
        .values()
        .map(|e| EnumContext {
            name: e.logical_name.clone(),
            variants: e.variants.clone(),
        })
        .collect();

    context.insert("enums", &enum_contexts);

    render("enums.py.tera", &context)
}

/// Generate Python client file with model delegates.
///
/// `is_async` determines whether the generated `Nautilus` class exposes an async
/// context manager (`async with Nautilus(...) as db`) or a sync one (`with Nautilus(...) as db`).
pub fn generate_python_client(
    models: &HashMap<String, ModelIr>,
    schema_path: &str,
    is_async: bool,
) -> String {
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
pub fn generate_package_init(has_enums: bool) -> String {
    let mut context = Context::new();
    context.insert("has_enums", &has_enums);

    render("package_init.py.tera", &context)
}

/// Generate models/__init__.py
pub fn generate_models_init(models: &[(String, String)]) -> String {
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
pub fn generate_enums_init(has_enums: bool) -> String {
    let mut context = Context::new();
    context.insert("has_enums", &has_enums);

    render("enums_init.py.tera", &context)
}

/// Generate errors/__init__.py.
///
/// Content is static (no template variables needed).
pub fn generate_errors_init() -> &'static str {
    include_str!("../../templates/python/errors_init.py.tera")
}

/// Generate _internal/__init__.py.
///
/// Content is static (no template variables needed).
pub fn generate_internal_init() -> &'static str {
    include_str!("../../templates/python/internal_init.py.tera")
}

/// Generate transaction.py at the package root.
///
/// Content is static: re-exports `IsolationLevel` and `TransactionClient`
/// from the internal `_internal.transaction` module so users can write
/// `from nautilus.transaction import IsolationLevel`.
pub fn generate_transaction_init() -> &'static str {
    include_str!("../../templates/python/transaction_init.py.tera")
}

/// Generate events.py at the package root.
pub fn generate_events_init() -> &'static str {
    include_str!("../../templates/python/events.py.tera")
}

/// Returns static runtime Python files to be written alongside generated code.
/// These files implement the base client, engine process manager, protocol, and errors.
pub fn python_runtime_files() -> Vec<(String, String)> {
    let protocol_version = nautilus_protocol::PROTOCOL_VERSION.to_string();
    vec![
        (
            "_errors.py".to_string(),
            include_str!("../../templates/python/runtime/_errors.py").to_string(),
        ),
        (
            "_protocol.py".to_string(),
            include_str!("../../templates/python/runtime/_protocol.py")
                .replace("{{ protocol_version }}", &protocol_version),
        ),
        (
            "_engine.py".to_string(),
            include_str!("../../templates/python/runtime/_engine.py").to_string(),
        ),
        (
            "_client.py".to_string(),
            include_str!("../../templates/python/runtime/_client.py").to_string(),
        ),
        (
            "_descriptors.py".to_string(),
            include_str!("../../templates/python/runtime/_descriptors.py").to_string(),
        ),
        (
            "_transaction.py".to_string(),
            include_str!("../../templates/python/runtime/_transaction.py").to_string(),
        ),
        (
            "_events.py".to_string(),
            include_str!("../../templates/python/runtime/_events.py").to_string(),
        ),
    ]
}
