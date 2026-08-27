//! Language-neutral view of a model, shared by every code generation backend.
//!
//! Each backend used to walk the IR on its own and rediscover the same
//! structural facts — which field is a primary key, which columns carry an
//! object value, which enums and composite types must be imported, how a
//! relation resolves to its foreign key columns. [`ModelView`] computes those
//! facts once, and the backends map them onto their own Tera contexts through
//! [`crate::backend::LanguageBackend`] for the language-specific type names.
//!
//! Everything here is deliberately free of type names, syntax, and formatting:
//! it answers *what* a model contains, never *how* a language spells it.

use heck::ToSnakeCase;
use nautilus_schema::ir::{FieldIr, ModelIr, RelationIr, ResolvedFieldType, ScalarType, SchemaIr};
use std::collections::{BTreeMap, BTreeSet};

use crate::extension_types::{ExtensionRegistry, ExtensionType};
use crate::type_helpers::{is_orderable_composite_field, is_orderable_model_field};

/// One scalar field of a model, with the classifications every backend needs.
pub(crate) struct FieldView<'a> {
    pub field: &'a FieldIr,
    /// Position among the model's scalar fields, used for column indexing.
    pub index: usize,
    pub is_pk: bool,
    /// The extension type backing this field, when it comes from a PostgreSQL
    /// extension (pgvector, PostGIS, …).
    pub extension_type: Option<ExtensionType>,
    pub doc_comment: String,
}

impl FieldView<'_> {
    pub fn logical_name(&self) -> &str {
        &self.field.logical_name
    }

    pub fn snake_name(&self) -> String {
        self.field.logical_name.to_snake_case()
    }

    pub fn is_enum(&self) -> bool {
        matches!(self.field.field_type, ResolvedFieldType::Enum { .. })
    }

    /// `true` for a single (non-array) column whose value is a JSON-like
    /// object, which the clients must pass through unflattened.
    pub fn is_object_valued(&self) -> bool {
        !self.field.is_array
            && matches!(
                self.field.field_type,
                ResolvedFieldType::Scalar(
                    ScalarType::Json | ScalarType::Jsonb | ScalarType::Hstore
                )
            )
    }

    /// The scalar type when the field can take part in numeric aggregates
    /// (`_avg` / `_sum`), `None` otherwise.
    pub fn numeric_scalar(&self) -> Option<&ScalarType> {
        match &self.field.field_type {
            ResolvedFieldType::Scalar(
                scalar @ (ScalarType::Int
                | ScalarType::BigInt
                | ScalarType::Float
                | ScalarType::Decimal { .. }),
            ) => Some(scalar),
            _ => None,
        }
    }

    pub fn is_orderable(&self) -> bool {
        is_orderable_model_field(self.field)
    }
}

/// One relation field of a model, resolved against the schema.
pub(crate) struct RelationView<'a> {
    pub field: &'a FieldIr,
    pub relation: &'a RelationIr,
    /// Position among the model's relation fields.
    pub index: usize,
    /// The target model, or `None` when the schema does not declare it.
    pub target: Option<&'a ModelIr>,
    /// Foreign key columns on the owning side, resolved through the inverse
    /// relation when this side declares none.
    pub fields: Vec<String>,
    pub references: Vec<String>,
    pub fields_db: Vec<String>,
    pub references_db: Vec<String>,
}

impl<'a> RelationView<'a> {
    pub fn logical_name(&self) -> &str {
        &self.field.logical_name
    }

    pub fn snake_name(&self) -> String {
        self.field.logical_name.to_snake_case()
    }

    pub fn target_model_name(&self) -> &str {
        &self.relation.target_model
    }

    pub fn is_array(&self) -> bool {
        self.field.is_array
    }

    /// The resolved target together with this view, for the backends that can
    /// only emit a relation whose target model exists.
    pub fn resolved_target(&self) -> Option<&'a ModelIr> {
        self.target
    }
}

/// An order-by path that reaches into a composite type column.
pub(crate) struct DottedOrderBy {
    /// Logical name of the composite column.
    pub parent: String,
    /// Logical name of the orderable field inside the composite.
    pub child: String,
}

impl DottedOrderBy {
    /// The wire form the engine expects: `parent.child`.
    pub fn path(&self) -> String {
        format!("{}.{}", self.parent, self.child)
    }
}

/// A module of extension types a generated model file must import.
pub(crate) struct ExtensionImportView {
    pub module: String,
    pub types: Vec<String>,
    /// The `…Input` companion of every entry in `types`.
    pub input_types: Vec<String>,
}

/// Everything a backend needs to know about a model before it starts naming
/// types.
pub(crate) struct ModelView<'a> {
    pub model: &'a ModelIr,
    pub primary_key_fields: Vec<&'a str>,
    pub scalars: Vec<FieldView<'a>>,
    pub relations: Vec<RelationView<'a>>,
    pub enum_imports: Vec<String>,
    pub composite_type_imports: Vec<String>,
    pub relation_imports: Vec<String>,
    pub extension_imports: BTreeMap<String, BTreeSet<String>>,
    pub vector_field_names: Vec<String>,
    pub object_value_db_names: Vec<String>,
    /// Order-by paths that reach into composite type columns.
    pub dotted_order_by: Vec<DottedOrderBy>,
}

impl<'a> ModelView<'a> {
    pub fn new(model: &'a ModelIr, ir: &'a SchemaIr, extensions: &ExtensionRegistry) -> Self {
        let primary_key_fields = model.primary_key.fields();

        let mut enum_imports = BTreeSet::new();
        let mut composite_type_imports = BTreeSet::new();
        let mut extension_imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut vector_field_names = Vec::new();
        let mut object_value_db_names = Vec::new();
        let mut scalars = Vec::new();

        for (index, field) in model.scalar_fields().enumerate() {
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

            let extension_type = extensions.type_for_field(field);
            if let Some(ty) = extension_type {
                extension_imports
                    .entry(ty.extension.to_string())
                    .or_default()
                    .insert(ty.type_name.to_string());
            }

            let view = FieldView {
                field,
                index,
                is_pk: primary_key_fields.contains(&field.logical_name.as_str()),
                extension_type,
                doc_comment: crate::schema_docs::field_modifier_doc(model, field),
            };

            if field.is_vector() {
                vector_field_names.push(field.logical_name.clone());
            }
            if view.is_object_valued() {
                object_value_db_names.push(field.db_name.clone());
            }
            scalars.push(view);
        }

        let relations = build_relations(model, ir);
        let relation_imports: BTreeSet<String> = relations
            .iter()
            .map(|relation| relation.relation.target_model.clone())
            .collect();

        Self {
            model,
            primary_key_fields,
            scalars,
            relations,
            enum_imports: enum_imports.into_iter().collect(),
            composite_type_imports: composite_type_imports.into_iter().collect(),
            relation_imports: relation_imports.into_iter().collect(),
            extension_imports,
            vector_field_names,
            object_value_db_names,
            dotted_order_by: dotted_order_by(model, ir),
        }
    }

    pub fn logical_name(&self) -> &str {
        &self.model.logical_name
    }

    pub fn snake_name(&self) -> String {
        self.model.logical_name.to_snake_case()
    }

    pub fn db_name(&self) -> &str {
        &self.model.db_name
    }

    /// The relations whose target model exists in the schema.
    pub fn resolved_relations(&self) -> impl Iterator<Item = (&RelationView<'a>, &'a ModelIr)> {
        self.relations
            .iter()
            .filter_map(|relation| relation.resolved_target().map(|target| (relation, target)))
    }

    /// The extension type modules to import, sorted by module name.
    pub fn extension_import_views(&self) -> Vec<ExtensionImportView> {
        self.extension_imports
            .iter()
            .map(|(module, types)| {
                let types: Vec<String> = types.iter().cloned().collect();
                let input_types = types.iter().map(|name| format!("{name}Input")).collect();
                ExtensionImportView {
                    module: module.clone(),
                    types,
                    input_types,
                }
            })
            .collect()
    }
}

fn build_relations<'a>(model: &'a ModelIr, ir: &'a SchemaIr) -> Vec<RelationView<'a>> {
    model
        .relation_fields()
        .enumerate()
        .filter_map(|(index, field)| {
            let ResolvedFieldType::Relation(relation) = &field.field_type else {
                return None;
            };
            let target = ir.models.get(&relation.target_model);

            let (fields, references) = match target {
                Some(target) if relation.fields.is_empty() => resolve_inverse_relation_fields(
                    &model.logical_name,
                    relation.name.as_deref(),
                    target,
                ),
                _ => (relation.fields.clone(), relation.references.clone()),
            };

            Some(RelationView {
                field,
                relation,
                index,
                target,
                fields_db: db_names_for(model, &fields),
                references_db: target
                    .map(|target| db_names_for(target, &references))
                    .unwrap_or_default(),
                fields,
                references,
            })
        })
        .collect()
}

/// The `(fields, references)` of the relation on `target_model` that points
/// back at `source_model_name`, swapped so they read from the source side.
///
/// A relation declared without `fields`/`references` is the inverse side; the
/// owning side carries the foreign key and is the only place to find it.
fn resolve_inverse_relation_fields(
    source_model_name: &str,
    relation_name: Option<&str>,
    target_model: &ModelIr,
) -> (Vec<String>, Vec<String>) {
    let inverse = target_model.relation_fields().find(|field| {
        let ResolvedFieldType::Relation(inverse) = &field.field_type else {
            return false;
        };
        inverse.target_model == source_model_name
            && match (relation_name, inverse.name.as_deref()) {
                (Some(expected), Some(actual)) => actual == expected,
                (None, None) => true,
                _ => false,
            }
    });

    let Some(inverse_field) = inverse else {
        return (vec![], vec![]);
    };
    let ResolvedFieldType::Relation(inverse) = &inverse_field.field_type else {
        return (vec![], vec![]);
    };

    (inverse.references.clone(), inverse.fields.clone())
}

/// Map logical field names onto the database column names of `model`.
pub(crate) fn db_names_for(model: &ModelIr, logical_names: &[String]) -> Vec<String> {
    logical_names
        .iter()
        .filter_map(|logical_name| {
            model
                .fields
                .iter()
                .find(|field| &field.logical_name == logical_name)
                .map(|field| field.db_name.clone())
        })
        .collect()
}

/// The order-by paths reaching the orderable fields of every non-array
/// composite type column.
fn dotted_order_by(model: &ModelIr, ir: &SchemaIr) -> Vec<DottedOrderBy> {
    let mut paths = Vec::new();
    for parent in model.scalar_fields().filter(|field| !field.is_array) {
        let ResolvedFieldType::CompositeType { type_name, .. } = &parent.field_type else {
            continue;
        };
        let Some(composite) = ir.composite_types.get(type_name) else {
            continue;
        };
        for nested in &composite.fields {
            if is_orderable_composite_field(nested) {
                paths.push(DottedOrderBy {
                    parent: parent.logical_name.clone(),
                    child: nested.logical_name.clone(),
                });
            }
        }
    }
    paths
}
