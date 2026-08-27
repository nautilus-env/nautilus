//! Code generator for Nautilus models, delegates, and builders.

use anyhow::{Context as _, Result};
use heck::{ToPascalCase, ToSnakeCase};
use nautilus_schema::ast::StorageStrategy;
use nautilus_schema::ir::{
    CompositeFieldIr, FieldIr, ModelIr, ResolvedFieldType, ScalarType, SchemaIr,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tera::{Context, Tera};

use crate::extension_types::ExtensionRegistry;
use crate::model_view::{FieldView, ModelView};
use crate::type_helpers::{
    field_to_rust_avg_type, field_to_rust_base_type, field_to_rust_sum_type, field_to_rust_type,
    is_orderable_composite_field, json_path_cast_variant, scalar_to_rust_type,
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

fn render(template: &str, ctx: &Context) -> Result<String> {
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
    let view = ModelView::new(model, ir, extensions);
    let mut context = Context::new();
    insert_derived_names(&mut context, &view);

    context.insert("primary_key_fields", &view.primary_key_fields);

    let pk_fields_with_db = build_pk_fields(&view);
    context.insert("pk_fields_with_db", &pk_fields_with_db);
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
        .with_context(|| format!("Failed to generate Rust model '{}'", view.logical_name()))
}

/// Insert the `{Model}Delegate` / `{Model}FindMany` / … type names the
/// templates refer to.
fn insert_derived_names(context: &mut Context, view: &ModelView<'_>) {
    let name = view.logical_name();
    context.insert("model_name", name);
    context.insert("table_name", view.db_name());
    context.insert("delegate_name", &format!("{}Delegate", name));
    context.insert("columns_name", &format!("{}Columns", name));
    context.insert("find_many_name", &format!("{}FindMany", name));
    context.insert("create_name", &format!("{}Create", name));
    context.insert("create_many_name", &format!("{}CreateMany", name));
    context.insert("entry_name", &format!("{}CreateEntry", name));
    context.insert("update_name", &format!("{}Update", name));
    context.insert("delete_name", &format!("{}Delete", name));
}

fn build_pk_fields(view: &ModelView<'_>) -> Vec<PkFieldContext> {
    view.primary_key_fields
        .iter()
        .filter_map(|logical| {
            view.scalars
                .iter()
                .find(|scalar| scalar.logical_name() == *logical)
                .map(|scalar| pk_field_context(scalar.field))
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
/// over the shared [`ModelView`].
#[derive(Default)]
struct ScalarFieldContexts {
    scalar: Vec<FieldContext>,
    create: Vec<FieldContext>,
    updated_at: Vec<FieldContext>,
    numeric: Vec<AggregateFieldContext>,
    orderable: Vec<AggregateFieldContext>,
}

fn build_scalar_fields(
    view: &ModelView<'_>,
    extensions: &ExtensionRegistry,
) -> ScalarFieldContexts {
    let mut contexts = ScalarFieldContexts::default();

    for scalar in &view.scalars {
        let field = scalar.field;
        let field_ctx = scalar_field_context(scalar, extensions);
        let base_rust_type = field_ctx.base_rust_type.clone();

        contexts.create.push(field_ctx.clone());
        if field.is_updated_at {
            contexts.updated_at.push(field_ctx.clone());
        }
        contexts.scalar.push(field_ctx);

        if scalar.numeric_scalar().is_some() {
            contexts.numeric.push(AggregateFieldContext {
                name: scalar.snake_name(),
                logical_name: field.logical_name.clone(),
                rust_type: base_rust_type.clone(),
                avg_rust_type: field_to_rust_avg_type(field),
                sum_rust_type: field_to_rust_sum_type(field, extensions),
                variant_name: field.logical_name.to_pascal_case(),
            });
        }

        if scalar.is_orderable() {
            contexts.orderable.push(AggregateFieldContext {
                name: scalar.snake_name(),
                logical_name: field.logical_name.clone(),
                rust_type: base_rust_type,
                avg_rust_type: String::new(),
                sum_rust_type: String::new(),
                variant_name: field.logical_name.to_pascal_case(),
            });
        }
    }

    contexts
}

fn scalar_field_context(scalar: &FieldView<'_>, extensions: &ExtensionRegistry) -> FieldContext {
    let field = scalar.field;
    let column_type = match &field.field_type {
        ResolvedFieldType::Scalar(scalar) => scalar_to_rust_type(scalar, extensions),
        ResolvedFieldType::Enum { enum_name } => enum_name.clone(),
        ResolvedFieldType::CompositeType { type_name, .. } => type_name.clone(),
        _ => String::new(),
    };

    FieldContext {
        name: scalar.snake_name(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        rust_type: field_to_rust_type(field, extensions),
        base_rust_type: field_to_rust_base_type(field, extensions),
        column_type,
        read_hint_expr: field_read_hint_expr(field),
        variant_name: field.logical_name.to_pascal_case(),
        is_array: field.is_array,
        index: scalar.index,
        is_pk: scalar.is_pk,
        is_optional: !field.is_required && !field.is_array,
        is_updated_at: field.is_updated_at,
        is_computed: field.computed.is_some(),
        doc_comment: scalar.doc_comment.clone(),
    }
}

/// Relation fields carry no column of their own: they are always hydrated
/// separately, so they get no column type, no read hint and are optional.
fn build_relation_fields(
    view: &ModelView<'_>,
    extensions: &ExtensionRegistry,
) -> Vec<FieldContext> {
    view.relations
        .iter()
        .map(|relation| {
            let field = relation.field;
            FieldContext {
                name: relation.snake_name(),
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
                doc_comment: crate::schema_docs::field_modifier_doc(view.model, field),
            }
        })
        .collect()
}

fn build_relations(
    view: &ModelView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> Vec<RelationContext> {
    view.resolved_relations()
        .map(|(relation, target)| {
            let target_view = ModelView::new(target, ir, extensions);
            RelationContext {
                field_name: relation.snake_name(),
                target_model: relation.target_model_name().to_string(),
                target_table: target.db_name.clone(),
                is_array: relation.is_array(),
                fields_db: relation.fields_db.clone(),
                references_db: relation.references_db.clone(),
                fields: relation.fields.clone(),
                references: relation.references.clone(),
                target_scalar_fields: target_view
                    .scalars
                    .iter()
                    .map(|scalar| scalar_field_context(scalar, extensions))
                    .collect(),
            }
        })
        .collect()
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
