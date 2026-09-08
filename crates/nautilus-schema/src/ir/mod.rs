//! Intermediate representation (IR) of a validated schema.
//!
//! This module defines a provider-agnostic IR that represents a schema after
//! semantic validation. All type references are resolved, relations are validated,
//! and both logical and physical names are stored explicitly.
mod config;
mod defaults;
pub mod index;
mod model;
mod provider;
mod relations;
mod types;

pub use crate::ast::ComputedKind;
use std::collections::HashMap;

pub use config::{
    DatasourceIr, GeneratorIr, InterfaceKind, JavaGenerationMode, PostgresExtensionIr,
};
pub use defaults::{DefaultValue, FunctionCall};
pub use index::{
    parse_index_type_tag, BasicIndexType, IndexIr, IndexKind, IndexTypeTag,
    ParseBasicIndexTypeError, PgvectorIndex, PgvectorIndexOptions, PgvectorMethod, PgvectorOpClass,
    ALL_INDEX_TYPE_NAMES,
};
pub use model::{
    CompositeFieldIr, CompositeTypeIr, EnumIr, FieldIr, ModelIr, PrimaryKeyIr, UniqueConstraintIr,
};
pub use provider::{
    ClientProvider, DatabaseProvider, ParseClientProviderError, ParseDatabaseProviderError,
};
pub use relations::{ManyToManyJoinIr, RelationIr};
pub use types::{ResolvedFieldType, ScalarType};

/// Validated intermediate representation of a complete schema.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaIr {
    /// The datasource declaration (if present).
    pub datasource: Option<DatasourceIr>,
    /// The generator declaration (if present).
    pub generator: Option<GeneratorIr>,
    /// All models in the schema, indexed by logical name.
    pub models: HashMap<String, ModelIr>,
    /// All enums in the schema, indexed by logical name.
    pub enums: HashMap<String, EnumIr>,
    /// All composite types in the schema, indexed by logical name.
    pub composite_types: HashMap<String, CompositeTypeIr>,
}

impl SchemaIr {
    /// Creates a new empty schema IR.
    pub fn new() -> Self {
        Self {
            datasource: None,
            generator: None,
            models: HashMap::new(),
            enums: HashMap::new(),
            composite_types: HashMap::new(),
        }
    }

    /// A copy of this schema with every `@@ignore`d model and `@ignore`d field
    /// removed.
    ///
    /// An ignored declaration names something that exists in the database but
    /// that Nautilus does not manage, so it must not reach a generated client:
    /// there is no faithful type for it and no way to write it. Code generation
    /// and the engine therefore run on the pruned schema, while everything that
    /// describes the schema *as written* — the formatter, the language server,
    /// `db pull` — keeps using the full IR. Migrations are the third case: they
    /// need to know an ignored table exists so they leave it alone instead of
    /// dropping it, so they read `is_ignored` directly rather than pruning.
    pub fn without_ignored(&self) -> SchemaIr {
        SchemaIr {
            datasource: self.datasource.clone(),
            generator: self.generator.clone(),
            models: self
                .models
                .iter()
                .filter(|(_, model)| !model.is_ignored)
                .map(|(name, model)| {
                    let mut kept = model.clone();
                    kept.fields.retain(|field| !field.is_ignored);
                    (name.clone(), kept)
                })
                .collect(),
            enums: self.enums.clone(),
            composite_types: self.composite_types.clone(),
        }
    }

    /// A copy of this schema without the join tables Nautilus synthesised for
    /// implicit many-to-many relations.
    ///
    /// A join table is an implementation detail of the two array fields it
    /// links: it has no meaning to someone writing against a generated client,
    /// and exposing it would offer a second, untyped way to write the relation.
    /// Code generation therefore prunes it, while migrations and the engine —
    /// which have to create it and query through it — keep it.
    pub fn without_join_tables(&self) -> SchemaIr {
        SchemaIr {
            datasource: self.datasource.clone(),
            generator: self.generator.clone(),
            models: self
                .models
                .iter()
                .filter(|(_, model)| !model.is_join_table)
                .map(|(name, model)| (name.clone(), model.clone()))
                .collect(),
            enums: self.enums.clone(),
            composite_types: self.composite_types.clone(),
        }
    }

    /// Gets a model by logical name.
    pub fn get_model(&self, name: &str) -> Option<&ModelIr> {
        self.models.get(name)
    }

    /// Gets an enum by logical name.
    pub fn get_enum(&self, name: &str) -> Option<&EnumIr> {
        self.enums.get(name)
    }

    /// Gets a composite type by logical name.
    pub fn get_composite_type(&self, name: &str) -> Option<&CompositeTypeIr> {
        self.composite_types.get(name)
    }
}

impl Default for SchemaIr {
    fn default() -> Self {
        Self::new()
    }
}
