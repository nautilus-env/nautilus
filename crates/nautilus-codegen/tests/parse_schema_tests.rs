//! Tests for `parse_schema` and `validate_command` entry points in `lib.rs`.

use nautilus_codegen::parse_schema;
use nautilus_schema::validate_schema;

#[test]
fn test_parse_schema_valid_returns_ast() {
    let result = parse_schema(
        r#"
model User {
  id   Int    @id @default(autoincrement())
  name String
}
"#,
    );
    assert!(
        result.is_ok(),
        "expected valid schema to parse successfully: {:?}",
        result
    );
}

#[test]
fn test_parse_schema_empty_source_is_valid() {
    let result = parse_schema("");
    assert!(
        result.is_ok(),
        "empty schema should parse successfully: {:?}",
        result
    );
}

#[test]
fn test_parse_schema_model_with_all_scalar_types() {
    let result = parse_schema(
        r#"
model AllScalars {
  id        Int      @id @default(autoincrement())
  text      String
  big       BigInt
  num       Float
  flag      Boolean
  created   DateTime @default(now())
  uid       Uuid     @default(uuid())
  money     Decimal(10, 2)
  data      Bytes
  meta      Json
}
"#,
    );
    assert!(
        result.is_ok(),
        "all-scalar model schema should parse: {:?}",
        result
    );
}

#[test]
fn test_parse_schema_enum_declaration() {
    let result = parse_schema(
        r#"
enum Color {
  RED
  GREEN
  BLUE
}

model Widget {
  id    Int   @id @default(autoincrement())
  color Color
}
"#,
    );
    assert!(result.is_ok(), "enum schema should parse: {:?}", result);
}

/// An unknown scalar type should cause validation to fail.
#[test]
fn test_validate_schema_unknown_field_type_fails() {
    let ast = parse_schema(
        r#"
model User {
  id   Int         @id @default(autoincrement())
  name NoSuchType
}
"#,
    )
    .expect("parse step should succeed even with unknown type");
    let result = validate_schema(ast);
    assert!(
        result.is_err(),
        "expected validation to fail for unknown scalar type 'NoSuchType'"
    );
}

/// A model with no `@id` field is currently accepted by the validator
/// (composite PKs or table-less projections may not need a single `@id`).
#[test]
fn test_validate_schema_model_without_id_is_accepted() {
    let ast = parse_schema(
        r#"
model Item {
  name  String
  value Int
}
"#,
    )
    .expect("parse step should succeed");
    let result = validate_schema(ast);
    assert!(
        result.is_ok(),
        "validator should accept a model with no @id field: {:?}",
        result
    );
}

/// A schema with a valid datasource block should parse and validate.
#[test]
fn test_validate_schema_with_datasource() {
    let ast = parse_schema(
        r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

model User {
  id   Int    @id @default(autoincrement())
  name String
}
"#,
    )
    .expect("parse failed");
    let result = validate_schema(ast);
    assert!(
        result.is_ok(),
        "schema with datasource should validate: {:?}",
        result
    );
}

/// A schema with two models and a valid bidirectional relation should validate.
#[test]
fn test_validate_schema_relation_is_valid() {
    let ast = parse_schema(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  posts Post[]
}

model Post {
  id       Int    @id @default(autoincrement())
  authorId Int
  author   User   @relation(fields: [authorId], references: [id])
}
"#,
    )
    .expect("parse failed");
    let result = validate_schema(ast);
    assert!(
        result.is_ok(),
        "valid relation schema should pass validation: {:?}",
        result
    );
}

/// A duplicate model name should cause validation to fail.
#[test]
fn test_validate_schema_duplicate_model_name_fails() {
    let ast = parse_schema(
        r#"
model User {
  id   Int    @id @default(autoincrement())
  name String
}

model User {
  id    Int    @id @default(autoincrement())
  email String
}
"#,
    )
    .expect("parse step should succeed");
    let result = validate_schema(ast);
    assert!(
        result.is_err(),
        "expected validation to fail for duplicate model name"
    );
}
