//! What an attribute accepts inside its parentheses.
//!
//! These answer once the cursor is known to be inside `@attr(…)`: the named
//! arguments of `@@index`, the functions `@default` accepts, and the
//! PostgreSQL extension names a datasource's `extensions = [...]` array takes.

use super::context::{extensions_value_mode_at, ExtensionsValueMode};
use super::{CompletionItem, CompletionKind};
use crate::token::{Token, TokenKind};
use crate::validator::KNOWN_POSTGRES_EXTENSIONS;

pub(super) fn attr_argument_completions(
    attr_name: &str,
    provider: Option<&str>,
    arg_index: usize,
) -> Vec<CompletionItem> {
    match attr_name {
        "store" => vec![CompletionItem::new(
            "json",
            CompletionKind::FieldAttribute,
            Some("Serialize array as JSON in the database".to_string()),
        )],
        "relation" => vec![
            CompletionItem::new(
                "fields: []",
                CompletionKind::FieldName,
                Some("Local FK field(s) on this model".to_string()),
            ),
            CompletionItem::new(
                "references: []",
                CompletionKind::FieldName,
                Some("Referenced field(s) on the target model".to_string()),
            ),
            CompletionItem::new(
                "name: \"\"",
                CompletionKind::FieldName,
                Some(
                    "Relation name (required when multiple relations to the same model)"
                        .to_string(),
                ),
            ),
            CompletionItem::new(
                "onDelete: Cascade",
                CompletionKind::FieldName,
                Some("Referential action on parent record delete".to_string()),
            ),
            CompletionItem::new(
                "onUpdate: Cascade",
                CompletionKind::FieldName,
                Some("Referential action on parent record update".to_string()),
            ),
        ],
        "default" => {
            let mut items = vec![
                CompletionItem::new(
                    "autoincrement()",
                    CompletionKind::Keyword,
                    Some("Auto-incrementing integer sequence".to_string()),
                ),
                CompletionItem::new(
                    "now()",
                    CompletionKind::Keyword,
                    Some("Current timestamp at insert time".to_string()),
                ),
                CompletionItem::new(
                    "uuid()",
                    CompletionKind::Keyword,
                    Some("Randomly generated UUID".to_string()),
                ),
            ];
            if matches!(provider, Some("postgresql") | None) {
                items.push(CompletionItem::new(
                    "uuidv7()",
                    CompletionKind::Keyword,
                    Some("Time-ordered UUIDv7 (PostgreSQL)".to_string()),
                ));
            }
            items
        }
        "computed" => match arg_index {
            0 => vec![CompletionItem::new(
                "SQL expression",
                CompletionKind::Keyword,
                Some("e.g. price * quantity  or  first_name || ' ' || last_name".to_string()),
            )],
            _ => vec![
                CompletionItem::new(
                    "Stored",
                    CompletionKind::Keyword,
                    Some("Computed on write, persisted on disk (all databases)".to_string()),
                ),
                CompletionItem::new(
                    "Virtual",
                    CompletionKind::Keyword,
                    Some("Computed on read, never stored (MySQL / SQLite only)".to_string()),
                ),
            ],
        },
        "index" => index_argument_completions(provider),
        _ => vec![],
    }
}

/// Return argument completions for `@@index(…)`, filtered by DB provider when known.
///
/// All DB types:   BTree (default, always shown)
/// PG + MySQL:     Hash
/// PG only:        Gin, Gist, Brin, Hnsw, Ivfflat
/// MySQL only:     FullText
fn index_argument_completions(provider: Option<&str>) -> Vec<CompletionItem> {
    struct TypeEntry {
        label: &'static str,
        desc: &'static str,
        providers: &'static [&'static str],
    }
    let type_entries = [
        TypeEntry {
            label: "type: BTree",
            desc: "B-Tree index — default on all databases",
            providers: &["postgresql", "mysql", "sqlite"],
        },
        TypeEntry {
            label: "type: Hash",
            desc: "Hash index — PostgreSQL and MySQL 8+",
            providers: &["postgresql", "mysql"],
        },
        TypeEntry {
            label: "type: Gin",
            desc: "GIN index — PostgreSQL only (arrays, JSONB, full-text)",
            providers: &["postgresql"],
        },
        TypeEntry {
            label: "type: Gist",
            desc: "GiST index — PostgreSQL only (geometry, range types)",
            providers: &["postgresql"],
        },
        TypeEntry {
            label: "type: Brin",
            desc: "BRIN index — PostgreSQL only (ordered large tables)",
            providers: &["postgresql"],
        },
        TypeEntry {
            label: "type: Hnsw",
            desc: "pgvector HNSW index — PostgreSQL only",
            providers: &["postgresql"],
        },
        TypeEntry {
            label: "type: Ivfflat",
            desc: "pgvector IVFFlat index — PostgreSQL only",
            providers: &["postgresql"],
        },
        TypeEntry {
            label: "type: FullText",
            desc: "FULLTEXT index — MySQL only",
            providers: &["mysql"],
        },
    ];

    let mut items: Vec<CompletionItem> = type_entries
        .iter()
        .filter(|e| match provider {
            Some(p) => e.providers.contains(&p),
            None => true,
        })
        .map(|e| CompletionItem::new(e.label, CompletionKind::Keyword, Some(e.desc.to_string())))
        .collect();

    items.push(CompletionItem::new(
        "name: \"\"",
        CompletionKind::FieldName,
        Some("Logical developer name for this index".to_string()),
    ));
    if matches!(provider, Some("postgresql") | None) {
        items.push(CompletionItem::new(
            "opclass: vector_l2_ops",
            CompletionKind::Keyword,
            Some("pgvector operator class for Hnsw/Ivfflat indexes".to_string()),
        ));
        items.push(CompletionItem::new(
            "m: 16",
            CompletionKind::Keyword,
            Some("pgvector HNSW graph connectivity parameter".to_string()),
        ));
        items.push(CompletionItem::new(
            "ef_construction: 64",
            CompletionKind::Keyword,
            Some("pgvector HNSW build parameter".to_string()),
        ));
        items.push(CompletionItem::new(
            "lists: 100",
            CompletionKind::Keyword,
            Some("pgvector IVFFlat inverted-list count".to_string()),
        ));
    }
    items.push(CompletionItem::new(
        "map: \"\"",
        CompletionKind::FieldName,
        Some("Physical DDL index name (overrides auto-generated idx_… name)".to_string()),
    ));
    if matches!(provider, Some("postgresql") | Some("sqlite") | None) {
        items.push(CompletionItem::new(
            "where: ",
            CompletionKind::Keyword,
            Some("Partial-index predicate — index only the matching rows".to_string()),
        ));
    }

    items
}

pub(super) fn datasource_extension_value_completions(
    tokens: &[Token],
    offset: usize,
    provider: Option<&str>,
) -> Vec<CompletionItem> {
    if matches!(provider, Some("mysql") | Some("sqlite")) {
        return Vec::new();
    }

    match extensions_value_mode_at(tokens, offset) {
        ExtensionsValueMode::StartOfValue => vec![CompletionItem::with_snippet(
            "extensions = [..]",
            "[${1:pg_trgm}]",
            CompletionKind::Keyword,
            Some("PostgreSQL-only array of extension names".to_string()),
        )],
        ExtensionsValueMode::InsideArray => KNOWN_POSTGRES_EXTENSIONS
            .iter()
            .map(|extension| {
                CompletionItem::with_insert(
                    *extension,
                    render_extension_completion_insert(extension),
                    CompletionKind::Keyword,
                    Some("Known PostgreSQL extension".to_string()),
                )
            })
            .chain(std::iter::once(CompletionItem::with_snippet(
                "extension(name = .., schema = ..)",
                "extension(name = ${1:pg_trgm}, schema = \"${2:public}\")",
                CompletionKind::Keyword,
                Some("Structured extension entry with an explicit target schema".to_string()),
            )))
            .collect(),
        ExtensionsValueMode::None => Vec::new(),
    }
}

pub(super) fn datasource_extension_array_item_completions(
    tokens: &[Token],
    offset: usize,
    provider: Option<&str>,
) -> Option<Vec<CompletionItem>> {
    if matches!(provider, Some("mysql") | Some("sqlite")) {
        return None;
    }

    if extensions_value_mode_at(tokens, offset) != ExtensionsValueMode::InsideArray {
        return None;
    }

    Some(
        KNOWN_POSTGRES_EXTENSIONS
            .iter()
            .map(|extension| {
                CompletionItem::with_insert(
                    *extension,
                    render_extension_completion_insert(extension),
                    CompletionKind::Keyword,
                    Some("Known PostgreSQL extension".to_string()),
                )
            })
            .chain(std::iter::once(CompletionItem::with_snippet(
                "extension(name = .., schema = ..)",
                "extension(name = ${1:pg_trgm}, schema = \"${2:public}\")",
                CompletionKind::Keyword,
                Some("Structured extension entry with an explicit target schema".to_string()),
            )))
            .collect(),
    )
}

fn render_extension_completion_insert(extension: &str) -> String {
    if is_bare_schema_identifier(extension) {
        extension.to_string()
    } else {
        format!("\"{}\"", extension)
    }
}

fn is_bare_schema_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return false;
    }
    matches!(TokenKind::from_ident(name), TokenKind::Ident(_))
}
