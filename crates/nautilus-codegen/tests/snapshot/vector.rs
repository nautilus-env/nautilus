use super::support::{generated_python_file, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_generated_vector_filters_are_typed_in_js_and_python() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [vector]
}

generator client {
  provider    = "nautilus-client-java"
  output      = "./db"
  package     = "com.example.db"
  group_id    = "com.example"
  artifact_id = "db"
}

model User {
  id        Int       @id @default(autoincrement())
  embedding Vector(3)
}
"#,
    );

    let (_, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_dts = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("user declaration missing");
    assert!(js_dts.contains("export interface VectorFilter {"));
    // With the `vector` extension declared, the filter accepts the wrapper
    // `Vector` instance or the raw `number[]` via the `VectorInput` union.
    assert!(js_dts.contains("equals?: VectorInput;"));
    assert!(js_dts.contains("not?:    VectorInput;"));
    assert!(js_dts.contains("isNull?: boolean;"));
    assert!(js_dts.contains("embedding?: VectorInput | VectorFilter;"));
    assert!(js_dts.contains("export type VectorMetric = 'l2' | 'innerProduct' | 'cosine';"));
    assert!(js_dts.contains("export type UserVectorFieldKeys = 'embedding';"));
    assert!(js_dts.contains("export interface UserNearestInput {"));
    assert!(js_dts.contains("nearest?:  UserNearestInput;"));
    // The Nearest input also widens its `query` to accept the wrapper.
    assert!(js_dts.contains("query:  VectorInput;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("class VectorFilter(TypedDict, total=False):"));
    assert!(py_model.contains("equals: NotRequired[VectorInput]"));
    assert!(py_model.contains("not_: NotRequired[VectorInput]"));
    assert!(py_model.contains("is_null: NotRequired[bool]"));
    assert!(py_model.contains("embedding: NotRequired[Union[VectorInput, VectorFilter]]"));
    assert!(py_model.contains("VectorMetric = Literal[\"l2\", \"innerProduct\", \"cosine\"]"));
    assert!(py_model.contains("UserVectorFieldKeys = Literal[\"embedding\"]"));
    assert!(py_model.contains("class UserNearestInput(TypedDict):"));
    assert!(py_model.contains("nearest: Optional[UserNearestInput] = None"));

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("java client generation");
    let java_dsl = java_files
        .iter()
        .find(|(name, _)| name.ends_with("/UserDsl.java"))
        .map(|(_, code)| code.as_str())
        .expect("UserDsl.java missing");
    assert!(java_dsl.contains("public enum VectorMetric {"));
    assert!(java_dsl.contains("public Nearest embedding() {"));
    assert!(java_dsl.contains("public FindManyArgs nearest(Consumer<Nearest> spec) {"));
}

#[test]
fn test_rust_client_supports_vector_nearest_search() {
    let ir = validate(
        r#"
model Document {
  id        Int      @id @default(autoincrement())
  title     String
  embedding Vector(3) @map("embedding_vec")
}
"#,
    );

    let models = generate_all_models(&ir, true).expect("generate_all_models should succeed");
    let code = models.get("Document").expect("Document model missing");

    assert!(
        code.contains("pub fn embedding_nearest(")
            && code.contains("metric: nautilus_core::VectorMetric,")
            && code.contains("field: \"embedding\".to_string(),"),
        "expected a typed nearest constructor next to the column accessors:\n{code}"
    );
    assert!(
        code.contains(
            "\"Document__embedding_vec\" | \"embedding\" | \"embedding_vec\" => Some(\"Document__embedding_vec\")"
        ),
        "expected nearest.field to resolve logical, database and qualified names:\n{code}"
    );
    assert!(
        code.contains("nautilus_core::Expr::vector_distance(")
            && code.contains("builder.order_by_expr(distance, OrderDir::Asc)"),
        "expected the SQL path to order by pgvector distance:\n{code}"
    );
    assert!(
        code.contains("'nearest' requires a positive 'take' limit")
            && code.contains("'nearest' cannot be combined with 'cursor'")
            && code.contains("'nearest' cannot be combined with 'distinct'"),
        "expected the generated client to mirror the engine's nearest restrictions:\n{code}"
    );
    assert!(
        code.contains("if let Some(nearest) = args.nearest {"),
        "expected the delegate to forward FindManyArgs::nearest to the builder:\n{code}"
    );
}

#[test]
fn test_rust_client_without_vector_fields_rejects_nearest() {
    let ir = validate(USER_SCHEMA);

    let models = generate_all_models(&ir, true).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");

    assert!(
        !code.contains("_nearest("),
        "a model without vector fields should not get nearest accessors:\n{code}"
    );
    assert!(
        code.contains("fn vector_distance_column(_field: &str) -> Option<&'static str>"),
        "expected nearest.field resolution to always exist and reject every field:\n{code}"
    );
}

#[test]
fn test_python_client_indents_the_nearest_argument_block() {
    let ir = validate(
        r#"
model Document {
  id        Int      @id @default(autoincrement())
  title     String
  embedding Vector(3)
}
"#,
    );

    let models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let code = generated_python_file(&models, "document.py");

    let blocks = code.matches("if nearest is not None:").count();
    let indented_blocks = code
        .matches(
            "\n        if nearest is not None:\n            args[\"nearest\"] = _serialize_nearest_input(nearest)\n",
        )
        .count();

    assert!(blocks > 0, "expected a nearest argument block:\n{code}");
    assert_eq!(
        indented_blocks, blocks,
        "every nearest argument block must stay indented inside its method; Tera whitespace \
         control once stripped the leading indentation and produced invalid Python:\n{code}"
    );
    assert!(
        !code.contains("\nif nearest is not None:"),
        "the nearest argument block must never start at column 0:\n{code}"
    );
}
