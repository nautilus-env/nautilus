//! Integration tests for the Visitor pattern.

mod common;

use common::parse_schema as parse;
use nautilus_schema::{
    ast::*,
    visitor::{walk_model, Visitor},
    Result, SchemaError,
};

/// Custom visitor that collects all model names.
#[derive(Default)]
struct ModelNameCollector {
    names: Vec<String>,
}

impl Visitor for ModelNameCollector {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        self.names.push(model.name.value.clone());
        walk_model(self, model)
    }
}

#[test]
fn test_model_name_collector() {
    let source = r#"
model User {
  id Int @id
}

model Post {
  id Int @id
}

model Comment {
  id Int @id
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = ModelNameCollector::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.names.len(), 3);
    assert_eq!(visitor.names[0], "User");
    assert_eq!(visitor.names[1], "Post");
    assert_eq!(visitor.names[2], "Comment");
}

/// Custom visitor that collects all field names with their types.
#[derive(Default)]
struct FieldCollector {
    fields: Vec<(String, String)>, // (field_name, type_name)
}

impl Visitor for FieldCollector {
    fn visit_field(&mut self, field: &FieldDecl) -> Result<()> {
        self.fields
            .push((field.name.value.clone(), field.field_type.to_string()));
        Ok(())
    }
}

#[test]
fn test_field_collector() {
    let source = r#"
model User {
  id       Int
  email    String
  isActive Boolean
}

model Post {
  id    BigInt
  title String
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = FieldCollector::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.fields.len(), 5);
    assert_eq!(visitor.fields[0], ("id".to_string(), "Int".to_string()));
    assert_eq!(
        visitor.fields[1],
        ("email".to_string(), "String".to_string())
    );
    assert_eq!(
        visitor.fields[2],
        ("isActive".to_string(), "Boolean".to_string())
    );
    assert_eq!(visitor.fields[3], ("id".to_string(), "BigInt".to_string()));
    assert_eq!(
        visitor.fields[4],
        ("title".to_string(), "String".to_string())
    );
}

/// Custom visitor that finds all @unique attributes.
#[derive(Default)]
struct UniqueFieldFinder {
    unique_fields: Vec<String>,
}

impl Visitor for UniqueFieldFinder {
    fn visit_field(&mut self, field: &FieldDecl) -> Result<()> {
        if field
            .attributes
            .iter()
            .any(|attr| matches!(attr, FieldAttribute::Unique))
        {
            self.unique_fields.push(field.name.value.clone());
        }
        Ok(())
    }
}

#[test]
fn test_unique_field_finder() {
    let source = r#"
model User {
  id       Int    @id
  email    String @unique
  username String @unique
  name     String
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = UniqueFieldFinder::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.unique_fields.len(), 2);
    assert_eq!(visitor.unique_fields[0], "email");
    assert_eq!(visitor.unique_fields[1], "username");
}

/// Custom visitor that finds all relation fields.
#[derive(Default)]
struct RelationFinder {
    relations: Vec<(String, String)>, // (model_name, field_name)
}

impl Visitor for RelationFinder {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        for field in &model.fields {
            if matches!(field.field_type, FieldType::UserType(_)) {
                self.relations
                    .push((model.name.value.clone(), field.name.value.clone()));
            }
        }
        walk_model(self, model)
    }
}

#[test]
fn test_relation_finder() {
    let source = r#"
model User {
  id    Int    @id
  posts Post[]
}

model Post {
  id     Int  @id
  userId Int
  user   User @relation(fields: [userId], references: [id])
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = RelationFinder::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.relations.len(), 2);
    assert_eq!(
        visitor.relations[0],
        ("User".to_string(), "posts".to_string())
    );
    assert_eq!(
        visitor.relations[1],
        ("Post".to_string(), "user".to_string())
    );
}

/// Custom visitor that validates model names (example of error propagation).
struct ModelNameValidator {
    invalid_names: Vec<String>,
}

impl ModelNameValidator {
    fn new() -> Self {
        Self {
            invalid_names: Vec::new(),
        }
    }

    fn is_valid_name(name: &str) -> bool {
        name.chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }
}

impl Visitor for ModelNameValidator {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        if !Self::is_valid_name(&model.name.value) {
            self.invalid_names.push(model.name.value.clone());
        }
        walk_model(self, model)
    }
}

#[test]
fn test_model_name_validator_success() {
    let source = r#"
model User {
  id Int @id
}

model Post {
  id Int @id
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = ModelNameValidator::new();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.invalid_names.len(), 0);
}

#[test]
fn test_model_name_validator_finds_invalid() {
    let source = r#"
model user {
  id Int @id
}

model Post {
  id Int @id
}

model post {
  id Int @id
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = ModelNameValidator::new();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.invalid_names.len(), 2);
    assert!(visitor.invalid_names.contains(&"user".to_string()));
    assert!(visitor.invalid_names.contains(&"post".to_string()));
}

/// Custom visitor that counts function calls in @default attributes.
#[derive(Default)]
struct FunctionCallCounter {
    function_names: Vec<String>,
}

impl Visitor for FunctionCallCounter {
    fn visit_expr(&mut self, expr: &Expr) -> Result<()> {
        if let Expr::FunctionCall { name, .. } = expr {
            self.function_names.push(name.value.clone());
        }
        nautilus_schema::visitor::walk_expr(self, expr)
    }
}

#[test]
fn test_function_call_counter() {
    let source = r#"
model User {
  id        Int      @id @default(autoincrement())
  uuid      Uuid     @default(uuid())
  createdAt DateTime @default(now())
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = FunctionCallCounter::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.function_names.len(), 3);
    assert!(visitor
        .function_names
        .contains(&"autoincrement".to_string()));
    assert!(visitor.function_names.contains(&"uuid".to_string()));
    assert!(visitor.function_names.contains(&"now".to_string()));
}

/// Custom visitor that builds a map of model dependencies based on relations.
#[derive(Default)]
struct DependencyMapper {
    dependencies: Vec<(String, String)>, // (from_model, to_model)
}

impl Visitor for DependencyMapper {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        let model_name = model.name.value.clone();

        for field in &model.fields {
            if let FieldType::UserType(type_name) = &field.field_type {
                if !field.is_array() {
                    self.dependencies
                        .push((model_name.clone(), type_name.clone()));
                }
            }
        }

        walk_model(self, model)
    }
}

#[test]
fn test_dependency_mapper() {
    let source = r#"
model User {
  id      Int      @id
  profile Profile?
  posts   Post[]
}

model Profile {
  id     Int  @id
  userId Int  @unique
  user   User @relation(fields: [userId], references: [id])
}

model Post {
  id       Int  @id
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = DependencyMapper::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.dependencies.len(), 3);
    assert!(visitor
        .dependencies
        .contains(&("User".to_string(), "Profile".to_string())));
    assert!(visitor
        .dependencies
        .contains(&("Profile".to_string(), "User".to_string())));
    assert!(visitor
        .dependencies
        .contains(&("Post".to_string(), "User".to_string())));
}

/// Visitor that errors on specific conditions (tests error propagation).
struct ErroringVisitor {
    fail_on_model: String,
}

impl Visitor for ErroringVisitor {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        if model.name.value == self.fail_on_model {
            return Err(SchemaError::Validation(
                format!("Intentional error on model {}", model.name.value),
                model.span,
            ));
        }
        walk_model(self, model)
    }
}

#[test]
fn test_visitor_error_propagation() {
    let source = r#"
model User {
  id Int @id
}

model Post {
  id Int @id
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = ErroringVisitor {
        fail_on_model: "Post".to_string(),
    };

    let result = visitor.visit_schema(&schema);
    assert!(result.is_err());

    if let Err(SchemaError::Validation(msg, _)) = result {
        assert!(msg.contains("Intentional error on model Post"));
    } else {
        panic!("Expected validation error");
    }
}

/// Custom visitor that finds all physical table/column names from @map attributes.
#[derive(Default)]
struct PhysicalNameCollector {
    table_names: Vec<String>,
    column_names: Vec<String>,
}

impl Visitor for PhysicalNameCollector {
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        for attr in &model.attributes {
            if let ModelAttribute::Map(name) = attr {
                self.table_names.push(name.clone());
            }
        }
        walk_model(self, model)
    }

    fn visit_field(&mut self, field: &FieldDecl) -> Result<()> {
        for attr in &field.attributes {
            if let FieldAttribute::Map(name) = attr {
                self.column_names.push(name.clone());
            }
        }
        Ok(())
    }
}

#[test]
fn test_physical_name_collector() {
    let source = r#"
model User {
  id    Int    @id @map("user_id")
  email String @map("email_address")
  
  @@map("users")
}

model Post {
  id      Int @id @map("post_id")
  content String
  
  @@map("posts")
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = PhysicalNameCollector::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.table_names.len(), 2);
    assert!(visitor.table_names.contains(&"users".to_string()));
    assert!(visitor.table_names.contains(&"posts".to_string()));

    assert_eq!(visitor.column_names.len(), 3);
    assert!(visitor.column_names.contains(&"user_id".to_string()));
    assert!(visitor.column_names.contains(&"email_address".to_string()));
    assert!(visitor.column_names.contains(&"post_id".to_string()));
}

#[test]
fn test_visitor_on_enum_declarations() {
    let source = r#"
enum Status {
  ACTIVE
  INACTIVE
  PENDING
}

enum Role {
  USER
  ADMIN
}
"#;

    let schema = parse(source).unwrap();

    let mut visitor = nautilus_schema::visitor::CountingVisitor::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.enums, 2);
}

#[test]
fn test_visitor_on_datasource_and_generator() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url = "postgres://localhost"
}

generator client {
  provider = "nautilus"
  output = "./generated"
}
"#;

    let schema = parse(source).unwrap();

    let mut visitor = nautilus_schema::visitor::CountingVisitor::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.datasources, 1);
    assert_eq!(visitor.generators, 1);
}
#[test]
fn test_counting_visitor_on_complex_schema() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url = "postgres://localhost"
}

generator client {
  provider = "nautilus"
}

enum Status {
  ACTIVE
  INACTIVE
}

model User {
  id Int @id
  name String
}

model Post {
  id Int @id
  title String
  content String
}
"#;

    let schema = parse(source).unwrap();
    let mut visitor = nautilus_schema::visitor::CountingVisitor::default();
    visitor.visit_schema(&schema).unwrap();

    assert_eq!(visitor.datasources, 1);
    assert_eq!(visitor.generators, 1);
    assert_eq!(visitor.enums, 1);
    assert_eq!(visitor.models, 2);
    assert_eq!(visitor.fields, 5);
}
