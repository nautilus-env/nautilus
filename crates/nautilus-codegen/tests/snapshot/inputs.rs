use super::support::{
    assert_codegen_snapshot, generated_named_file, generated_python_file, python_runtime_codec,
    section_until, validate,
};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::{generate_all_python_models, generate_python_composite_types};

const COMPOSITE_ADDRESS_SCHEMA: &str =
    include_str!("../fixtures/schemas/composite_address.nautilus");

#[test]
fn test_rust_generates_create_input() {
    let ir = validate(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");
    assert_codegen_snapshot!("rust_generates_create_input", code);
}

#[test]
fn test_uuidv7_id_is_not_required_in_create_inputs() {
    let ir = validate(
        r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

model User {
  id   Uuid   @id @default(uuidv7())
  name String
}
"#,
    );

    let py_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let py_code = generated_python_file(&py_models, "user.py");
    let py_create_input = section_until(
        py_code,
        "class UserCreateInput",
        "\n\nclass UserUpdateInput",
    );
    assert!(
        py_create_input.contains("name: Required[str]"),
        "expected name to remain required in Python create input:\n{py_create_input}"
    );
    assert!(
        py_create_input.contains("id: NotRequired[UUID]"),
        "uuidv7 id should be optional in Python create input:\n{py_create_input}"
    );
    assert!(
        !py_create_input.contains("id: Required[UUID]"),
        "uuidv7 id should not be required in Python create input:\n{py_create_input}"
    );

    let (_js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_code = generated_named_file(&dts_models, "user.d.ts");
    let js_create_input = section_until(
        js_code,
        "export interface UserCreateInput",
        "\n\nexport interface UserUpdateInput",
    );
    assert!(
        js_create_input.contains("name: string;"),
        "expected name to remain required in TypeScript create input:\n{js_create_input}"
    );
    assert!(
        !js_create_input.contains("\n  id"),
        "uuidv7 id should not be required in TypeScript create input:\n{js_create_input}"
    );
}

#[test]
fn test_python_create_many_normalizes_mapped_fields() {
    let ir = validate(
        r#"
enum Role {
  USER
  ADMIN
}

model User {
  id          Int    @id @default(autoincrement()) @map("user_id")
  displayName String @map("display_name")
  role        Role   @map("user_role")

  @@map("users")
}
"#,
    );
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");

    assert!(
        code.contains(r#"_process_create_data(_entry, _users_py_to_db)"#),
        "expected create_many() to normalize each entry through _process_create_data:\n{code}"
    );
}

#[test]
fn test_python_composite_write_inputs_use_generated_types() {
    let ir = validate(COMPOSITE_ADDRESS_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");

    assert!(
        code.contains("shippingAddress: NotRequired[Optional[Address]]"),
        "expected nullable composite create/update inputs to use Optional[Address]:\n{code}"
    );
    assert!(
        code.contains("shippingAddresses: NotRequired[List[Address]]"),
        "expected composite array update inputs to use List[Address]:\n{code}"
    );
    assert!(
        code.contains("_process_create_data = _User_input_codec.create_data"),
        "expected composite payloads to be written through the shared codec:\n{code}"
    );
    assert!(
        section_until(&python_runtime_codec(), "def create_data", "\n\n    def ")
            .contains("result[db_key] = self.scalar_input(key, value)"),
        "expected composite payload serialization to flow through scalar_input"
    );

    let composite_types = generate_python_composite_types(&ir.composite_types)
        .expect("generate_python_composite_types should succeed")
        .expect("types should be generated");
    assert!(
        composite_types.contains("from typing_extensions import TypedDict"),
        "expected Python composite TypedDicts to use typing_extensions on Python < 3.12:\n{composite_types}"
    );
}

#[test]
fn test_js_composite_write_inputs_use_generated_types() {
    let ir = validate(COMPOSITE_ADDRESS_SCHEMA);
    let (_js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let (_, code) = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .expect("user declaration missing");

    assert!(
        code.contains("shippingAddress?: Address | null;"),
        "expected nullable composite create/update input to use Address | null:\n{code}"
    );
    assert!(
        code.contains("shippingAddresses?: Address[];"),
        "expected composite array create input to use Address[] instead of object[]:\n{code}"
    );
    assert!(
        code.contains("shippingAddress?: Address | null;"),
        "expected composite update input to use Address instead of object:\n{code}"
    );
}

#[test]
fn test_js_nullable_input_fields_match_schema_nullability() {
    let ir = validate(
        r#"
model User {
  id       Int     @id @default(autoincrement())
  name     String
  nickname String?
}
"#,
    );
    let (_js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let code = generated_named_file(&dts_models, "user.d.ts");

    assert!(
        code.contains(
            "export interface UserCreateInput {\n  name: string;\n  nickname?: string | null;"
        ),
        "expected create input to require name and allow null for nullable nickname:\n{code}"
    );
    assert!(
        code.contains("nickname?: string | null | StringFilter;"),
        "expected nullable where input fields to allow explicit null equality:\n{code}"
    );
    assert!(
        code.contains(
            "export interface UserUpdateInput {\n  name?: string;\n  nickname?: string | null;"
        ),
        "expected update input to allow omission separately from schema nullability:\n{code}"
    );
}
