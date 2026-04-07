//! Token types for schema lexing.

use crate::span::Span;
use std::fmt;

/// A token in the schema language.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The kind of token.
    pub kind: TokenKind,
    /// The span of the token in source.
    pub span: Span,
}

impl Token {
    /// Create a new token.
    pub const fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} @ {}", self.kind, self.span)
    }
}

/// Token kinds for the schema language.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// `datasource` keyword.
    Datasource,
    /// `generator` keyword.
    Generator,
    /// `model` keyword.
    Model,
    /// `enum` keyword.
    Enum,
    /// `type` keyword.
    Type,
    /// `true` keyword.
    True,
    /// `false` keyword.
    False,

    /// Identifier (e.g., `User`, `email`, `autoincrement`).
    Ident(String),
    /// String literal (e.g., `"postgresql"`).
    String(String),
    /// Number literal (e.g., `42`, `3.14`).
    Number(String),

    /// `@` symbol (field attribute).
    At,
    /// `@@` symbol (model attribute).
    AtAt,

    /// `{` symbol.
    LBrace,
    /// `}` symbol.
    RBrace,
    /// `[` symbol.
    LBracket,
    /// `]` symbol.
    RBracket,
    /// `(` symbol.
    LParen,
    /// `)` symbol.
    RParen,
    /// `,` symbol.
    Comma,
    /// `:` symbol.
    Colon,
    /// `=` symbol.
    Equal,
    /// `?` symbol (optional field).
    Question,
    /// `!` symbol (not-null field).
    Bang,
    /// `.` symbol (for method calls).
    Dot,

    /// `*` operator.
    Star,
    /// `+` operator.
    Plus,
    /// `-` operator.
    Minus,
    /// `/` operator.
    Slash,
    /// `|` operator.
    Pipe,
    /// `||` operator (SQL string concatenation).
    DoublePipe,
    /// `<` operator.
    LAngle,
    /// `>` operator.
    RAngle,
    /// `%` operator.
    Percent,
    /// `<=` operator.
    LessEqual,
    /// `>=` operator.
    GreaterEqual,
    /// `!=` operator.
    BangEqual,

    /// Newline (significant for statement termination).
    Newline,
    /// End of file.
    Eof,
}

impl TokenKind {
    /// Check if this token is a keyword.
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            TokenKind::Datasource
                | TokenKind::Generator
                | TokenKind::Model
                | TokenKind::Enum
                | TokenKind::Type
                | TokenKind::True
                | TokenKind::False
        )
    }

    /// Try to convert an identifier string to a keyword.
    pub fn from_ident(ident: &str) -> Self {
        match ident {
            "datasource" => TokenKind::Datasource,
            "generator" => TokenKind::Generator,
            "model" => TokenKind::Model,
            "enum" => TokenKind::Enum,
            "type" => TokenKind::Type,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            _ => TokenKind::Ident(ident.to_string()),
        }
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Datasource => write!(f, "datasource"),
            TokenKind::Generator => write!(f, "generator"),
            TokenKind::Model => write!(f, "model"),
            TokenKind::Enum => write!(f, "enum"),
            TokenKind::Type => write!(f, "type"),
            TokenKind::True => write!(f, "true"),
            TokenKind::False => write!(f, "false"),
            TokenKind::Ident(s) => write!(f, "identifier '{}'", s),
            TokenKind::String(s) => write!(f, "string \"{}\"", s),
            TokenKind::Number(s) => write!(f, "number {}", s),
            TokenKind::At => write!(f, "@"),
            TokenKind::AtAt => write!(f, "@@"),
            TokenKind::LBrace => write!(f, "{{"),
            TokenKind::RBrace => write!(f, "}}"),
            TokenKind::LBracket => write!(f, "["),
            TokenKind::RBracket => write!(f, "]"),
            TokenKind::LParen => write!(f, "("),
            TokenKind::RParen => write!(f, ")"),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Equal => write!(f, "="),
            TokenKind::Question => write!(f, "?"),
            TokenKind::Bang => write!(f, "!"),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::DoublePipe => write!(f, "||"),
            TokenKind::LAngle => write!(f, "<"),
            TokenKind::RAngle => write!(f, ">"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::LessEqual => write!(f, "<="),
            TokenKind::GreaterEqual => write!(f, ">="),
            TokenKind::BangEqual => write!(f, "!="),
            TokenKind::Newline => write!(f, "newline"),
            TokenKind::Eof => write!(f, "end of file"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_detection() {
        assert!(TokenKind::Datasource.is_keyword());
        assert!(TokenKind::Model.is_keyword());
        assert!(!TokenKind::Ident("foo".to_string()).is_keyword());
    }

    #[test]
    fn test_from_ident() {
        assert_eq!(TokenKind::from_ident("model"), TokenKind::Model);
        assert_eq!(TokenKind::from_ident("datasource"), TokenKind::Datasource);
        assert_eq!(
            TokenKind::from_ident("User"),
            TokenKind::Ident("User".to_string())
        );
    }

    #[test]
    fn test_token_display() {
        let token = Token::new(TokenKind::Model, Span::new(0, 5));
        let display = token.to_string();
        assert!(display.contains("Model"));
    }
}
