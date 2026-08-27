//! Code generator for Nautilus models, delegates, and builders.

use heck::{ToPascalCase, ToSnakeCase};
use nautilus_schema::ast::StorageStrategy;
use nautilus_schema::ir::{
    CompositeFieldIr, FieldIr, ModelIr, ResolvedFieldType, ScalarType, SchemaIr,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tera::{Context, Tera};

use crate::extension_types::ExtensionRegistry;
use crate::type_helpers::{
    field_to_rust_avg_type, field_to_rust_base_type, field_to_rust_sum_type, field_to_rust_type,
    is_orderable_composite_field, is_orderable_model_field, json_path_cast_variant,
    scalar_to_rust_type,
};

pub static TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "columns_struct.tera",
            include_str!("../templates/rust/columns_struct.tera"),
        ),
        (
            "column_impl.tera",
            include_str!("../templates/rust/column_impl.tera"),
        ),
        ("create.tera", include_str!("../templates/rust/create.tera")),
        (
            "create_many.tera",
            include_str!("../templates/rust/create_many.tera"),
        ),
        (
            "delegate.tera",
            include_str!("../templates/rust/delegate.tera"),
        ),
        ("delete.tera", include_str!("../templates/rust/delete.tera")),
        ("enum.tera", include_str!("../templates/rust/enum.tera")),
        (
            "find_many.tera",
            include_str!("../templates/rust/find_many.tera"),
        ),
        (
            "from_row_impl.tera",
            include_str!("../templates/rust/from_row_impl.tera"),
        ),
        (
            "model_file.tera",
            include_str!("../templates/rust/model_file.tera"),
        ),
        ("lib_rs.tera", include_str!("../templates/rust/lib_rs.tera")),
        (
            "model_struct.tera",
            include_str!("../templates/rust/model_struct.tera"),
        ),
        ("update.tera", include_str!("../templates/rust/update.tera")),
        (
            "composite_type.tera",
            include_str!("../templates/rust/composite_type.tera"),
        ),
    ])
    .expect("embedded Rust templates must parse");
    tera
});

fn render(template: &str, ctx: &Context) -> String {
    crate::template::render(&TEMPLATES, template, ctx)
}

/// Template context for a single model field in the Rust codegen backend.
///
/// This struct is intentionally separate from [`PythonFieldContext`] in
/// `python/generator.rs`: the two backends expose different template
/// variables (Rust needs `rust_type` / `column_type`; Python needs
/// `python_type` / `base_type` / `is_enum` / `has_default` / `default`) and
/// are expected to evolve independently.
#[derive(Debug, Clone, Serialize)]
struct FieldContext {
    name: String,
    logical_name: String,
    db_name: String,
    rust_type: String,
    base_rust_type: String,
    column_type: String,
    read_hint_expr: String,
    variant_name: String,
    is_array: bool,
    index: usize,
    is_pk: bool,
    /// `true` when the field maps to an `Option<T>` Rust type
    /// (i.e. the schema field is not required and is not a relation).
    is_optional: bool,
    /// `true` when the field has `@updatedAt` — auto-defaults to `now()` if not provided.
    is_updated_at: bool,
    /// `true` when the field is a `@computed` generated column (read-only from client side).
    is_computed: bool,
    doc_comment: String,
}

#[derive(Debug, Clone, Serialize)]
struct AggregateFieldContext {
    name: String,
    logical_name: String,
    rust_type: String,
    avg_rust_type: String,
    sum_rust_type: String,
    variant_name: String,
}

#[derive(Debug, Clone, Serialize)]
struct NestedOrderByFieldContext {
    method_name: String,
    path: String,
    parent_db_name: String,
    field_db_name: String,
    json_key: String,
    json_cast: String,
    rust_type: String,
}

/// Serialisable (logical_name, db_name) pair for primary-key fields.
/// Used in templates to generate cursor predicate slices.
#[derive(Debug, Clone, Serialize)]
struct PkFieldContext {
    /// Snake-case logical name — used as the cursor map key in generated code.
    name: String,
    /// Original logical field name from the schema.
    logical_name: String,
    /// Database column name — used to build the `table__db_col` column reference.
    db_name: String,
}

#[derive(Debug, Clone, Serialize)]
struct RelationContext {
    field_name: String,
    target_model: String,
    target_table: String,
    is_array: bool,
    fields: Vec<String>,
    references: Vec<String>,
    fields_db: Vec<String>,
    references_db: Vec<String>,
    target_scalar_fields: Vec<FieldContext>,
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

    let Some(inverse_field) = inverse else {
        return (vec![], vec![]);
    };
    let ResolvedFieldType::Relation(inv_rel) = &inverse_field.field_type else {
        return (vec![], vec![]);
    };

    (inv_rel.references.clone(), inv_rel.fields.clone())
}

fn field_read_hint_expr(field: &FieldIr) -> String {
    if field.is_array && field.storage_strategy == Some(StorageStrategy::Json) {
        return "Some(crate::ValueHint::Json)".to_string();
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => {
            "Some(crate::ValueHint::Decimal)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::DateTime) => {
            "Some(crate::ValueHint::DateTime)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::Json | ScalarType::Jsonb) => {
            "Some(crate::ValueHint::Json)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::Uuid) => "Some(crate::ValueHint::Uuid)".to_string(),
        ResolvedFieldType::Scalar(ScalarType::Geometry) => {
            "Some(crate::ValueHint::Geometry)".to_string()
        }
        ResolvedFieldType::Scalar(ScalarType::Geography) => {
            "Some(crate::ValueHint::Geography)".to_string()
        }
        ResolvedFieldType::CompositeType { .. }
            if field.storage_strategy == Some(StorageStrategy::Json) =>
        {
            "Some(crate::ValueHint::Json)".to_string()
        }
        _ => "None".to_string(),
    }
}

fn composite_field_rust_type(
    field: &CompositeFieldIr,
    extensions: &ExtensionRegistry,
) -> Option<String> {
    match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => Some(scalar_to_rust_type(scalar, extensions)),
        ResolvedFieldType::Enum { enum_name } => Some(enum_name.clone()),
        _ => None,
    }
}

fn build_nested_order_by_fields(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
    reserved_methods: &HashSet<String>,
) -> Vec<NestedOrderByFieldContext> {
    let mut fields = Vec::new();
    let mut used_methods = reserved_methods.clone();

    for parent in model.scalar_fields() {
        if parent.is_array {
            continue;
        }

        let ResolvedFieldType::CompositeType { type_name, .. } = &parent.field_type else {
            continue;
        };
        let Some(composite) = ir.composite_types.get(type_name) else {
            continue;
        };

        for nested in &composite.fields {
            if !is_orderable_composite_field(nested) {
                continue;
            }
            let Some(rust_type) = composite_field_rust_type(nested, extensions) else {
                continue;
            };

            let path = format!("{}.{}", parent.logical_name, nested.logical_name);
            let base_method_name =
                format!("{}_{}", parent.logical_name, nested.logical_name).to_snake_case();
            let mut method_name = base_method_name.clone();
            let mut suffix = 0usize;
            while used_methods.contains(&method_name) {
                suffix += 1;
                method_name = if suffix == 1 {
                    format!("{base_method_name}_order")
                } else {
                    format!("{base_method_name}_order_{suffix}")
                };
            }
            used_methods.insert(method_name.clone());

            fields.push(NestedOrderByFieldContext {
                method_name,
                path,
                parent_db_name: parent.db_name.clone(),
                field_db_name: nested.db_name.clone(),
                json_key: nested.logical_name.clone(),
                json_cast: json_path_cast_variant(&nested.field_type).to_string(),
                rust_type,
            });
        }
    }

    fields
}

/// Generate complete code for a model (struct, impls, delegate, builders).
///
/// `is_async` determines whether the generated delegate methods and internal
/// builders use `async fn`/`.await` (`true`) or blocking sync wrappers (`false`).
pub fn generate_model(model: &ModelIr, ir: &SchemaIr, is_async: bool) -> String {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_model_with_registry(model, ir, is_async, &extensions)
}

fn generate_model_with_registry(
    model: &ModelIr,
    ir: &SchemaIr,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> String {
    let mut context = Context::new();
    insert_derived_names(&mut context, model);

    let pk_field_names = model.primary_key.fields();
    context.insert("primary_key_fields", &pk_field_names);

    let pk_fields_with_db = build_pk_fields(model, &pk_field_names);
    context.insert("pk_fields_with_db", &pk_fields_with_db);
    context.insert(
        "single_record_constraints",
        &build_single_record_constraints(model, &pk_fields_with_db),
    );

    let fields = build_scalar_fields(model, ir, extensions, &pk_field_names);
    let reserved_order_methods: HashSet<String> = fields
        .scalar
        .iter()
        .map(|field| field.name.clone())
        .collect();
    let nested_order_by_fields =
        build_nested_order_by_fields(model, ir, extensions, &reserved_order_methods);

    let relation_imports: Vec<String> = model
        .relation_fields()
        .filter_map(|field| match &field.field_type {
            ResolvedFieldType::Relation(rel) => Some(rel.target_model.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    context.insert("has_enums", &!fields.enum_imports.is_empty());
    context.insert("enum_imports", &fields.enum_imports);
    context.insert("has_relations", &!relation_imports.is_empty());
    context.insert("relation_imports", &relation_imports);
    context.insert(
        "has_composite_types",
        &!fields.composite_type_imports.is_empty(),
    );
    context.insert("composite_type_imports", &fields.composite_type_imports);

    let relation_fields: Vec<FieldContext> = model
        .relation_fields()
        .map(|field| relation_field_context(model, field, extensions))
        .collect();

    context.insert("scalar_fields", &fields.scalar);
    context.insert("relation_fields", &relation_fields);
    context.insert("relations", &build_relations(model, ir, extensions));
    context.insert("create_fields", &fields.create);
    context.insert("updated_at_fields", &fields.updated_at);
    context.insert("all_scalar_fields", &fields.scalar);
    context.insert("numeric_fields", &fields.numeric);
    context.insert("orderable_fields", &fields.orderable);
    context.insert("nested_order_by_fields", &nested_order_by_fields);
    context.insert("has_numeric_fields", &!fields.numeric.is_empty());
    context.insert("has_orderable_fields", &!fields.orderable.is_empty());
    context.insert("is_async", &is_async);

    render("model_file.tera", &context)
}

/// Insert the `{Model}Delegate` / `{Model}FindMany` / … type names the
/// templates refer to.
fn insert_derived_names(context: &mut Context, model: &ModelIr) {
    let name = &model.logical_name;
    context.insert("model_name", name);
    context.insert("table_name", &model.db_name);
    context.insert("delegate_name", &format!("{}Delegate", name));
    context.insert("columns_name", &format!("{}Columns", name));
    context.insert("find_many_name", &format!("{}FindMany", name));
    context.insert("create_name", &format!("{}Create", name));
    context.insert("create_many_name", &format!("{}CreateMany", name));
    context.insert("entry_name", &format!("{}CreateEntry", name));
    context.insert("update_name", &format!("{}Update", name));
    context.insert("delete_name", &format!("{}Delete", name));
}

fn build_pk_fields(model: &ModelIr, pk_field_names: &[&str]) -> Vec<PkFieldContext> {
    pk_field_names
        .iter()
        .filter_map(|logical| {
            model
                .scalar_fields()
                .find(|f| f.logical_name.as_str() == *logical)
                .map(pk_field_context)
        })
        .collect()
}

fn pk_field_context(field: &FieldIr) -> PkFieldContext {
    PkFieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
    }
}

/// Every column set that identifies at most one row: the primary key plus each
/// unique constraint, deduplicated by database column names.
fn build_single_record_constraints(
    model: &ModelIr,
    pk_fields: &[PkFieldContext],
) -> Vec<Vec<PkFieldContext>> {
    let mut constraints = Vec::new();
    let mut seen_keys = HashSet::new();
    let key_of = |fields: &[PkFieldContext]| -> Vec<String> {
        fields.iter().map(|field| field.db_name.clone()).collect()
    };

    if !pk_fields.is_empty() && seen_keys.insert(key_of(pk_fields)) {
        constraints.push(pk_fields.to_vec());
    }

    for constraint in &model.unique_constraints {
        let fields: Vec<PkFieldContext> = constraint
            .fields
            .iter()
            .filter_map(|logical| {
                model
                    .scalar_fields()
                    .find(|f| f.logical_name == *logical)
                    .map(pk_field_context)
            })
            .collect();

        if fields.len() != constraint.fields.len() || fields.is_empty() {
            continue;
        }
        if seen_keys.insert(key_of(&fields)) {
            constraints.push(fields);
        }
    }

    constraints
}

/// The per-field template contexts a model needs, collected in a single pass
/// over its scalar fields.
struct ScalarFieldContexts {
    scalar: Vec<FieldContext>,
    create: Vec<FieldContext>,
    updated_at: Vec<FieldContext>,
    numeric: Vec<AggregateFieldContext>,
    orderable: Vec<AggregateFieldContext>,
    enum_imports: Vec<String>,
    composite_type_imports: Vec<String>,
}

fn build_scalar_fields(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
    pk_field_names: &[&str],
) -> ScalarFieldContexts {
    let mut scalar = Vec::new();
    let mut create = Vec::new();
    let mut updated_at = Vec::new();
    let mut numeric = Vec::new();
    let mut orderable = Vec::new();
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
            _ => {}
        }

        let is_pk = pk_field_names.contains(&field.logical_name.as_str());
        let field_ctx = scalar_field_context(model, field, idx, is_pk, extensions);
        let base_rust_type = field_ctx.base_rust_type.clone();

        create.push(field_ctx.clone());
        if field.is_updated_at {
            updated_at.push(field_ctx.clone());
        }
        scalar.push(field_ctx);

        let is_numeric = matches!(
            &field.field_type,
            ResolvedFieldType::Scalar(ScalarType::Int)
                | ResolvedFieldType::Scalar(ScalarType::BigInt)
                | ResolvedFieldType::Scalar(ScalarType::Float)
                | ResolvedFieldType::Scalar(ScalarType::Decimal { .. })
        );
        if is_numeric {
            numeric.push(AggregateFieldContext {
                name: field.logical_name.to_snake_case(),
                logical_name: field.logical_name.clone(),
                rust_type: base_rust_type.clone(),
                avg_rust_type: field_to_rust_avg_type(field),
                sum_rust_type: field_to_rust_sum_type(field, extensions),
                variant_name: field.logical_name.to_pascal_case(),
            });
        }

        if is_orderable_model_field(field) {
            orderable.push(AggregateFieldContext {
                name: field.logical_name.to_snake_case(),
                logical_name: field.logical_name.clone(),
                rust_type: base_rust_type,
                avg_rust_type: String::new(),
                sum_rust_type: String::new(),
                variant_name: field.logical_name.to_pascal_case(),
            });
        }
    }

    ScalarFieldContexts {
        scalar,
        create,
        updated_at,
        numeric,
        orderable,
        enum_imports: enum_imports.into_iter().collect(),
        composite_type_imports: composite_type_imports.into_iter().collect(),
    }
}

fn scalar_field_context(
    model: &ModelIr,
    field: &FieldIr,
    index: usize,
    is_pk: bool,
    extensions: &ExtensionRegistry,
) -> FieldContext {
    let column_type = match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_rust_type(scalar, extensions),
        ResolvedFieldType::Enum { enum_name } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        _ => String::new(),
    };

    FieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        rust_type: field_to_rust_type(field, extensions),
        base_rust_type: field_to_rust_base_type(field, extensions),
        column_type,
        read_hint_expr: field_read_hint_expr(field),
        variant_name: field.logical_name.to_pascal_case(),
        is_array: field.is_array,
        index,
        is_pk,
        is_optional: !field.is_required && !field.is_array,
        is_updated_at: field.is_updated_at,
        is_computed: field.computed.is_some(),
        doc_comment: crate::schema_docs::field_modifier_doc(model, field),
    }
}

/// Relation fields carry no column of their own: they are always hydrated
/// separately, so they get no column type, no read hint and are optional.
fn relation_field_context(
    model: &ModelIr,
    field: &FieldIr,
    extensions: &ExtensionRegistry,
) -> FieldContext {
    FieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        rust_type: field_to_rust_type(field, extensions),
        base_rust_type: field_to_rust_base_type(field, extensions),
        column_type: String::new(),
        read_hint_expr: "None".to_string(),
        variant_name: field.logical_name.to_pascal_case(),
        is_array: field.is_array,
        index: 0,
        is_pk: false,
        is_optional: true,
        is_updated_at: false,
        is_computed: false,
        doc_comment: crate::schema_docs::field_modifier_doc(model, field),
    }
}

fn build_relations(
    model: &ModelIr,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Vec<RelationContext> {
    model
        .relation_fields()
        .filter_map(|field| {
            let ResolvedFieldType::Relation(rel) = &field.field_type else {
                return None;
            };
            let target_model = ir.models.get(&rel.target_model)?;
            let target_pk_names = target_model.primary_key.fields();
            let target_scalar_fields: Vec<FieldContext> = target_model
                .scalar_fields()
                .enumerate()
                .map(|(idx, f)| {
                    let is_pk = target_pk_names.contains(&f.logical_name.as_str());
                    scalar_field_context(target_model, f, idx, is_pk, extensions)
                })
                .collect();

            let (fields, references) = if rel.fields.is_empty() {
                resolve_inverse_relation_fields(
                    &model.logical_name,
                    rel.name.as_deref(),
                    target_model,
                )
            } else {
                (rel.fields.clone(), rel.references.clone())
            };

            Some(RelationContext {
                field_name: field.logical_name.to_snake_case(),
                target_model: rel.target_model.clone(),
                target_table: target_model.db_name.clone(),
                is_array: field.is_array,
                fields_db: db_names_for(model, &fields),
                references_db: db_names_for(target_model, &references),
                fields,
                references,
                target_scalar_fields,
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

/// Generate all models from a schema IR.
///
/// `is_async` is forwarded to every [`generate_model`] call.
pub fn generate_all_models(ir: &SchemaIr, is_async: bool) -> HashMap<String, String> {
    let extensions = ExtensionRegistry::from_schema(ir);
    generate_all_models_with_registry(ir, is_async, &extensions)
}

pub(crate) fn generate_all_models_with_registry(
    ir: &SchemaIr,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> HashMap<String, String> {
    let mut generated = HashMap::new();

    for (model_name, model_ir) in &ir.models {
        let code = generate_model_with_registry(model_ir, ir, is_async, extensions);
        generated.insert(model_name.clone(), code);
    }

    generated
}
