//! Hover documentation for `.nautilus` schema files.

use super::{analyze, catalog, span_contains, AnalysisResult};
use crate::ast::{
    ComputedKind, Declaration, FieldAttribute, FieldModifier, FieldType, ModelAttribute, Schema,
};
use crate::span::Span;
use crate::token::{Token, TokenKind};

/// Information to display when hovering over a token.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverInfo {
    /// Markdown-formatted documentation string.
    pub content: String,
    /// Span of the token the hover applies to (for range highlighting).
    pub span: Option<Span>,
}

/// Returns hover documentation for the symbol at byte `offset` in `source`.
///
/// Looks up the innermost AST node whose span contains `offset` and returns
/// relevant documentation:
/// - Scalar types -> SQL mapping and description.
/// - Identifiers matching a model name -> model summary.
/// - Identifiers matching an enum name -> enum variant list.
/// - Field declarations -> field type and modifiers.
pub fn hover(source: &str, offset: usize) -> Option<HoverInfo> {
    let result = analyze(source);
    hover_with_analysis(source, &result, offset)
}

/// Returns hover documentation for the symbol at `offset` using a previously
/// computed [`AnalysisResult`].
pub fn hover_with_analysis(
    _source: &str,
    result: &AnalysisResult,
    offset: usize,
) -> Option<HoverInfo> {
    let ast = result.ast.as_ref()?;

    if let Some(h) = attribute_hover_at(&result.tokens, offset, Some(ast)) {
        return Some(h);
    }

    for decl in &ast.declarations {
        if !span_contains(decl.span(), offset) {
            continue;
        }

        match decl {
            Declaration::Import(_) => {}
            Declaration::Model(model) => {
                for field in &model.fields {
                    if span_contains(field.span, offset) {
                        let modifier = match field.modifier {
                            FieldModifier::Array => "[]",
                            FieldModifier::Optional => "?",
                            FieldModifier::NotNull => "!",
                            FieldModifier::None => "",
                        };
                        let type_str =
                            format!("{}{}", field_type_name(&field.field_type), modifier);

                        if field.has_relation_attribute() {
                            let base = format!("**{}**: `{}`", field.name.value, type_str);
                            let extra = relation_hover_details(ast, offset).unwrap_or_default();
                            let content = if extra.is_empty() {
                                base
                            } else {
                                format!("{base}  \n\n{extra}")
                            };
                            return Some(HoverInfo {
                                content,
                                span: Some(field.span),
                            });
                        }

                        let attrs_str = format_field_attrs_short(&field.attributes);
                        let detail = field_type_description(&field.field_type);
                        let nullability = match field.modifier {
                            FieldModifier::Optional => Some("nullable"),
                            FieldModifier::NotNull => Some("not null"),
                            _ => None,
                        };
                        let mut content = format!("**{}**: `{}`", field.name.value, type_str);
                        if !attrs_str.is_empty() {
                            content.push_str(&format!("  \n{}", attrs_str));
                        }
                        if let Some(hint) = nullability {
                            content.push_str(&format!("  \n_{}_", hint));
                        }
                        if !detail.is_empty() {
                            content.push_str(&format!("  \n{}", detail));
                        }
                        return Some(HoverInfo {
                            content,
                            span: Some(field.span),
                        });
                    }
                }
                let composite_names: std::collections::HashSet<String> =
                    ast.types().map(|t| t.name.value.clone()).collect();
                return Some(HoverInfo {
                    content: model_hover_content(model, &composite_names),
                    span: Some(model.span),
                });
            }

            Declaration::Enum(enum_decl) => {
                let variants: Vec<&str> = enum_decl
                    .variants
                    .iter()
                    .map(|v| v.name.value.as_str())
                    .collect();
                return Some(HoverInfo {
                    content: format!(
                        "**enum** `{}`  \n**Variants ({}):** {}  \n",
                        enum_decl.name.value,
                        variants.len(),
                        variants
                            .iter()
                            .map(|v| format!("`{v}`"))
                            .collect::<Vec<_>>()
                            .join(" · ")
                    ),
                    span: Some(enum_decl.span),
                });
            }

            Declaration::Datasource(ds) => {
                for field in &ds.fields {
                    if span_contains(field.span, offset) {
                        return Some(HoverInfo {
                            content: config_field_hover(&field.name.value),
                            span: Some(field.span),
                        });
                    }
                }
                return Some(HoverInfo {
                    content: format!("**datasource** `{}`", ds.name.value),
                    span: Some(ds.span),
                });
            }

            Declaration::Generator(gen) => {
                for field in &gen.fields {
                    if span_contains(field.span, offset) {
                        return Some(HoverInfo {
                            content: config_field_hover(&field.name.value),
                            span: Some(field.span),
                        });
                    }
                }
                return Some(HoverInfo {
                    content: format!("**generator** `{}`", gen.name.value),
                    span: Some(gen.span),
                });
            }

            Declaration::Type(type_decl) => {
                for field in &type_decl.fields {
                    if span_contains(field.span, offset) {
                        let modifier = match field.modifier {
                            FieldModifier::Array => "[]",
                            FieldModifier::Optional => "?",
                            FieldModifier::NotNull => "!",
                            FieldModifier::None => "",
                        };
                        let type_str =
                            format!("{}{}", field_type_name(&field.field_type), modifier);
                        let attrs_str = format_field_attrs_short(&field.attributes);
                        let mut content = format!("**{}**: `{}`", field.name.value, type_str);
                        if !attrs_str.is_empty() {
                            content.push_str(&format!("  \n{}", attrs_str));
                        }
                        return Some(HoverInfo {
                            content,
                            span: Some(field.span),
                        });
                    }
                }
                return Some(HoverInfo {
                    content: composite_type_hover_content(type_decl),
                    span: Some(type_decl.span),
                });
            }
        }
    }

    None
}

/// Hover documentation for datasource/generator config fields.
pub fn config_field_hover(key: &str) -> String {
    match catalog::config_field(key) {
        Some(doc) => doc.documentation(),
        None => format!("**{key}**"),
    }
}

/// Returns hover info if `offset` falls on a `@attr` or `@@attr` token
/// (including its parenthesised argument list, if any).
fn attribute_hover_at(tokens: &[Token], offset: usize, ast: Option<&Schema>) -> Option<HoverInfo> {
    let n = tokens.len();
    let mut i = 0;
    while i < n {
        let tok = &tokens[i];
        let is_double = tok.kind == TokenKind::AtAt;
        let is_single = tok.kind == TokenKind::At;
        if !is_double && !is_single {
            i += 1;
            continue;
        }

        let ident_i = match (i + 1..n).find(|&j| !matches!(tokens[j].kind, TokenKind::Newline)) {
            Some(j) => j,
            None => {
                i += 1;
                continue;
            }
        };
        let ident_tok = &tokens[ident_i];
        let attr_name = match &ident_tok.kind {
            TokenKind::Ident(name) => name.clone(),
            _ => {
                i += 1;
                continue;
            }
        };

        let attr_start = tok.span.start;
        let attr_name_end = ident_tok.span.end;

        let lparen_i = (ident_i + 1..n).find(|&j| !matches!(tokens[j].kind, TokenKind::Newline));
        let full_end = if lparen_i.map(|j| &tokens[j].kind) == Some(&TokenKind::LParen) {
            find_paren_end(tokens, lparen_i.unwrap()).unwrap_or(attr_name_end)
        } else {
            attr_name_end
        };

        if offset >= attr_start && offset <= full_end {
            let content = if is_double {
                model_attr_hover_text(&attr_name)
            } else {
                field_attr_hover_text(&attr_name, ast, offset)
            };
            return Some(HoverInfo {
                content,
                span: Some(Span {
                    start: attr_start,
                    end: attr_name_end,
                }),
            });
        }
        i += 1;
    }
    None
}

/// Walk tokens from the `(` at `lparen_idx` and return the byte-end of the
/// matching `)`.
fn find_paren_end(tokens: &[Token], lparen_idx: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    for tok in &tokens[lparen_idx..] {
        match tok.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(tok.span.end);
                }
            }
            _ => {}
        }
    }
    None
}

fn field_attr_hover_text(name: &str, ast: Option<&Schema>, offset: usize) -> String {
    let Some(doc) = catalog::field_attribute(name) else {
        return format!("**@{name}**");
    };
    let text = doc.documentation();

    // The relation attribute says more once the schema around it is known.
    if name == "relation" {
        if let Some(schema) = ast {
            if let Some(extra) = relation_hover_details(schema, offset) {
                return format!(
                    "{text}  

{extra}"
                );
            }
        }
    }
    text
}

fn model_attr_hover_text(name: &str) -> String {
    match catalog::model_attribute(name) {
        Some(doc) => doc.documentation(),
        None => format!("**@@{name}**"),
    }
}

/// Extracts a rich Markdown summary of the `@relation(...)` attribute on the
/// field whose span contains `offset`.
///
/// Shows:
/// - Inferred relation type (one-to-many / one-to-one)
/// - `ParentModel -> TargetType` arrow
/// - All explicit arguments: name, fields, references, onDelete, onUpdate
fn relation_hover_details(ast: &Schema, offset: usize) -> Option<String> {
    for decl in &ast.declarations {
        if let Declaration::Model(model) = decl {
            for field in &model.fields {
                if !span_contains(field.span, offset) {
                    continue;
                }
                for attr in &field.attributes {
                    if let FieldAttribute::Relation {
                        name,
                        fields,
                        references,
                        on_delete,
                        on_update,
                        ..
                    } = attr
                    {
                        let target = field_type_name(&field.field_type);
                        let modifier_str = match field.modifier {
                            FieldModifier::Array => "[]",
                            FieldModifier::Optional => "?",
                            FieldModifier::NotNull => "!",
                            FieldModifier::None => "",
                        };
                        let relation_kind = match field.modifier {
                            FieldModifier::Array => "one-to-many",
                            _ if fields.is_some() => "one-to-many",
                            _ => "one-to-one",
                        };

                        let mut lines: Vec<String> = vec![format!(
                            "**Type:** `{relation_kind}`  ·  `{}` -> `{target}{modifier_str}`",
                            model.name.value
                        )];

                        let has_args = name.is_some()
                            || fields.is_some()
                            || references.is_some()
                            || on_delete.is_some()
                            || on_update.is_some();

                        if has_args {
                            lines.push(String::new());
                            if let Some(n) = name {
                                lines.push(format!("- **name**: `\"{n}\"` "));
                            }
                            if let Some(fs) = fields {
                                let names: Vec<&str> =
                                    fs.iter().map(|f| f.value.as_str()).collect();
                                lines.push(format!("- **fields**: `[{}]`", names.join(", ")));
                            }
                            if let Some(rs) = references {
                                let names: Vec<&str> =
                                    rs.iter().map(|r| r.value.as_str()).collect();
                                lines.push(format!("- **references**: `[{}]`", names.join(", ")));
                            }
                            if let Some(od) = on_delete {
                                lines.push(format!("- **onDelete**: `{od}`"));
                            }
                            if let Some(ou) = on_update {
                                lines.push(format!("- **onUpdate**: `{ou}`"));
                            }
                        }

                        return Some(lines.join("  \n"));
                    }
                }
            }
        }
    }
    None
}

/// Formats field-level attributes as an inline string, e.g.
/// `@id · @default(uuid()) · @map("user_id")`.
/// `@relation` is omitted — it has its own dedicated hover.
fn format_field_attrs_short(attrs: &[FieldAttribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| match attr {
            FieldAttribute::Id => Some("@id".to_string()),
            FieldAttribute::Unique => Some("@unique".to_string()),
            FieldAttribute::Ignore { .. } => Some("@ignore".to_string()),
            FieldAttribute::Default(expr, _) => {
                Some(format!("@default({})", crate::formatter::format_expr(expr)))
            }
            FieldAttribute::Map(name) => Some(format!("@map(\"{}\")", name)),
            FieldAttribute::Store { .. } => Some("@store(json)".to_string()),
            FieldAttribute::UpdatedAt { .. } => Some("@updatedAt".to_string()),
            FieldAttribute::Computed { expr, kind, .. } => {
                let kind_str = match kind {
                    ComputedKind::Stored => "Stored",
                    ComputedKind::Virtual => "Virtual",
                };
                Some(format!("@computed({}, {})", expr, kind_str))
            }
            FieldAttribute::Check { expr, .. } => Some(format!("@check({})", expr)),
            FieldAttribute::Relation { .. } => None,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Builds the full Markdown hover content for a `model` declaration.
fn model_hover_content(
    model: &crate::ast::ModelDecl,
    composite_names: &std::collections::HashSet<String>,
) -> String {
    let table_name = model.table_name();
    let mut lines: Vec<String> = vec![format!("**model** `{}`", model.name.value)];

    if table_name != model.name.value {
        lines.push(format!("**Table:** `{}`", table_name));
    }

    let composite_count = model
        .fields
        .iter()
        .filter(|f| matches!(&f.field_type, FieldType::UserType(n) if composite_names.contains(n)))
        .count();
    let relation_count = model
        .fields
        .iter()
        .filter(|f| matches!(&f.field_type, FieldType::UserType(n) if !composite_names.contains(n)))
        .count();
    let scalar_count = model.fields.len() - composite_count - relation_count;
    let mut count_parts = vec![format!("{} scalar", scalar_count)];
    if relation_count > 0 {
        count_parts.push(format!("{} relation", relation_count));
    }
    if composite_count > 0 {
        count_parts.push(format!("{} composite", composite_count));
    }
    lines.push(format!("**Fields:** {}", count_parts.join(" · ")));
    lines.push(String::new());

    for field in &model.fields {
        let modifier = match field.modifier {
            FieldModifier::Array => "[]",
            FieldModifier::Optional => "?",
            FieldModifier::NotNull => "!",
            FieldModifier::None => "",
        };
        let type_str = format!("{}{}", field_type_name(&field.field_type), modifier);
        let attrs_str = format_field_attrs_short(&field.attributes);
        if attrs_str.is_empty() {
            lines.push(format!("- `{}`: `{}`", field.name.value, type_str));
        } else {
            lines.push(format!(
                "- `{}`: `{}`  — {}",
                field.name.value, type_str, attrs_str
            ));
        }
    }

    let extra_attrs: Vec<String> = model
        .attributes
        .iter()
        .filter_map(|attr| match attr {
            ModelAttribute::Map(_) => None,
            ModelAttribute::Id(fields) => {
                let fs: Vec<&str> = fields.iter().map(|f| f.value.as_str()).collect();
                Some(format!("_@@id([{}])_", fs.join(", ")))
            }
            ModelAttribute::Unique(fields) => {
                let fs: Vec<&str> = fields.iter().map(|f| f.value.as_str()).collect();
                Some(format!("_@@unique([{}])_", fs.join(", ")))
            }
            ModelAttribute::Index {
                fields,
                index_type,
                opclass,
                m,
                ef_construction,
                lists,
                name,
                map,
                predicate,
                ..
            } => {
                let fs: Vec<&str> = fields.iter().map(|f| f.value.as_str()).collect();
                let mut parts = vec![format!("[{}]", fs.join(", "))];
                if let Some(t) = index_type {
                    parts.push(format!("type: {}", t.value));
                }
                if let Some(opclass) = opclass {
                    parts.push(format!("opclass: {}", opclass.value));
                }
                if let Some(m) = m {
                    parts.push(format!("m: {}", m));
                }
                if let Some(ef_construction) = ef_construction {
                    parts.push(format!("ef_construction: {}", ef_construction));
                }
                if let Some(lists) = lists {
                    parts.push(format!("lists: {}", lists));
                }
                if let Some(n) = name {
                    parts.push(format!("name: \"{}\"", n));
                }
                if let Some(m) = map {
                    parts.push(format!("map: \"{}\"", m));
                }
                if let Some(predicate) = predicate {
                    parts.push(format!("where: {}", predicate));
                }
                Some(format!("_@@index({})_", parts.join(", ")))
            }
            ModelAttribute::Check { expr, .. } => Some(format!("_@@check({})_", expr)),
            ModelAttribute::Ignore { .. } => Some("_@@ignore_".to_string()),
            ModelAttribute::Schema { name, .. } => Some(format!("_@@schema(\"{}\")_", name)),
        })
        .collect();

    if !extra_attrs.is_empty() {
        lines.push(String::new());
        lines.extend(extra_attrs);
    }

    lines.join("  \n")
}

/// Builds the full Markdown hover content for a `type` declaration.
fn composite_type_hover_content(type_decl: &crate::ast::TypeDecl) -> String {
    let mut lines: Vec<String> = vec![format!("**type** `{}`", type_decl.name.value)];
    if let Some(mapped) = type_decl.mapped_name() {
        lines.push(format!("**SQL type:** `{}`", mapped));
    }
    lines.push(format!("**Fields:** {}", type_decl.fields.len()));
    lines.push(String::new());

    for field in &type_decl.fields {
        let modifier = match field.modifier {
            FieldModifier::Array => "[]",
            FieldModifier::Optional => "?",
            FieldModifier::NotNull => "!",
            FieldModifier::None => "",
        };
        let type_str = format!("{}{}", field_type_name(&field.field_type), modifier);
        let attrs_str = format_field_attrs_short(&field.attributes);
        if attrs_str.is_empty() {
            lines.push(format!("- `{}`: `{}`", field.name.value, type_str));
        } else {
            lines.push(format!(
                "- `{}`: `{}`  — {}",
                field.name.value, type_str, attrs_str
            ));
        }
    }

    lines.join("  \n")
}

fn field_type_name(ft: &FieldType) -> String {
    match ft {
        FieldType::String => "String".to_string(),
        FieldType::Boolean => "Boolean".to_string(),
        FieldType::Int => "Int".to_string(),
        FieldType::BigInt => "BigInt".to_string(),
        FieldType::Float => "Float".to_string(),
        FieldType::Decimal { precision, scale } => format!("Decimal({}, {})", precision, scale),
        FieldType::DateTime => "DateTime".to_string(),
        FieldType::Bytes => "Bytes".to_string(),
        FieldType::Json => "Json".to_string(),
        FieldType::Uuid => "Uuid".to_string(),
        FieldType::Citext => "Citext".to_string(),
        FieldType::Hstore => "Hstore".to_string(),
        FieldType::Ltree => "Ltree".to_string(),
        FieldType::Geometry => "Geometry".to_string(),
        FieldType::Geography => "Geography".to_string(),
        FieldType::Vector { dimension } => format!("Vector({})", dimension),
        FieldType::Jsonb => "Jsonb".to_string(),
        FieldType::Xml => "Xml".to_string(),
        FieldType::Char { length } => format!("Char({})", length),
        FieldType::VarChar { length } => format!("VarChar({})", length),
        FieldType::UserType(name) => name.clone(),
    }
}

fn field_type_description(ft: &FieldType) -> String {
    match catalog::scalar_doc(ft) {
        Some(doc) => doc.documentation(),
        None => "Reference to another model or enum.".to_string(),
    }
}
