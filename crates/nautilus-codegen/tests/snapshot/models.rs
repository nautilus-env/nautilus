use super::support::{assert_codegen_snapshot, validate};
use nautilus_codegen::enum_gen::generate_all_enums;
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::python::{generate_all_python_models, generate_python_enums};

const USER_POST_SCHEMA: &str = include_str!("../fixtures/schemas/user_post.nautilus");
const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_rust_struct_is_generated() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");
    assert_codegen_snapshot!("rust_struct_is_generated", code);
}

#[test]
fn test_rust_optional_field_is_option() {
    let ir = validate(
        r#"
model Post {
  id      Int     @id @default(autoincrement())
  content String?
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("Post").expect("Post model missing");
    let model_decl = code
        .split("impl Post")
        .next()
        .expect("generated Post struct should precede impl block");
    assert!(
        code.contains("pub content: Option<String>,"),
        "expected nullable schema field to be nullable on the full Rust model:\n{code}"
    );
    assert!(
        !model_decl.contains("pub content: Option<Option<String>>"),
        "full Rust model should not wrap nullable fields again for projection:\n{code}"
    );
    assert!(
        code.contains("pub fn content(&self) -> nautilus_core::Column<Option<String>>"),
        "typed Rust projection columns should preserve nullable output type:\n{code}"
    );
    assert!(
        code.contains("pub fn find_many_select<C, F>(")
            && code.contains("select returns partial rows and cannot be decoded as a full Post"),
        "expected Rust delegates to expose typed projection APIs and reject model-returning select:\n{code}"
    );
    assert_codegen_snapshot!("rust_optional_field_is_option", code);
}

#[test]
fn test_rust_enum_generation() {
    let ir = validate(
        r#"
enum Status {
  ACTIVE
  INACTIVE
  PENDING
}

model User {
  id     Int    @id @default(autoincrement())
  status Status
}
"#,
    );
    let enums_code = generate_all_enums(&ir.enums).expect("generate_all_enums should succeed");
    assert_codegen_snapshot!("rust_enum_generation", enums_code);
}

#[test]
fn test_rust_multiple_models_generated() {
    let ir = validate(USER_POST_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    assert!(models.contains_key("User"), "expected User model");
    assert!(models.contains_key("Post"), "expected Post model");
}

#[test]
fn test_rust_from_row_impl_generated() {
    let ir = validate(
        r#"
model Product {
  id    Int    @id @default(autoincrement())
  name  String
  price Float
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("Product").expect("Product missing");
    assert_codegen_snapshot!("rust_from_row_impl_generated", code);
}

#[test]
fn test_rust_model_generates_schema_aware_read_hints() {
    let ir = validate(
        r#"
model User {
  id         Int           @id @default(autoincrement())
  externalId Uuid
  price      Decimal(10, 2)
  profile    Json
  tags       String[]      @store(json)
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User missing");

    assert!(
        code.contains("normalize_value_with_hint"),
        "expected generated Rust model to normalize projected values inline during decode:\n{code}"
    );
    assert!(
        code.contains("FromValue::from_value_owned"),
        "expected generated Rust model to decode normalized values without extra cloning:\n{code}"
    );
    assert!(
        code.contains("Some(crate::ValueHint::Uuid)"),
        "expected generated Rust model to emit a UUID read hint:\n{code}"
    );
    assert!(
        code.contains("Some(crate::ValueHint::Decimal)"),
        "expected generated Rust model to emit a Decimal read hint:\n{code}"
    );
    assert!(
        code.contains("Some(crate::ValueHint::Json)"),
        "expected generated Rust model to emit JSON read hints:\n{code}"
    );
}

#[test]
fn test_python_class_is_generated() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");
    assert_codegen_snapshot!("python_class_is_generated", code);
}

#[test]
fn test_python_optional_field_is_optional_type() {
    let ir = validate(
        r#"
model Post {
  id      Int     @id @default(autoincrement())
  title   String
  content String?
}
"#,
    );
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "post.py")
        .expect("post missing");
    assert!(
        code.contains("content: Optional[str]"),
        "expected nullable output field to be Optional[str]:\n{code}"
    );
    assert!(
        code.contains("content: NotRequired[Optional[str]]"),
        "expected nullable create/update input fields to allow explicit None:\n{code}"
    );
    assert!(
        code.contains("content: NotRequired[Union[str, None, StringFilter]]"),
        "expected nullable where input fields to allow explicit None equality:\n{code}"
    );
    assert!(
        code.contains("title: Required[str]"),
        "expected required create input fields to stay required inside total=False TypedDicts:\n{code}"
    );
    assert_codegen_snapshot!("python_optional_field_is_optional_type", code);
}

#[test]
fn test_python_enum_class() {
    let ir = validate(
        r#"
enum Role {
  USER
  ADMIN
}

model User {
  id   Int  @id @default(autoincrement())
  role Role
}
"#,
    );
    let enums_code =
        generate_python_enums(&ir.enums).expect("generate_python_enums should succeed");
    assert_codegen_snapshot!("python_enum_class", enums_code);
}

#[test]
fn test_python_multiple_models_generated() {
    let ir = validate(USER_POST_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let names: Vec<&str> = models.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"user.py"), "expected user in {names:?}");
    assert!(names.contains(&"post.py"), "expected post in {names:?}");
}
