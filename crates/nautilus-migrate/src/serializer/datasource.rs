//! Rendering of the `datasource` block: provider, url, declared schemas and
//! PostgreSQL extensions.

use super::escape_schema_string;
use crate::ddl::DatabaseProvider;
use crate::live::LiveSchema;
use nautilus_core::TableName;
use nautilus_schema::TokenKind;

pub(super) fn render_datasource_block(
    live: &LiveSchema,
    provider: DatabaseProvider,
    url: &str,
) -> String {
    let mut fields = vec![
        (
            "provider".to_string(),
            format!("\"{}\"", provider.schema_provider_name()),
        ),
        ("url".to_string(), render_datasource_url(url)),
    ];

    let schemas = declared_schemas(live);
    if !schemas.is_empty() {
        fields.push((
            "schemas".to_string(),
            format!(
                "[{}]",
                schemas
                    .iter()
                    .map(|schema| format!("\"{}\"", escape_schema_string(schema)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    if provider == DatabaseProvider::Postgres && !live.extensions.is_empty() {
        let mut extensions: Vec<(&str, &str)> = live
            .extensions
            .iter()
            .map(|(name, state)| (name.as_str(), state.schema.as_str()))
            .collect();
        extensions.sort_unstable_by(|a, b| a.0.cmp(b.0));
        fields.push((
            "extensions".to_string(),
            format!(
                "[{}]",
                extensions
                    .iter()
                    .map(|(name, schema)| render_extension_entry(name, schema))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let max_key = fields.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let mut lines = vec!["datasource db {".to_string()];
    for (key, value) in fields {
        let padding = max_key - key.len() + 1;
        lines.push(format!("  {}{}= {}", key, " ".repeat(padding), value));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

/// Render the `url` field of a datasource block.
///
/// An `env("NAME")` reference is emitted verbatim so a pulled schema points at
/// the variable instead of embedding a connection string — which carries the
/// password — in a file that usually ends up committed.
fn render_datasource_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("env(") && trimmed.ends_with(')') {
        return trimmed.to_string();
    }
    format!("\"{}\"", escape_schema_string(url))
}

/// The schemas the introspected relations live in, in sorted order.
///
/// Non-empty only when the inspector was given a schema list: a single-schema
/// pull leaves every table unqualified, and emitting `schemas = [...]` for it
/// would force `@@schema` onto a schema that never asked for it.
fn declared_schemas(live: &LiveSchema) -> Vec<&str> {
    let mut schemas: Vec<&str> = live
        .tables
        .keys()
        .chain(live.views.keys())
        .filter_map(TableName::schema)
        .collect();
    schemas.sort_unstable();
    schemas.dedup();
    schemas
}

/// Render a single array entry for the `extensions = [...]` field in a
/// serialized datasource block.
///
/// Round-trips live state back into source: when the extension lives in the
/// default `public` namespace we emit the compact form (`pg_trgm` or
/// `"uuid-ossp"`); any other schema is captured explicitly via the structured
/// `extension(name = ..., schema = "...")` syntax so `db pull` does not
/// silently drop the namespace information.
fn render_extension_entry(name: &str, schema: &str) -> String {
    if schema == "public" {
        render_extension_schema_name(name)
    } else {
        format!(
            "extension(name = {}, schema = \"{}\")",
            render_extension_schema_name(name),
            escape_schema_string(schema)
        )
    }
}

fn render_extension_schema_name(name: &str) -> String {
    if is_bare_schema_identifier(name) {
        name.to_string()
    } else {
        format!("\"{}\"", escape_schema_string(name))
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
