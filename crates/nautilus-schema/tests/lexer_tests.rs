//! Integration tests for the schema lexer.
//!
//! Per-token unit tests (keywords, punctuation, literals, comments, errors) live
//! inline in `src/lexer.rs`.  This file covers:
//!   - Full-schema tokenization (presence of all token categories together)
//!   - Span accuracy
//!   - Whitespace normalisation
//!   - Method-call syntax
//!   - Unterminated block-comment error path

use nautilus_schema::{Lexer, Result, SchemaError, TokenKind};

/// Helper to collect all tokens from a source string, skipping newlines.
fn tokenize(source: &str) -> Result<Vec<TokenKind>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();

    loop {
        let token = lexer.next_token()?;
        if matches!(token.kind, TokenKind::Eof) {
            break;
        }
        if !matches!(token.kind, TokenKind::Newline) {
            tokens.push(token.kind);
        }
    }

    Ok(tokens)
}

#[test]
fn test_simple_schema() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

generator client {
  provider = "nautilus-client-rs"
  output   = "../crates/nautilus-connector/src/generated"
}

enum Role {
  USER
  ADMIN
}

model User {
  id        Uuid     @id @default(uuid()) @map("user_id")
  email     String   @unique
  role      Role     @default(USER)
  createdAt DateTime @default(now()) @map("created_at")

  posts     Post[]

  @@map("users")
}

model Post {
  id        BigInt   @id @default(autoincrement())
  userId    Uuid     @map("user_id")
  title     String
  rating    Decimal(10, 2)
  createdAt DateTime @default(now()) @map("created_at")

  user      User     @relation(fields: [userId], references: [id], onUpdate: Cascade, onDelete: Cascade)

  @@map("posts")
}
"#;

    let result = tokenize(source);
    assert!(
        result.is_ok(),
        "Failed to tokenize schema: {:?}",
        result.err()
    );

    let tokens = result.unwrap();

    assert!(tokens.contains(&TokenKind::Datasource));
    assert!(tokens.contains(&TokenKind::Generator));
    assert!(tokens.contains(&TokenKind::Enum));
    assert!(tokens.contains(&TokenKind::Model));

    assert!(tokens.contains(&TokenKind::Ident("User".to_string())));
    assert!(tokens.contains(&TokenKind::Ident("Post".to_string())));
    assert!(tokens.contains(&TokenKind::Ident("Role".to_string())));

    assert!(tokens.contains(&TokenKind::String("postgresql".to_string())));
    assert!(tokens.contains(&TokenKind::String("user_id".to_string())));
}

#[test]
fn test_error_unterminated_block_comment() {
    let source = r#"/* unterminated comment
model User { }"#;

    let result = tokenize(source);
    assert!(result.is_err());

    match result.unwrap_err() {
        SchemaError::Lexer(msg, _) if msg.contains("Unterminated") => {}
        e => panic!("Expected unterminated block comment error, got {:?}", e),
    }
}

#[test]
fn test_span_accuracy() {
    let source = "model User";
    let mut lexer = Lexer::new(source);

    let token1 = lexer.next_token().unwrap();
    assert_eq!(token1.kind, TokenKind::Model);
    assert_eq!(token1.span.slice(source), "model");

    let token2 = lexer.next_token().unwrap();
    assert_eq!(token2.kind, TokenKind::Ident("User".to_string()));
    assert_eq!(token2.span.slice(source), "User");
}

#[test]
fn test_whitespace_handling() {
    let source = "model    User  {  id  Int  }";
    let tokens = tokenize(source).unwrap();

    assert_eq!(tokens[0], TokenKind::Model);
    assert_eq!(tokens[1], TokenKind::Ident("User".to_string()));
    assert_eq!(tokens[2], TokenKind::LBrace);
    assert_eq!(tokens[3], TokenKind::Ident("id".to_string()));
    assert_eq!(tokens[4], TokenKind::Ident("Int".to_string()));
    assert_eq!(tokens[5], TokenKind::RBrace);
}

#[test]
fn test_method_call_syntax() {
    let source = r#"default(now())"#;

    let tokens = tokenize(source).unwrap();

    assert_eq!(tokens[0], TokenKind::Ident("default".to_string()));
    assert_eq!(tokens[1], TokenKind::LParen);
    assert_eq!(tokens[2], TokenKind::Ident("now".to_string()));
    assert_eq!(tokens[3], TokenKind::LParen);
    assert_eq!(tokens[4], TokenKind::RParen);
    assert_eq!(tokens[5], TokenKind::RParen);
}
