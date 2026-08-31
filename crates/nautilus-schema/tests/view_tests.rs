mod common;

use common::parse_schema as parse;
use nautilus_schema::{format_schema, validate_schema, SchemaError};

fn validation_error(source: &str) -> String {
    let ast = parse(source).unwrap();
    match validate_schema(ast).unwrap_err() {
        SchemaError::Validation(message, _) => message,
        other => panic!("Expected a validation error, got {other:?}"),
    }
}

#[test]
fn view_block_parses_into_a_read_only_model() {
    let source = r#"
view ActiveUser {
  id    Int    @id
  email String
  @@map("active_users")
}
"#;
    let ast = parse(source).unwrap();
    let view = ast.views().next().expect("view declaration");
    assert!(view.is_view);
    assert_eq!(view.name.value, "ActiveUser");
    assert_eq!(view.table_name(), "active_users");

    let ir = validate_schema(ast).unwrap();
    let model = ir.get_model("ActiveUser").unwrap();
    assert!(model.is_view);
    assert_eq!(model.db_name, "active_users");
}

#[test]
fn a_model_is_not_a_view() {
    let source = r#"
model User {
  id Int @id
}
"#;
    let ir = validate_schema(parse(source).unwrap()).unwrap();
    assert!(!ir.get_model("User").unwrap().is_view);
}

#[test]
fn formatter_round_trips_the_view_keyword() {
    let source = "view ActiveUser {\n  id Int @id\n}\n";
    let ast = parse(source).unwrap();
    let formatted = format_schema(&ast, source);
    assert!(
        formatted.starts_with("view ActiveUser {"),
        "expected a view block, got:\n{formatted}"
    );
}

#[test]
fn a_view_cannot_declare_a_relation() {
    let source = r#"
model Post {
  id       Int  @id
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}

model User {
  id    Int    @id
  posts Post[]
}

view ActiveUser {
  id    Int    @id
  posts Post[]
}
"#;
    let message = validation_error(source);
    assert!(
        message.contains("view 'ActiveUser'") && message.contains("cannot take part in a relation"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_model_cannot_relate_to_a_view() {
    let source = r#"
model Post {
  id       Int        @id
  authorId Int
  author   ActiveUser @relation(fields: [authorId], references: [id])
}

view ActiveUser {
  id Int @id
}
"#;
    let message = validation_error(source);
    assert!(
        message.contains("points at view 'ActiveUser'"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_view_cannot_declare_an_index() {
    let source = r#"
view ActiveUser {
  id    Int    @id
  email String
  @@index([email])
}
"#;
    let message = validation_error(source);
    assert!(
        message.contains("View 'ActiveUser' cannot declare @@index"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_view_cannot_declare_a_table_check() {
    let source = r#"
view ActiveUser {
  id  Int @id
  age Int
  @@check(age >= 0)
}
"#;
    let message = validation_error(source);
    assert!(
        message.contains("View 'ActiveUser' cannot declare @@check"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_view_field_cannot_declare_a_default() {
    let source = r#"
view ActiveUser {
  id     Int     @id
  active Boolean @default(true)
}
"#;
    let message = validation_error(source);
    assert!(
        message.contains("cannot declare @default") && message.contains("view 'ActiveUser'"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_view_field_cannot_be_computed() {
    let source = r#"
view OrderTotal {
  id    Int @id
  price Int
  qty   Int
  total Int @computed(price * qty, Stored)
}
"#;
    let message = validation_error(source);
    assert!(
        message.contains("cannot declare @computed"),
        "unexpected message: {message}"
    );
}
