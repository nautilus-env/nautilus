//! Nautilus Schema Parser and Validator
//!
//! This crate provides end-to-end processing of `.nautilus` schema files.
//!
//! # Pipeline
//!
//! Processing a schema runs in four stages:
//! - **Lexer** — converts source text into typed tokens with span tracking.
//! - **Parser** — builds a syntax [`ast::Schema`] via recursive descent.
//! - **Validator** — performs multi-pass semantic validation and emits a fully
//!   resolved [`ir::SchemaIr`].
//! - **Formatter** — renders an AST back to canonical source text.
//!
//! # Quick Start
//!
//! [`analyze`] runs the whole pipeline in one call and collects every
//! diagnostic; [`visitor`] walks an [`ast::Schema`] node by node.  Both
//! modules carry a worked example.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use ast::Schema;
use ir::SchemaIr;
use std::path::{Path, PathBuf};

pub mod analysis;
pub mod ast;
pub mod bool_expr;
pub mod diagnostic;
mod error;
pub mod formatter;
pub mod ir;
mod lexer;
pub mod parser;
mod schema_set;
mod span;
pub mod sql_expr;
mod token;
mod validator;
pub mod visitor;

pub use analysis::{
    analyze, completion, completion_with_analysis, goto_definition, goto_definition_with_analysis,
    hover, hover_with_analysis, semantic_tokens, AnalysisResult, CompletionItem, CompletionKind,
    HoverInfo, SemanticKind, SemanticToken,
};
pub use ast::ComputedKind;
pub use diagnostic::{Diagnostic, Severity};
pub use error::{Result, SchemaError};
pub use formatter::format_schema;
pub use lexer::Lexer;
pub use parser::Parser;
pub use schema_set::SchemaSet;
pub use span::{LineIndex, Position, Span};
pub use token::{Token, TokenKind};
pub use validator::validate_schema;
use validator::validate_schema_ref;

/// Parsed schema plus any non-fatal parse errors recovered during parsing.
#[derive(Debug, Clone)]
pub struct ParsedSchema {
    /// Parsed AST.
    pub ast: Schema,
    /// Errors recovered by the parser while still producing an AST.
    pub recovered_errors: Vec<SchemaError>,
}

/// Parsed AST together with the validated IR.
#[derive(Debug, Clone)]
pub struct ValidatedSchema {
    /// Parsed AST.
    pub ast: Schema,
    /// Fully validated schema IR.
    pub ir: SchemaIr,
}

fn lex_source(source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let is_eof = matches!(token.kind, TokenKind::Eof);
        tokens.push(token);
        if is_eof {
            break;
        }
    }
    Ok(tokens)
}

/// Parse a schema source string and return the AST plus parser recovery errors.
pub fn parse_schema_source_with_recovery(source: &str) -> Result<ParsedSchema> {
    let tokens = lex_source(source)?;
    let mut parser = Parser::new(&tokens, source);
    let ast = parser.parse_schema()?;
    let recovered_errors = parser.take_errors();
    Ok(ParsedSchema {
        ast,
        recovered_errors,
    })
}

/// Parse a schema source string strictly, failing if the parser had to recover.
pub fn parse_schema_source(source: &str) -> Result<Schema> {
    let parsed = parse_schema_source_with_recovery(source)?;
    if let Some(error) = parsed.recovered_errors.into_iter().next() {
        return Err(error);
    }
    Ok(parsed.ast)
}

/// Parse and validate a schema source string.
pub fn validate_schema_source(source: &str) -> Result<ValidatedSchema> {
    let ast = parse_schema_source(source)?;
    let ir = validate_schema_ref(&ast)?;
    Ok(ValidatedSchema { ast, ir })
}

/// Return every `.nautilus` file directly inside `dir`, sorted lexicographically.
pub fn discover_schema_paths(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("nautilus")
        })
        .collect();
    paths.sort();
    Ok(paths)
}

/// Return every `.nautilus` file in the current working directory, sorted lexicographically.
pub fn discover_schema_paths_in_current_dir() -> std::io::Result<Vec<PathBuf>> {
    let current_dir = std::env::current_dir()?;
    discover_schema_paths(&current_dir)
}

/// Resolve `env(VAR_NAME)` syntax in a connection URL against the process environment.
///
/// If `raw` matches the pattern `env(...)`, the value of the named
/// environment variable is returned.  Otherwise `raw` is returned as-is.
pub fn resolve_env_url(raw: &str) -> std::result::Result<String, String> {
    resolve_env_url_with(raw, |var| std::env::var(var).ok())
}

/// Resolve `env(VAR_NAME)` syntax in a connection URL against `lookup`.
///
/// Callers that already hold the values a command was configured with — a
/// `.env` file, a fixture — pass them here instead of going through the
/// process environment.
pub fn resolve_env_url_with(
    raw: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> std::result::Result<String, String> {
    if raw.starts_with("env(") && raw.ends_with(')') {
        let var = &raw[4..raw.len() - 1];
        lookup(var).ok_or_else(|| format!("environment variable '{}' is not set", var))
    } else {
        Ok(raw.to_string())
    }
}
