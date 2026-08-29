//! Integration tests for the schema formatter (canonical AST -> source round-trip).

mod common;

use common::parse_schema;
use nautilus_schema::format_schema;

/// Parse -> format -> re-parse -> format again.  The two formatted strings must be
/// identical (idempotency) and the re-parsed AST must equal the first AST.
fn round_trip(source: &str) -> String {
    let ast1 = parse_schema(source).expect("parse error");
    let formatted1 = format_schema(&ast1, source);

    let ast2 = parse_schema(&formatted1).expect("parse error");
    let formatted2 = format_schema(&ast2, &formatted1);

    assert_eq!(formatted1, formatted2, "format_schema is not idempotent");
    formatted1
}

#[test]
fn test_format_simple_model() {
    let source = r#"model User {
  id   Int    @id
  name String
}"#;
    round_trip(source);
}

#[test]
fn test_format_datasource() {
    let source = r#"datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}"#;
    round_trip(source);
}

#[test]
fn test_format_generator() {
    let source = r#"generator client {
  provider = "nautilus"
  output   = "./generated"
}"#;
    round_trip(source);
}

#[test]
fn test_format_enum() {
    let source = r#"enum Role {
  USER
  ADMIN
  MODERATOR
}"#;
    round_trip(source);
}

#[test]
fn test_format_optional_and_array_fields() {
    let source = r#"model Post {
  id      Int     @id
  title   String
  content String?
  tags    String[]
}"#;
    round_trip(source);
}

#[test]
fn test_format_array_default_literal() {
    let source = r#"model Post {
  id   Int      @id
  tags String[] @default(["TEST","TEST2"])
}"#;
    let out = round_trip(source);
    assert!(out.contains(r#"tags String[] @default(["TEST", "TEST2"])"#));
}

#[test]
fn test_format_relation() {
    let source = r#"model Post {
  id       Int  @id
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}"#;
    round_trip(source);
}

#[test]
fn test_format_unique_and_map() {
    let source = r#"model User {
  id    Int    @id
  email String @unique

  @@map("users")
}"#;
    round_trip(source);
}

#[test]
fn test_format_composite_primary_key() {
    let source = r#"model PostTag {
  postId Int
  tagId  Int

  @@id([postId, tagId])
}"#;
    round_trip(source);
}

#[test]
fn test_format_composite_type_with_map() {
    let source = r#"type Address {
  street String
  zip    String @map("zip_code")

  @@map("address_t")
}"#;
    let formatted = round_trip(source);
    assert!(formatted.contains("@map(\"zip_code\")"));
    assert!(formatted.contains("@@map(\"address_t\")"));
}

#[test]
fn test_format_full_schema() {
    let source = r#"datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

generator client {
  provider = "nautilus"
}

enum Status {
  ACTIVE
  INACTIVE
}

model User {
  id    Int    @id
  name  String
  email String @unique
}

model Post {
  id       Int    @id
  title    String
  authorId Int
  author   User   @relation(fields: [authorId], references: [id])
}"#;
    round_trip(source);
}

#[test]
fn test_format_aligns_datasource_keys() {
    let source = r#"datasource db {
  provider   = "postgresql"
  url        = "postgres://localhost"
  direct_url = "postgres://localhost/direct"
}"#;
    let out = round_trip(source);
    let lines: Vec<&str> = out.lines().filter(|l| l.contains('=')).collect();
    assert_eq!(lines.len(), 3);
    let col0 = lines[0].find('=').unwrap();
    let col1 = lines[1].find('=').unwrap();
    let col2 = lines[2].find('=').unwrap();
    assert_eq!(col0, col1, "= signs should be aligned: {out:?}");
    assert_eq!(col1, col2, "= signs should be aligned: {out:?}");
}

#[test]
fn test_format_model_field_columns_aligned() {
    let source = r#"model User {
  id    Int    @id
  name  String
  email String @unique
}"#;
    let out = round_trip(source);
    for line in out
        .lines()
        .filter(|l| !l.starts_with("model") && !l.starts_with('}') && !l.is_empty())
    {
        assert!(
            line.starts_with("  "),
            "field body should be indented: {line:?}"
        );
    }
}

#[test]
fn test_format_partial_index_round_trips() {
    let source = r#"model Task {
  id      Int     @id
  done    Boolean
  ownerId Int

  @@index([ownerId], where: done = FALSE)
}
"#;
    let formatted = round_trip(source);
    assert!(
        formatted.contains("@@index([ownerId], where: done = FALSE)"),
        "unexpected output:\n{}",
        formatted
    );
}

#[test]
fn test_format_partial_index_with_map_round_trips() {
    let source = r#"model Task {
  id      Int    @id
  status  String
  ownerId Int

  @@index([ownerId], map: "idx_open", where: status IN [OPEN, BLOCKED])
}
"#;
    let formatted = round_trip(source);
    assert!(
        formatted.contains("map: \"idx_open\""),
        "unexpected output:\n{}",
        formatted
    );
    assert!(
        formatted.contains("where: status IN [OPEN, BLOCKED]"),
        "unexpected output:\n{}",
        formatted
    );
}

#[test]
fn test_format_imports_stay_on_adjacent_lines() {
    let source = r#"import   "./enums.nautilus"
import "./post.nautilus"
model User {
  id Int @id
}"#;
    let formatted = round_trip(source);

    assert!(
        formatted
            .starts_with("import \"./enums.nautilus\"\nimport \"./post.nautilus\"\n\nmodel User"),
        "{}",
        formatted
    );
}
