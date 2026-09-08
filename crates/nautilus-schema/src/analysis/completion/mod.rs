//! Completion suggestions for `.nautilus` schema files.

use super::{analyze, catalog, span_contains, AnalysisResult};

mod attribute_args;
mod context;

use crate::ast::Declaration;
use crate::token::Token;
use attribute_args::{
    attr_argument_completions, datasource_extension_array_item_completions,
    datasource_extension_value_completions,
};
use context::{
    attr_arg_index_at, attribute_context_at, config_block_kind_at, config_value_context_at,
    declaration_context_at_tokens, extract_provider_from_tokens, inside_attr_args_at,
    user_enums_from_tokens, AttributeContext, ConfigBlockKind, DeclarationContext,
};

/// The kind of a completion item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// A language keyword (`model`, `enum`, …).
    Keyword,
    /// A scalar or user-defined field type.
    Type,
    /// A field-level attribute (`@id`, `@unique`, …).
    FieldAttribute,
    /// A model-level attribute (`@@id`, `@@map`, …).
    ModelAttribute,
    /// A reference to a model name.
    ModelName,
    /// A reference to an enum name.
    EnumName,
    /// A field name inside a model or datasource.
    FieldName,
}

/// A single completion suggestion.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionItem {
    /// The text displayed in the completion popup.
    pub label: String,
    /// The text to actually insert (defaults to `label` when `None`).
    pub insert_text: Option<String>,
    /// Whether `insert_text` uses LSP snippet syntax (`$1`, `${1:placeholder}`, etc.).
    pub is_snippet: bool,
    /// What kind of thing this item represents.
    pub kind: CompletionKind,
    /// Optional extra description shown in the completion popup.
    pub detail: Option<String>,
}

impl CompletionItem {
    pub(super) fn new(
        label: impl Into<String>,
        kind: CompletionKind,
        detail: impl Into<Option<String>>,
    ) -> Self {
        Self {
            label: label.into(),
            insert_text: None,
            is_snippet: false,
            kind,
            detail: detail.into(),
        }
    }

    pub(super) fn with_insert(
        label: impl Into<String>,
        insert_text: impl Into<String>,
        kind: CompletionKind,
        detail: impl Into<Option<String>>,
    ) -> Self {
        Self {
            label: label.into(),
            insert_text: Some(insert_text.into()),
            is_snippet: false,
            kind,
            detail: detail.into(),
        }
    }

    pub(super) fn with_snippet(
        label: impl Into<String>,
        snippet: impl Into<String>,
        kind: CompletionKind,
        detail: impl Into<Option<String>>,
    ) -> Self {
        Self {
            label: label.into(),
            insert_text: Some(snippet.into()),
            is_snippet: true,
            kind,
            detail: detail.into(),
        }
    }
}

/// Returns completions appropriate at `offset` (byte offset) in `source`.
///
/// Uses the parsed AST to determine context:
/// - Outside all declarations -> top-level keywords.
/// - Inside a `datasource` or `generator` block -> config key suggestions.
/// - Inside a `model` block:
///   - After `@` -> field attribute names.
///   - After `@@` -> model attribute names.
///   - Otherwise -> scalar types, user-defined model/enum names, and common
///     field attributes as a convenience.
pub fn completion(source: &str, offset: usize) -> Vec<CompletionItem> {
    let result = analyze(source);
    completion_with_analysis(source, &result, offset)
}

/// Returns completions appropriate at `offset` using a previously computed
/// [`AnalysisResult`].
pub fn completion_with_analysis(
    _source: &str,
    result: &AnalysisResult,
    offset: usize,
) -> Vec<CompletionItem> {
    let tokens = &result.tokens;
    let provider: Option<String> = extract_provider_from_tokens(tokens);
    let provider = provider.as_deref();

    if let Some(items) = attribute_completions_at(tokens, offset, provider) {
        return items;
    }
    if let Some(items) = config_completions_at(tokens, offset, provider) {
        return items;
    }

    let Some(ast) = result.ast.as_ref() else {
        // AST unavailable (e.g. fatal parse error).  Use the raw token
        // stream to make a best-effort guess about the enclosing block.
        return match declaration_context_at_tokens(tokens, offset) {
            DeclarationContext::Model => scalar_type_completions(provider),
            DeclarationContext::Type => {
                type_body_completions(provider, &UserTypes::from_tokens(tokens))
            }
            DeclarationContext::Other => top_level_completions(),
        };
    };

    let user_types = UserTypes::from_ast(ast);
    let containing_decl = ast
        .declarations
        .iter()
        .find(|d| span_contains(d.span(), offset));

    match containing_decl {
        // The offset isn't inside any parsed declaration. This can happen when
        // error recovery dropped the enclosing block.  Fall back to the token
        // stream to make a best-effort guess.
        None => match declaration_context_at_tokens(tokens, offset) {
            DeclarationContext::Model => model_body_completions(provider, &user_types),
            DeclarationContext::Type => type_body_completions(provider, &user_types),
            DeclarationContext::Other => top_level_completions(),
        },

        Some(Declaration::Datasource(_)) => datasource_field_completions(),
        Some(Declaration::Generator(_)) => generator_field_completions(),

        // Inside an enum body: only enum variants are meaningful here, and they
        // are user-defined identifiers.
        Some(Declaration::Enum(_)) => Vec::new(),

        Some(Declaration::Import(_)) => Vec::new(),
        Some(Declaration::Model(_)) => model_body_completions(provider, &user_types),
        Some(Declaration::Type(_)) => type_body_completions(provider, &user_types),
    }
}

/// Completions for an attribute position: inside an attribute's arguments,
/// after `@`, or after `@@`.  `None` when the offset is not in an attribute.
fn attribute_completions_at(
    tokens: &[Token],
    offset: usize,
    provider: Option<&str>,
) -> Option<Vec<CompletionItem>> {
    if let Some(attr_name) = inside_attr_args_at(tokens, offset) {
        let arg_index = attr_arg_index_at(tokens, offset).unwrap_or(0);
        return Some(attr_argument_completions(&attr_name, provider, arg_index));
    }

    match attribute_context_at(tokens, offset) {
        AttributeContext::FieldAttr => Some(field_attribute_completions()),
        AttributeContext::ModelAttr => {
            // Composite types only support `@@map`; restrict the suggestion list.
            if declaration_context_at_tokens(tokens, offset) == DeclarationContext::Type {
                Some(type_attribute_completions())
            } else {
                Some(model_attribute_completions())
            }
        }
        AttributeContext::None => None,
    }
}

/// Completions for a `datasource` / `generator` config value position.
/// `None` when the offset is not on a config value, or nothing is known for it.
fn config_completions_at(
    tokens: &[Token],
    offset: usize,
    provider: Option<&str>,
) -> Option<Vec<CompletionItem>> {
    if let Some(completions) = datasource_extension_array_item_completions(tokens, offset, provider)
    {
        return Some(completions);
    }

    let key = config_value_context_at(tokens, offset)?;
    if key == "extensions" {
        let completions = datasource_extension_value_completions(tokens, offset, provider);
        if !completions.is_empty() {
            return Some(completions);
        }
    }

    let completions = config_value_completions(&key, config_block_kind_at(tokens, offset));
    if completions.is_empty() {
        None
    } else {
        Some(completions)
    }
}

/// Names declared in the document that can be referenced from a body.
#[derive(Default)]
struct UserTypes {
    models: Vec<String>,
    enums: Vec<String>,
    composite_types: Vec<String>,
}

impl UserTypes {
    fn from_ast(ast: &crate::ast::Schema) -> Self {
        let mut types = Self::default();
        for declaration in &ast.declarations {
            match declaration {
                Declaration::Model(m) => types.models.push(m.name.value.clone()),
                Declaration::Enum(e) => types.enums.push(e.name.value.clone()),
                Declaration::Type(t) => types.composite_types.push(t.name.value.clone()),
                _ => {}
            }
        }
        types
    }

    fn from_tokens(tokens: &[Token]) -> Self {
        Self {
            enums: user_enums_from_tokens(tokens),
            ..Self::default()
        }
    }
}

/// Scalar types plus every model, enum and composite type reference valid in a
/// model body.
fn model_body_completions(provider: Option<&str>, user_types: &UserTypes) -> Vec<CompletionItem> {
    let mut items = type_body_completions(provider, user_types);
    push_references(
        &mut items,
        &user_types.models,
        CompletionKind::ModelName,
        "Model reference",
    );
    push_references(
        &mut items,
        &user_types.composite_types,
        CompletionKind::Type,
        "Composite type reference",
    );
    items
}

/// Scalar types plus enum references — composite type bodies cannot hold model
/// or composite fields.
fn type_body_completions(provider: Option<&str>, user_types: &UserTypes) -> Vec<CompletionItem> {
    let mut items = scalar_type_completions(provider);
    push_references(
        &mut items,
        &user_types.enums,
        CompletionKind::EnumName,
        "Enum reference",
    );
    items
}

fn push_references(
    items: &mut Vec<CompletionItem>,
    names: &[String],
    kind: CompletionKind,
    detail: &str,
) {
    items.extend(
        names
            .iter()
            .map(|name| CompletionItem::new(name.clone(), kind, Some(detail.to_string()))),
    );
}

fn top_level_completions() -> Vec<CompletionItem> {
    vec![
        CompletionItem::new(
            "model",
            CompletionKind::Keyword,
            Some("Define a data model".to_string()),
        ),
        CompletionItem::new(
            "view",
            CompletionKind::Keyword,
            Some("Define a read-only database view".to_string()),
        ),
        CompletionItem::new(
            "enum",
            CompletionKind::Keyword,
            Some("Define an enumeration".to_string()),
        ),
        CompletionItem::new(
            "type",
            CompletionKind::Keyword,
            Some("Define a composite type".to_string()),
        ),
        CompletionItem::new(
            "datasource",
            CompletionKind::Keyword,
            Some("Configure a data source".to_string()),
        ),
        CompletionItem::new(
            "generator",
            CompletionKind::Keyword,
            Some("Configure code generation".to_string()),
        ),
        CompletionItem::with_snippet(
            "import",
            "import \"$1\"",
            CompletionKind::Keyword,
            Some("Join another schema file to this one".to_string()),
        ),
    ]
}

fn scalar_type_completions(provider: Option<&str>) -> Vec<CompletionItem> {
    catalog::SCALAR_TYPES
        .iter()
        .filter(|doc| doc.supported_by(provider))
        .map(|doc| match doc.snippet {
            Some(snippet) => CompletionItem::with_snippet(
                doc.label,
                snippet,
                CompletionKind::Type,
                Some(doc.detail()),
            ),
            None => CompletionItem::new(doc.label, CompletionKind::Type, Some(doc.detail())),
        })
        .collect()
}

fn field_attribute_completions() -> Vec<CompletionItem> {
    catalog::FIELD_ATTRIBUTES
        .iter()
        .map(|doc| attribute_item(doc, CompletionKind::FieldAttribute))
        .collect()
}

/// Type-level attribute completions (composite types only support `@@map`).
fn type_attribute_completions() -> Vec<CompletionItem> {
    vec![attribute_item(
        &catalog::type_attribute_map(),
        CompletionKind::ModelAttribute,
    )]
}

fn model_attribute_completions() -> Vec<CompletionItem> {
    catalog::MODEL_ATTRIBUTES
        .iter()
        .map(|doc| attribute_item(doc, CompletionKind::ModelAttribute))
        .collect()
}

/// One catalog attribute as the completion item that offers it.
fn attribute_item(doc: &catalog::AttributeDoc, kind: CompletionKind) -> CompletionItem {
    match doc.snippet {
        Some(snippet) => {
            CompletionItem::with_snippet(doc.label, snippet, kind, Some(doc.detail.to_string()))
        }
        None => CompletionItem::new(doc.label, kind, Some(doc.detail.to_string())),
    }
}

fn datasource_field_completions() -> Vec<CompletionItem> {
    catalog::CONFIG_FIELDS
        .iter()
        .filter_map(|doc| {
            doc.datasource_detail.map(|detail| {
                CompletionItem::new(doc.key, CompletionKind::FieldName, Some(detail.to_string()))
            })
        })
        .collect()
}

fn generator_field_completions() -> Vec<CompletionItem> {
    catalog::CONFIG_FIELDS
        .iter()
        .filter_map(|doc| {
            doc.generator_detail.map(|detail| {
                CompletionItem::new(doc.key, CompletionKind::FieldName, Some(detail.to_string()))
            })
        })
        .collect()
}

fn config_value_completions(key: &str, block_kind: Option<ConfigBlockKind>) -> Vec<CompletionItem> {
    let Some(field) = catalog::config_field(key) else {
        return Vec::new();
    };

    field
        .values
        .iter()
        .filter(|value| {
            !matches!(
                (value.block, block_kind),
                (
                    Some(catalog::ConfigBlock::Datasource),
                    Some(ConfigBlockKind::Generator)
                ) | (
                    Some(catalog::ConfigBlock::Generator),
                    Some(ConfigBlockKind::Datasource)
                )
            )
        })
        .map(|value| {
            CompletionItem::with_insert(
                value.value,
                format!("\"{}\"", value.value),
                CompletionKind::Keyword,
                Some(value.detail.to_string()),
            )
        })
        .collect()
}
