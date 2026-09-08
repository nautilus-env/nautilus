//! The configuration blocks of a schema, and the imports that pull other
//! files into it.

use crate::span::Span;

use super::{Expr, Ident, Literal};

/// An `import` statement pulling another schema file into this one.
///
/// The path is relative to the directory of the file that declares it and names
/// either a `.nautilus` file or a directory of them.  Importing is how a schema
/// spread across files declares its own extent: nothing is joined to a file
/// unless that file, or one it imports, asks for it.
///
/// # Example
///
/// ```text
/// import "./enums.nautilus"
/// import "../shared"
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    /// The path exactly as written, without the surrounding quotes.
    pub path: String,
    /// Span of the quoted path literal.
    pub path_span: Span,
    /// Span covering the whole statement.
    pub span: Span,
}

/// A datasource block declaration.
///
/// # Example
///
/// ```prisma
/// datasource db {
///   provider = "postgresql"
///   url      = env("DATABASE_URL")
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DatasourceDecl {
    /// The name of the datasource (e.g., "db").
    pub name: Ident,
    /// Configuration fields (key-value pairs).
    pub fields: Vec<ConfigField>,
    /// Span covering the entire datasource block.
    pub span: Span,
}

impl DatasourceDecl {
    /// Finds a configuration field by name.
    pub fn find_field(&self, name: &str) -> Option<&ConfigField> {
        self.fields.iter().find(|f| f.name.value == name)
    }

    /// Gets the provider value if present.
    pub fn provider(&self) -> Option<&str> {
        self.find_field("provider").and_then(|f| match &f.value {
            Expr::Literal(Literal::String(s, _)) => Some(s.as_str()),
            _ => None,
        })
    }

    /// Gets the declared PostgreSQL extensions (best-effort).
    ///
    /// Accepts both identifiers (`pg_trgm`) and string literals (`"uuid-ossp"`)
    /// as array elements. Returns `None` when the `extensions` field is absent,
    /// and an empty vec when it is present but empty or malformed. Rigorous
    /// validation (including error reporting) happens in the validator.
    pub fn extensions(&self) -> Option<Vec<String>> {
        let field = self.find_field("extensions")?;
        let elements = match &field.value {
            Expr::Array { elements, .. } => elements,
            _ => return Some(Vec::new()),
        };
        Some(
            elements
                .iter()
                .filter_map(|e| match e {
                    Expr::Ident(ident) => Some(ident.value.clone()),
                    Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
        )
    }
}

/// A generator block declaration.
///
/// # Example
///
/// ```prisma
/// generator client {
///   provider = "nautilus-client-rs"
///   output   = "../generated"
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorDecl {
    /// The name of the generator (e.g., "client").
    pub name: Ident,
    /// Configuration fields (key-value pairs).
    pub fields: Vec<ConfigField>,
    /// Span covering the entire generator block.
    pub span: Span,
}

impl GeneratorDecl {
    /// Finds a configuration field by name.
    pub fn find_field(&self, name: &str) -> Option<&ConfigField> {
        self.fields.iter().find(|f| f.name.value == name)
    }
}

/// A configuration field in a datasource or generator block.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigField {
    /// The field name.
    pub name: Ident,
    /// The field value (typically a string or function call).
    pub value: Expr,
    /// Span covering the entire field declaration.
    pub span: Span,
}
