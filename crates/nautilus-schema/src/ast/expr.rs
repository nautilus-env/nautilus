//! Identifiers and the expressions that appear inside attribute arguments.

use std::fmt;

use crate::span::Span;

/// An identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    /// The identifier value.
    pub value: String,
    /// Span of the identifier.
    pub span: Span,
}

impl Ident {
    /// Creates a new identifier.
    pub fn new(value: String, span: Span) -> Self {
        Self { value, span }
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

/// An expression (used in attribute arguments).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal value.
    Literal(Literal),
    /// A function call: `name(arg1, arg2, ...)`.
    FunctionCall {
        /// Function name.
        name: Ident,
        /// Arguments.
        args: Vec<Expr>,
        /// Span of the entire call.
        span: Span,
    },
    /// An array: `[item1, item2, ...]`.
    Array {
        /// Array elements.
        elements: Vec<Expr>,
        /// Span of the entire array.
        span: Span,
    },
    /// A named argument: `name: value`.
    NamedArg {
        /// Argument name.
        name: Ident,
        /// Argument value.
        value: Box<Expr>,
        /// Span of the entire named argument.
        span: Span,
    },
    /// An identifier reference.
    Ident(Ident),
}

impl Expr {
    /// Returns the span of this expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Literal(lit) => lit.span(),
            Expr::FunctionCall { span, .. } => *span,
            Expr::Array { span, .. } => *span,
            Expr::NamedArg { span, .. } => *span,
            Expr::Ident(ident) => ident.span,
        }
    }
}

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// String literal.
    String(String, Span),
    /// Number literal (stored as string, can be int or float).
    Number(String, Span),
    /// Boolean literal.
    Boolean(bool, Span),
}

impl Literal {
    /// Returns the span of this literal.
    pub fn span(&self) -> Span {
        match self {
            Literal::String(_, span) => *span,
            Literal::Number(_, span) => *span,
            Literal::Boolean(_, span) => *span,
        }
    }
}
