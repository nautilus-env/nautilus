//! Where the cursor is, read from the token stream.
//!
//! Completion has to answer inside a file that is being typed and often does
//! not parse, so the context comes from the tokens around the offset rather
//! than from the AST: which declaration encloses it, whether it sits after an
//! `@`, inside an attribute's arguments, or in the value of a configuration
//! key.

use crate::token::{Token, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigBlockKind {
    Datasource,
    Generator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttributeContext {
    FieldAttr,
    ModelAttr,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclarationContext {
    Model,
    Type,
    Other,
}

/// Returns the name of the attribute whose argument list contains `offset`,
/// e.g. `"store"` when the cursor is inside `@store(|)`, `"relation"` for
/// `@relation(|)`, etc.  Returns `None` when `offset` is not inside any
/// attribute argument list.
pub(super) fn inside_attr_args_at(tokens: &[Token], offset: usize) -> Option<String> {
    let relevant: Vec<&Token> = tokens
        .iter()
        .filter(|t| t.span.end <= offset && !matches!(t.kind, TokenKind::Newline))
        .collect();

    let mut depth: i32 = 0;
    for tok in relevant.iter().rev() {
        match tok.kind {
            TokenKind::RParen => depth += 1,
            TokenKind::LParen => {
                if depth == 0 {
                    let lparen_start = tok.span.start;
                    let before: Vec<&Token> = tokens
                        .iter()
                        .filter(|t| {
                            t.span.end <= lparen_start && !matches!(t.kind, TokenKind::Newline)
                        })
                        .collect();
                    if let Some(name_tok) = before.last() {
                        if let TokenKind::Ident(attr_name) = &name_tok.kind {
                            let attr_name = attr_name.clone();
                            let before_name: Vec<&Token> = tokens
                                .iter()
                                .filter(|t| {
                                    t.span.end <= name_tok.span.start
                                        && !matches!(t.kind, TokenKind::Newline)
                                })
                                .collect();
                            if let Some(at_tok) = before_name.last() {
                                if matches!(at_tok.kind, TokenKind::At | TokenKind::AtAt) {
                                    return Some(attr_name);
                                }
                            }
                        }
                    }
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Detects whether `offset` sits in a config value position (`key = <cursor>`)
/// within a datasource or generator block. Returns the key name if so.
pub(super) fn config_value_context_at(tokens: &[Token], offset: usize) -> Option<String> {
    let mut eq_pos: Option<usize> = None;
    let mut key_pos: Option<usize> = None;

    for (i, tok) in tokens.iter().enumerate() {
        if tok.span.end > offset {
            break;
        }
        if tok.kind == TokenKind::Newline {
            eq_pos = None;
            key_pos = None;
        } else if tok.kind == TokenKind::Equal {
            eq_pos = Some(i);
        } else if let TokenKind::Ident(_) = tok.kind {
            if eq_pos.is_none() {
                key_pos = Some(i);
            }
        }
    }

    let eq_idx = eq_pos?;
    let key_idx = key_pos?;

    if eq_idx != key_idx + 1 {
        return None;
    }

    if let TokenKind::Ident(key) = &tokens[key_idx].kind {
        return Some(key.clone());
    }
    None
}

/// Detect whether `offset` immediately follows a `@` or `@@` token.
pub(super) fn attribute_context_at(tokens: &[Token], offset: usize) -> AttributeContext {
    let last = tokens
        .iter()
        .rfind(|t| t.span.end <= offset && !matches!(t.kind, TokenKind::Newline));

    match last {
        Some(t) if t.kind == TokenKind::AtAt => AttributeContext::ModelAttr,
        Some(t) if t.kind == TokenKind::At => AttributeContext::FieldAttr,
        // The cursor might be in the middle of an identifier that started
        // after `@` — look one token further back.
        Some(t) if matches!(t.kind, TokenKind::Ident(_)) => {
            let before = tokens.iter().rfind(|tok| tok.span.end <= t.span.start);
            match before {
                Some(b) if b.kind == TokenKind::AtAt => AttributeContext::ModelAttr,
                Some(b) if b.kind == TokenKind::At => AttributeContext::FieldAttr,
                _ => AttributeContext::None,
            }
        }
        _ => AttributeContext::None,
    }
}

/// Returns the declaration block that appears to enclose `offset`, based
/// purely on the token stream (no AST required).
pub(super) fn declaration_context_at_tokens(tokens: &[Token], offset: usize) -> DeclarationContext {
    let relevant: Vec<&Token> = tokens.iter().filter(|t| t.span.end <= offset).collect();

    let mut depth: i32 = 0;
    for tok in relevant.iter().rev() {
        match tok.kind {
            TokenKind::RBrace => depth += 1,
            TokenKind::LBrace => {
                if depth == 0 {
                    let idx = tokens
                        .iter()
                        .position(|t| std::ptr::eq(t, *tok))
                        .unwrap_or(0);
                    let before: Vec<&Token> = tokens[..idx]
                        .iter()
                        .filter(|t| !matches!(t.kind, TokenKind::Newline))
                        .collect();
                    if let Some(name_tok) = before.last() {
                        if matches!(name_tok.kind, TokenKind::Ident(_)) {
                            let before_name: Vec<&Token> = tokens[..idx]
                                .iter()
                                .filter(|t| !matches!(t.kind, TokenKind::Newline))
                                .rev()
                                .skip(1)
                                .take(1)
                                .collect();
                            if let Some(kw) = before_name.first() {
                                return match kw.kind {
                                    TokenKind::Model => DeclarationContext::Model,
                                    TokenKind::Type => DeclarationContext::Type,
                                    _ => DeclarationContext::Other,
                                };
                            }
                        }
                    }
                    return DeclarationContext::Other;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    DeclarationContext::Other
}

pub(super) fn user_enums_from_tokens(tokens: &[Token]) -> Vec<String> {
    let mut enums = Vec::new();

    for window in tokens.windows(2) {
        if window[0].kind == TokenKind::Enum {
            if let TokenKind::Ident(name) = &window[1].kind {
                enums.push(name.clone());
            }
        }
    }

    enums
}

/// Extract the datasource `provider` value from a token stream.
///
/// Looks for the pattern:  `datasource <ident> { … provider = "<value>" … }`
/// Returns `Some("postgresql" | "mysql" | "sqlite")` when found, `None` otherwise.
pub(super) fn extract_provider_from_tokens(tokens: &[Token]) -> Option<String> {
    let n = tokens.len();
    for i in 0..n {
        if let TokenKind::Ident(ref kw) = tokens[i].kind {
            if kw != "provider" {
                continue;
            }
        } else {
            continue;
        }
        let mut j = i + 1;
        while j < n && matches!(tokens[j].kind, TokenKind::Newline) {
            j += 1;
        }
        if j >= n || tokens[j].kind != TokenKind::Equal {
            continue;
        }
        j += 1;
        while j < n && matches!(tokens[j].kind, TokenKind::Newline) {
            j += 1;
        }
        if j < n {
            if let TokenKind::String(ref val) = tokens[j].kind {
                let v = val.as_str();
                if matches!(v, "postgresql" | "mysql" | "sqlite") {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Returns context-sensitive completions for the arguments of a specific attribute.
///
/// Called when the cursor is detected to be inside `@attr(|)` argument parens.
/// Returns the 0-based argument index of `offset` inside the innermost
/// unmatched `(...)`, scanning backwards through `tokens`.
/// Returns `None` if not inside any parentheses.
pub(super) fn attr_arg_index_at(tokens: &[Token], offset: usize) -> Option<usize> {
    let relevant: Vec<&Token> = tokens
        .iter()
        .filter(|t| t.span.end <= offset && !matches!(t.kind, TokenKind::Newline))
        .collect();
    let mut depth: i32 = 0;
    let mut commas: usize = 0;
    for tok in relevant.iter().rev() {
        match tok.kind {
            TokenKind::RParen => depth += 1,
            TokenKind::LParen => {
                if depth == 0 {
                    return Some(commas);
                }
                depth -= 1;
            }
            TokenKind::Comma if depth == 0 => commas += 1,
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExtensionsValueMode {
    None,
    StartOfValue,
    InsideArray,
}

pub(super) fn extensions_value_mode_at(tokens: &[Token], offset: usize) -> ExtensionsValueMode {
    let mut pending_block_kind: Option<Option<ConfigBlockKind>> = None;
    let mut block_stack: Vec<Option<ConfigBlockKind>> = Vec::new();
    let mut current_field_key: Option<String> = None;
    let mut seen_equal = false;
    let mut saw_value_token_after_equal = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for token in tokens.iter().take_while(|token| token.span.end <= offset) {
        match token.kind {
            TokenKind::Datasource => pending_block_kind = Some(Some(ConfigBlockKind::Datasource)),
            TokenKind::Generator => pending_block_kind = Some(Some(ConfigBlockKind::Generator)),
            TokenKind::Model | TokenKind::Enum | TokenKind::Type => pending_block_kind = Some(None),
            TokenKind::LBrace => {
                block_stack.push(pending_block_kind.take().unwrap_or(None));
                current_field_key = None;
                seen_equal = false;
                saw_value_token_after_equal = false;
                paren_depth = 0;
                bracket_depth = 0;
            }
            TokenKind::RBrace => {
                block_stack.pop();
                current_field_key = None;
                seen_equal = false;
                saw_value_token_after_equal = false;
                paren_depth = 0;
                bracket_depth = 0;
            }
            _ if block_stack.last() != Some(&Some(ConfigBlockKind::Datasource)) => {}
            TokenKind::Newline => {
                if bracket_depth == 0 && paren_depth == 0 {
                    current_field_key = None;
                    seen_equal = false;
                    saw_value_token_after_equal = false;
                }
            }
            TokenKind::Ident(ref name) if current_field_key.is_none() && !seen_equal => {
                current_field_key = Some(name.clone());
            }
            TokenKind::Equal if current_field_key.is_some() => {
                seen_equal = true;
            }
            TokenKind::LParen if seen_equal => {
                saw_value_token_after_equal = true;
                paren_depth += 1;
            }
            TokenKind::RParen if seen_equal && paren_depth > 0 => {
                saw_value_token_after_equal = true;
                paren_depth -= 1;
            }
            TokenKind::LBracket if seen_equal => {
                saw_value_token_after_equal = true;
                bracket_depth += 1;
            }
            TokenKind::RBracket if seen_equal && bracket_depth > 0 => {
                saw_value_token_after_equal = true;
                bracket_depth -= 1;
            }
            _ if seen_equal => {
                saw_value_token_after_equal = true;
            }
            _ => {}
        }
    }

    if current_field_key.as_deref() != Some("extensions") || !seen_equal {
        return ExtensionsValueMode::None;
    }

    if bracket_depth > 0 {
        ExtensionsValueMode::InsideArray
    } else if !saw_value_token_after_equal {
        ExtensionsValueMode::StartOfValue
    } else {
        ExtensionsValueMode::None
    }
}

/// Detects whether `offset` is inside a `datasource` or `generator` block,
/// by scanning the token stream backwards to find the enclosing block keyword.
pub(super) fn config_block_kind_at(tokens: &[Token], offset: usize) -> Option<ConfigBlockKind> {
    let relevant: Vec<&Token> = tokens.iter().filter(|t| t.span.end <= offset).collect();

    let mut depth: i32 = 0;
    for tok in relevant.iter().rev() {
        match tok.kind {
            TokenKind::RBrace => depth += 1,
            TokenKind::LBrace => {
                if depth == 0 {
                    let idx = tokens
                        .iter()
                        .position(|t| std::ptr::eq(t, *tok))
                        .unwrap_or(0);
                    let before: Vec<&Token> = tokens[..idx]
                        .iter()
                        .filter(|t| !matches!(t.kind, TokenKind::Newline))
                        .collect();
                    if before.len() >= 2 {
                        let kw_tok = &before[before.len() - 2];
                        return match kw_tok.kind {
                            TokenKind::Datasource => Some(ConfigBlockKind::Datasource),
                            TokenKind::Generator => Some(ConfigBlockKind::Generator),
                            _ => None,
                        };
                    }
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}
