//! Snapshot tests for the code generator: parse a schema, generate code, and
//! optionally assert the full rendered output against local-only snapshots.
//!
//! Snapshot baselines live in `tests/snapshots/`, which is gitignored on
//! purpose. Regular test runs ignore those local `.snap` files so stale
//! baselines do not break unrelated codegen work. To force snapshot assertions
//! or generate fresh local baselines, run with `NAUTILUS_LOCAL_SNAPSHOTS=1`
//! (typically alongside `INSTA_UPDATE=always`). To skip snapshot assertions
//! explicitly even when that env var is set, run with
//! `NAUTILUS_SKIP_LOCAL_SNAPSHOTS=1`.

use std::sync::OnceLock;

use nautilus_codegen::{
    enum_gen::generate_all_enums,
    extension_types::{
        generate_java_extension_files, generate_js_extension_files,
        generate_python_extension_files, generate_rust_extension_files, ExtensionRegistry,
    },
    generator::generate_all_models,
    java::generate_java_client,
    js::{generate_all_js_models, generate_js_client, js_runtime_files},
    python::{
        generate_all_python_models, generate_python_client, generate_python_composite_types,
        generate_python_enums, python_runtime_files,
    },
};
use nautilus_schema::validate_schema_source;

const BLOG_RELATIONS_SCHEMA: &str = include_str!("fixtures/schemas/blog_relations.nautilus");
const COMPOSITE_ADDRESS_SCHEMA: &str = include_str!("fixtures/schemas/composite_address.nautilus");
const JAVA_CLIENT_ASYNC_SCHEMA: &str = include_str!("fixtures/schemas/java_client_async.nautilus");
const JAVA_CLIENT_SCHEMA: &str = include_str!("fixtures/schemas/java_client.nautilus");
const JAVA_CLIENT_SYNC_SCHEMA: &str = include_str!("fixtures/schemas/java_client_sync.nautilus");
const USER_MAPPED_SCHEMA: &str = include_str!("fixtures/schemas/user_mapped.nautilus");
const USER_POST_SCHEMA: &str = include_str!("fixtures/schemas/user_post.nautilus");
const USER_SCHEMA: &str = include_str!("fixtures/schemas/user.nautilus");

fn local_snapshots_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();

    *ENABLED.get_or_init(|| {
        if std::env::var_os("NAUTILUS_SKIP_LOCAL_SNAPSHOTS").is_some() {
            return false;
        }

        if std::env::var_os("NAUTILUS_LOCAL_SNAPSHOTS").is_some() {
            return true;
        }

        false
    })
}

macro_rules! assert_local_snapshot {
    ($value:expr $(,)?) => {{
        let snapshot_value = &$value;
        assert!(
            !snapshot_value.is_empty(),
            "generated snapshot content should not be empty"
        );
        if local_snapshots_enabled() {
            insta::assert_snapshot!(snapshot_value);
        }
    }};
    ($name:expr, $value:expr $(,)?) => {{
        let snapshot_value = &$value;
        assert!(
            !snapshot_value.is_empty(),
            "generated snapshot content should not be empty"
        );
        if local_snapshots_enabled() {
            insta::assert_snapshot!($name, snapshot_value);
        }
    }};
}

fn validate(source: &str) -> nautilus_schema::ir::SchemaIr {
    validate_schema_source(source)
        .expect("validation failed")
        .ir
}

fn generated_java_file<'a>(files: &'a [(String, String)], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path.ends_with(suffix))
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated Java file ending with '{suffix}'"))
}

fn generated_python_file<'a>(files: &'a [(String, String)], file_name: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path == file_name)
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated Python file '{file_name}'"))
}

fn generated_named_file<'a>(files: &'a [(String, String)], file_name: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path == file_name)
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated file '{file_name}'"))
}

fn section_until<'a>(code: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let start = code
        .find(start_marker)
        .unwrap_or_else(|| panic!("missing section start '{start_marker}'"));
    let rest = &code[start..];
    let end = rest.find(end_marker).unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn test_rust_struct_is_generated() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");
    assert_local_snapshot!(code);
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
    assert_local_snapshot!(code);
}

#[test]
fn test_rust_generates_find_many_builder() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");
    assert!(
        code.contains("FindMany"),
        "expected FindMany builder:\n{code}"
    );
    assert_local_snapshot!(code);
}

#[test]
fn test_rust_generates_count_and_group_by_api() {
    let ir = validate(
        r#"
enum Role {
  ADMIN
  MEMBER
}

model User {
  id          Int    @id @default(autoincrement()) @map("user_id")
  displayName String @map("display_name")
  role        Role
  views       Int

  @@map("users")
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");

    assert!(
        code.contains("pub struct UserCountArgs"),
        "expected generated Rust code to expose count args:\n{code}"
    );
    assert!(
        code.contains("pub fn count("),
        "expected generated Rust code to expose count():\n{code}"
    );
    assert!(
        code.contains("pub fn group_by("),
        "expected generated Rust code to expose group_by():\n{code}"
    );
    assert!(
        code.contains("pub enum UserScalarField"),
        "expected generated Rust code to expose scalar field enums for group_by():\n{code}"
    );
    assert!(
        code.contains("Self::DisplayName => \"displayName\""),
        "expected mapped fields to serialize through logical names in aggregate APIs:\n{code}"
    );
    assert!(
        code.contains("pub struct UserGroupByOutput"),
        "expected generated Rust code to expose a typed group_by output:\n{code}"
    );
}

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
    assert_local_snapshot!(code);
}

#[test]
fn test_rust_generated_query_builders_use_static_column_markers() {
    let ir = validate(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  name  String?

  @@map("users")
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");

    assert!(
        code.contains("ColumnMarker::from_static(\"users\", \"email\")"),
        "expected generated Rust code to use borrowed column metadata for known columns:\n{code}"
    );
    assert!(
        code.contains("ColumnMarker::from_static(\"users\", \"id\")"),
        "expected generated Rust code to reuse borrowed PK metadata in returning/select paths:\n{code}"
    );
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
    assert_local_snapshot!(enums_code);
}

#[test]
fn test_rust_multiple_models_generated() {
    let ir = validate(USER_POST_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    assert!(models.contains_key("User"), "expected User model");
    assert!(models.contains_key("Post"), "expected Post model");
}

#[test]
fn test_rust_async_generates_async_fns() {
    let ir = validate(USER_SCHEMA);
    let sync_models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let async_models = generate_all_models(&ir, true).expect("generate_all_models should succeed");
    let sync_code = sync_models.get("User").unwrap();
    let async_code = async_models.get("User").unwrap();
    assert!(
        async_code.contains("async"),
        "expected async in async mode:\n{async_code}"
    );
    assert_ne!(sync_code, async_code, "sync and async should differ");
    assert_local_snapshot!("rust_user_async", async_code);
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
    assert_local_snapshot!(code);
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

/// Exercises RelationContext: a model with both a to-one and a to-many relation.
#[test]
fn test_rust_model_with_relation() {
    let ir = validate(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  name  String
  posts Post[]
}

model Post {
  id       Int    @id @default(autoincrement())
  title    String
  authorId Int
  author   User   @relation(fields: [authorId], references: [id])
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let user_code = models.get("User").expect("User missing");
    let post_code = models.get("Post").expect("Post missing");
    assert_local_snapshot!("rust_user_with_posts_relation", user_code);
    assert_local_snapshot!("rust_post_with_author_relation", post_code);
}

#[test]
fn test_rust_async_delegate_exposes_stream_many() {
    let ir = validate(USER_SCHEMA);
    let async_models = generate_all_models(&ir, true).expect("generate_all_models should succeed");
    let async_code = async_models.get("User").expect("User missing");

    assert!(
        async_code.contains("pub fn stream_many("),
        "expected async delegate to expose stream_many:\n{async_code}"
    );
    assert!(
        async_code.contains("execute_owned(sql)"),
        "expected stream_many to drive the executor's owned-stream path:\n{async_code}"
    );
    assert!(
        async_code.contains("stream_many does not support backward pagination"),
        "expected stream_many to reject backward pagination explicitly:\n{async_code}"
    );

    let sync_models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let sync_code = sync_models.get("User").expect("User missing");
    assert!(
        !sync_code.contains("pub fn stream_many("),
        "stream_many should not be emitted for sync clients (the runtime would have to block on iteration); got:\n{sync_code}"
    );
}

#[test]
fn test_rust_relation_include_routes_through_engine_path() {
    let ir = validate(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  posts Post[]
}

model Post {
  id       Int    @id @default(autoincrement())
  title    String
  authorId Int
  author   User   @relation(fields: [authorId], references: [id])
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let user_code = models.get("User").expect("User missing");

    assert!(
        user_code.contains("crate::runtime::try_find_many_via_engine::<_, User>("),
        "expected relation include reads to route through the embedded engine path:\n{user_code}"
    );
    assert!(
        user_code.contains("if !args.include.is_empty() {"),
        "expected generated delegate to treat include queries as engine-only in the local fallback:\n{user_code}"
    );
    assert!(
        user_code.contains("include queries require the embedded engine path in the generated Rust client"),
        "expected the fallback path to explain why include queries stay on the engine path:\n{user_code}"
    );
}

#[test]
fn test_rust_delete_uses_single_record_fast_path_for_unique_filters() {
    let ir = validate(
        r#"
model User {
  id       Int    @id @default(autoincrement())
  email    String @unique
  tenantId Int
  slug     String

  @@unique([tenantId, slug])
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let user_code = models.get("User").expect("User missing");

    assert!(
        user_code.contains("fn is_single_record_filter(filter: &nautilus_core::Expr) -> bool"),
        "expected generated Rust code to recognize single-record filters:\n{user_code}"
    );
    assert!(
        user_code.contains("&[\"tenant_id\", \"slug\"]"),
        "expected composite unique constraints to participate in the delete fast path:\n{user_code}"
    );
    assert!(
        user_code.contains("supports_returning()")
            && user_code.contains("is_single_record_filter(&filter)")
            && user_code.contains("return match deleted.len()"),
        "expected delete() to use the single-query fast path for unique filters:\n{user_code}"
    );
}

#[test]
fn test_rust_upsert_attempts_update_before_find_on_returning_backends() {
    let ir = validate(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  name  String
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let user_code = models.get("User").expect("User missing");

    let update_idx = user_code
        .find("if self.client.dialect().supports_returning() && has_update_assignments {")
        .expect("missing upsert update-first fast path");
    let find_idx = user_code
        .find("let existing = self.find_first(")
        .expect("missing upsert fallback lookup");

    assert!(
        update_idx < find_idx,
        "expected upsert() to try the update path before the read fallback:\n{user_code}"
    );
    assert!(
        user_code.contains("let has_update_assignments = args.update.has_assignments();"),
        "expected generated upsert() to reuse update-input assignment detection:\n{user_code}"
    );
}

#[test]
fn test_rust_named_inverse_relations_use_matching_relation_name() {
    let ir = validate(
        r#"
model User {
  id            Int    @id @default(autoincrement())
  authoredPosts Post[] @relation(name: "AuthoredPosts")
  reviewedPosts Post[] @relation(name: "ReviewedPosts")
}

model Post {
  id         Int    @id @default(autoincrement())
  title      String
  authorId   Int
  reviewerId Int
  author     User   @relation(name: "AuthoredPosts", fields: [authorId], references: [id])
  reviewer   User   @relation(name: "ReviewedPosts", fields: [reviewerId], references: [id])
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let user_code = models.get("User").expect("User missing");

    assert!(
        user_code.contains(
            "nautilus_core::Expr::relation_some(\n            \"reviewed_posts\",\n            \"User\",\n            \"Post\",\n            \"reviewerId\",\n            \"id\","
        ),
        "expected reviewed_posts inverse relation helpers to target reviewer_id instead of another FK:\n{user_code}"
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
    assert_local_snapshot!(code);
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
    assert_local_snapshot!(code);
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
    assert_local_snapshot!(enums_code);
}

#[test]
fn test_python_async_generates_async_defs() {
    let ir = validate(USER_SCHEMA);
    let sync_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let async_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let (_, sync_code) = sync_models.iter().find(|(n, _)| n == "user.py").unwrap();
    let (_, async_code) = async_models.iter().find(|(n, _)| n == "user.py").unwrap();
    assert!(
        async_code.contains("async def"),
        "expected async def:\n{async_code}"
    );
    assert_ne!(sync_code, async_code, "sync and async should differ");
    assert_local_snapshot!("python_user_async", async_code);
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

/// Exercises `generate_python_client`: verifies the output contains the top-level
/// `NautilusClient` class and per-model delegate attributes.
#[test]
fn test_python_client_generation() {
    let ir = validate(USER_POST_SCHEMA);
    let client_sync = generate_python_client(&ir.models, "schema.nautilus", false)
        .expect("generate_python_client should succeed");
    let client_async = generate_python_client(&ir.models, "schema.nautilus", true)
        .expect("generate_python_client should succeed");
    assert!(
        client_sync.contains("NautilusClient"),
        "expected NautilusClient:\n{client_sync}"
    );
    assert!(
        client_async.contains("async def") || client_async.contains("async"),
        "expected async keyword in async client:\n{client_async}"
    );
    assert_ne!(
        client_sync, client_async,
        "sync and async clients should differ"
    );
    assert_local_snapshot!("python_client_sync", &client_sync);
}

#[test]
fn test_js_client_exposes_batch_transactions_and_runtime_stays_on_protocol_v1() {
    let ir = validate(USER_SCHEMA);
    let (client_js, client_dts) = generate_js_client(&ir.models, "schema.nautilus")
        .expect("generate_js_client should succeed");
    let runtime = js_runtime_files();
    let client_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_client.js")
        .expect("missing JS runtime client")
        .1
        .as_str();
    let protocol_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_protocol.js")
        .expect("missing JS runtime protocol")
        .1
        .as_str();
    let error_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_errors.js")
        .expect("missing JS runtime errors")
        .1
        .as_str();
    let tx_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_transaction.js")
        .expect("missing JS runtime transaction")
        .1
        .as_str();

    assert!(
        client_js.contains("async $transactionBatch(operations, options)"),
        "expected generated JS client to expose $transactionBatch():\n{client_js}"
    );
    assert!(
        client_dts.contains("$transactionBatch("),
        "expected generated JS declarations to expose $transactionBatch():\n{client_dts}"
    );
    assert!(
        protocol_runtime.contains("export const PROTOCOL_VERSION = 1;")
            && client_runtime.contains("protocolVersion: PROTOCOL_VERSION")
            && client_runtime.contains("client expects ${PROTOCOL_VERSION}")
            && client_runtime.contains("transaction.batch")
            && client_runtime.contains("async *_streamRpc(")
            && client_runtime.contains("method: 'request.cancel'"),
        "expected JS runtime client to reuse the shared protocol version constant and expose transaction.batch:\n{client_runtime}\n\nProtocol:\n{protocol_runtime}"
    );
    assert!(
        error_runtime.contains("this.data = details?.data"),
        "expected JS runtime errors to retain error.data from the engine:\n{error_runtime}"
    );
    assert!(
        !tx_runtime.contains("snapshot"),
        "expected JS runtime isolation levels to match the protocol exactly:\n{tx_runtime}"
    );
}

#[test]
fn test_python_runtime_stays_on_protocol_v1_and_preserves_error_data() {
    let runtime = python_runtime_files();
    let client_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_client.py")
        .expect("missing Python runtime client")
        .1
        .as_str();
    let protocol_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_protocol.py")
        .expect("missing Python runtime protocol")
        .1
        .as_str();
    let error_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_errors.py")
        .expect("missing Python runtime errors")
        .1
        .as_str();
    let tx_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_transaction.py")
        .expect("missing Python runtime transaction")
        .1
        .as_str();

    assert!(
        protocol_runtime.contains("PROTOCOL_VERSION = 1")
            && client_runtime.contains("\"protocolVersion\": PROTOCOL_VERSION")
            && client_runtime.contains("client expects {PROTOCOL_VERSION}")
            && client_runtime.contains("async def transaction_batch(")
            && client_runtime.contains("async def _stream_rpc(")
            && client_runtime.contains("method=\"request.cancel\""),
        "expected Python runtime client to reuse the shared protocol version constant and keep transaction_batch():\n{client_runtime}\n\nProtocol:\n{protocol_runtime}"
    );
    assert!(
        protocol_runtime.contains("self.error.data"),
        "expected Python runtime protocol to preserve error.data:\n{protocol_runtime}"
    );
    assert!(
        error_runtime.contains("self.data = data"),
        "expected Python runtime errors to retain error.data from the engine:\n{error_runtime}"
    );
    assert!(
        !tx_runtime.contains("SNAPSHOT"),
        "expected Python runtime isolation levels to match the protocol exactly:\n{tx_runtime}"
    );
}

#[test]
fn test_python_runtime_exposes_engine_pool_options() {
    let ir = validate(USER_SCHEMA);
    let client = generate_python_client(&ir.models, "schema.nautilus", false)
        .expect("generate_python_client should succeed");
    let runtime = python_runtime_files();
    let engine_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_engine.py")
        .expect("missing Python runtime engine")
        .1
        .as_str();

    assert!(
        client.contains("pool_options: EnginePoolOptions | None = None"),
        "expected generated Python client to expose pool_options:\n{client}"
    );
    assert!(
        engine_runtime.contains("class EnginePoolOptions:")
            && engine_runtime.contains("--max-connections")
            && engine_runtime.contains("--disable-idle-timeout")
            && engine_runtime.contains("--test-before-acquire")
            && engine_runtime.contains("--statement-cache-capacity"),
        "expected Python runtime engine to forward pool options to the CLI:\n{engine_runtime}"
    );
}

#[test]
fn test_generated_python_and_js_cud_event_api() {
    let ir = validate(USER_SCHEMA);

    let py_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let py_user = generated_python_file(&py_models, "user.py");
    let py_runtime = python_runtime_files();
    let py_events_runtime = generated_named_file(&py_runtime, "_events.py");

    assert!(
        py_user.contains("__nautilus_model_name__")
            && py_user.contains("def onDelete")
            && py_user.contains("UserCreateEventContext = CrudEventContext")
            && py_user.contains("Callable[[\"UserCreateEventContext\"], Any]")
            && py_user.contains("model_event_decorator(cls, \"delete\"")
            && py_user.contains("priority: int = 0")
            && py_user.contains("run_crud_event(_before_ctx)")
            && py_user.contains(
                "resolve_stop_result(_stop, default_cud_result(\"deleteMany\", return_data))"
            ),
        "expected generated Python model to expose and run CUD events:\n{py_user}"
    );
    assert!(
        py_events_runtime.contains("class StopPropagation")
            && py_events_runtime.contains("class EventPhase")
            && py_events_runtime.contains("class CrudEventContext(")
            && py_events_runtime.contains("Generic[ModelT, OperationT")
            && py_events_runtime.contains("CrudEventHandler")
            && py_events_runtime.contains("handle_stop_propagation: bool = True")
            && py_events_runtime.contains("normalize_event_priority")
            && py_events_runtime
                .contains("handlers.sort(key=lambda registered: registered.priority, reverse=True)"),
        "expected Python event runtime to expose phases, context, and StopPropagation:\n{py_events_runtime}"
    );

    let (js_models, js_dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_user = generated_named_file(&js_models, "user.js");
    let js_user_dts = generated_named_file(&js_dts_models, "user.d.ts");
    let (js_client, js_client_dts) = generate_js_client(&ir.models, "schema.nautilus")
        .expect("generate_js_client should succeed");
    let js_runtime = js_runtime_files();
    let js_events_runtime = generated_named_file(&js_runtime, "_events.js");
    let js_events_dts = generated_named_file(&js_runtime, "_events.d.ts");

    assert!(
        js_user.contains("export const User = createModelEvents('User')")
            && js_user.contains("runCrudEvent(_UserEventContext")
            && js_user
                .contains("resolveStopResult(stop, defaultCrudResult('deleteMany', returnData))"),
        "expected generated JS model to expose and run CUD events:\n{js_user}"
    );
    assert!(
        js_user_dts.contains("export declare const User: ModelEventToken")
            && js_user_dts.contains("UserCreateEventContext = CrudEventContext")
            && js_user_dts.contains("UserEventContexts extends ModelEventContexts")
            && js_user_dts.contains("data: UserCreateInput"),
        "expected generated JS declarations to type the model event token:\n{js_user_dts}"
    );
    assert!(
        js_client.contains("export { EventPhase, StopPropagation }")
            && js_client.contains("export * from './models/index.js'")
            && js_client_dts.contains("CrudEventContext")
            && js_client_dts.contains("EventPhaseValue")
            && js_client_dts.contains("ModelEventToken"),
        "expected JS root client to re-export events and model tokens:\n{js_client}\n\n{js_client_dts}"
    );
    assert!(
        js_events_runtime.contains("class StopPropagation")
            && js_events_runtime.contains("createModelEvents")
            && js_events_runtime.contains("runCrudEvent")
            && js_events_runtime.contains("model_name")
            && js_events_runtime.contains("normalizeEventPriority")
            && js_events_runtime
                .contains("handlers.sort((left, right) => right.priority - left.priority)")
            && js_events_dts.contains("result?: TResult")
            && js_events_dts.contains("interface EventPriorityOptions")
            && js_events_dts.contains("interface ModelEventToken"),
        "expected JS event runtime to expose phases, context, and StopPropagation:\n{js_events_runtime}\n\n{js_events_dts}"
    );
}

#[test]
fn test_generated_java_cud_event_api() {
    let ir = validate(JAVA_CLIENT_SCHEMA);

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let on_create = generated_java_file(&java_files, "events/OnCreate.java");
    let context = generated_java_file(&java_files, "events/CrudEventContext.java");
    let stop = generated_java_file(&java_files, "events/StopPropagation.java");
    let options = generated_java_file(&java_files, "client/NautilusOptions.java");
    let nautilus = generated_java_file(&java_files, "client/Nautilus.java");
    let delegate = generated_java_file(&java_files, "client/UserDelegate.java");
    let registry = generated_java_file(&java_files, "internal/EventRegistry.java");

    assert!(
        on_create.contains("@Retention(RetentionPolicy.RUNTIME)")
            && on_create.contains("public @interface OnCreate")
            && on_create.contains("Class<?> value();")
            && on_create.contains("EventPhase phase() default EventPhase.BEFORE;")
            && on_create.contains("int priority() default 0;"),
        "expected generated Java @OnCreate annotation to be runtime-visible and model-scoped:\n{on_create}"
    );
    assert!(
        context.contains("public final class CrudEventContext")
            && context.contains("private final Map<String, Object> args;")
            && context.contains("private final String transactionId;")
            && context.contains("private final Map<String, Object> state;"),
        "expected generated Java event context to expose args, transaction id, and shared state:\n{context}"
    );
    assert!(
        stop.contains("public final class StopPropagation extends RuntimeException")
            && stop.contains("public Object result()"),
        "expected generated Java StopPropagation runtime type:\n{stop}"
    );
    assert!(
        options.contains("public NautilusOptions eventPackages(String... packageNames)")
            && options.contains("public List<String> eventPackages()")
            && options.contains("Collections.unmodifiableList(this.eventPackages)"),
        "expected NautilusOptions to expose opt-in event package scanning:\n{options}"
    );
    assert!(
        nautilus.contains("eventRegistry().registerAnnotatedPackages(options().eventPackages().toArray(String[]::new));"),
        "expected Nautilus client construction to register configured event packages:\n{nautilus}"
    );
    assert!(
        delegate.contains("events().run(eventContext(\"create\", EventPhase.BEFORE")
            && delegate.contains("events().run(eventContext(\"create\", EventPhase.AFTER")
            && delegate.contains("events().run(eventContext(\"update\", EventPhase.BEFORE")
            && delegate.contains("events().run(eventContext(\"deleteMany\", EventPhase.ERROR"),
        "expected Java delegate mutations to run before/after/error CRUD events:\n{delegate}"
    );
    assert!(
        registry.contains("public void registerAnnotatedPackages(String... packageNames)")
            && registry.contains("scanDirectory(classes, loader")
            && registry.contains("scanJar(classes, loader")
            && registry.contains("Modifier.isStatic(method.getModifiers())")
            && registry.contains("candidate.getConstructor()")
            && registry.contains("constructor.newInstance()")
            && registry.contains("normalizePriority(")
            && registry.contains("registered.sort((left, right) -> Integer.compare(right.priority(), left.priority()))")
            && registry.contains("catch (StopPropagation stop)"),
        "expected Java EventRegistry to scan configured packages and invoke static/no-arg instance handlers:\n{registry}"
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
fn test_python_hydrates_relation_json_payloads_recursively() {
    let ir = validate(BLOG_RELATIONS_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, user_code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");
    let (_, post_code) = models
        .iter()
        .find(|(name, _)| name == "post.py")
        .expect("post model missing");
    let (_, comment_code) = models
        .iter()
        .find(|(name, _)| name == "comment.py")
        .expect("comment model missing");

    assert!(
        user_code.contains(r#"_get_wire_value(row, "users__display_name", "displayName")"#),
        "expected Python hydration to read nested logical scalar keys for mapped fields:\n{user_code}"
    );
    assert!(
        user_code.contains(r#"kwargs["display_name"] = _coerce_user_scalar("display_name", value)"#),
        "expected Python hydration to map logical scalar keys back to snake_case model fields:\n{user_code}"
    );
    assert!(
        post_code.contains(r#"relation_value = _get_wire_value(row, "author_json")"#),
        "expected Python hydration to read relation JSON columns on nested models"
    );
    assert!(
        post_code.contains(r#"from .user import _user_from_wire"#),
        "expected Python nested include hydration to recurse into related models"
    );
    assert!(
        comment_code.contains(r#"relation_value = _get_wire_value(row, "post_json")"#)
            && comment_code.contains(r#"relation_value = _get_wire_value(row, "user_json")"#),
        "expected Python top-level include hydration to read multiple relation JSON columns:\n{comment_code}"
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
        code.contains("result[db_key] = _serialize_scalar_input(key, value)"),
        "expected composite payload serialization to flow through _serialize_scalar_input:\n{code}"
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

#[test]
fn test_js_hydrates_relation_json_payloads_recursively() {
    let ir = validate(BLOG_RELATIONS_SCHEMA);
    let (js_models, _dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let (_, user_code) = js_models
        .iter()
        .find(|(name, _)| name == "user.js")
        .expect("user runtime missing");
    let (_, post_code) = js_models
        .iter()
        .find(|(name, _)| name == "post.js")
        .expect("post runtime missing");
    let (_, comment_code) = js_models
        .iter()
        .find(|(name, _)| name == "comment.js")
        .expect("comment runtime missing");

    assert!(
        user_code
            .contains("const value = _getWireValue(row, 'users__display_name', 'displayName');"),
        "expected JS hydration to read nested logical scalar keys for mapped fields:\n{user_code}"
    );
    assert!(
        post_code.contains("  _coerceUser as _coerceUser_for_author,")
            && post_code
                .contains("  _serializeUserIncludeArgs as _serializeUserIncludeArgs_for_author,")
            && post_code.contains("} from './user.js';"),
        "expected JS nested include hydration to import the related model's coercer and include serializer:
{post_code}"
    );
    assert!(
        post_code.contains("const relationValue = _getWireValue(row, 'author_json');"),
        "expected JS hydration to read relation JSON columns on nested models"
    );
    assert!(
        comment_code.contains("const relationValue = _getWireValue(row, 'post_json');")
            && comment_code.contains("const relationValue = _getWireValue(row, 'user_json');"),
        "expected JS top-level include hydration to read multiple relation JSON columns:\n{comment_code}"
    );
}

#[test]
fn test_python_select_input_supports_projection_safe_models() {
    let ir = validate(USER_MAPPED_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");

    assert!(
        code.contains("display_name: str"),
        "expected generated Python models to keep required schema fields required:\n{code}"
    );
    assert!(
        code.contains("class UserProjection(TypedDict, total=False):")
            && code.contains("display_name: NotRequired[str]"),
        "expected generated Python projection type to allow missing selected fields:\n{code}"
    );
    assert!(
        code.contains("class UserSelectInput(TypedDict, total=False):"),
        "expected a typed UserSelectInput to be generated:\n{code}"
    );
    assert!(
        code.contains("display_name: NotRequired[bool]"),
        "expected select input to expose the Python model field name:\n{code}"
    );
    assert!(
        code.contains("\"display_name\": \"displayName\""),
        "expected select serialization to map Python field names back to logical names:\n{code}"
    );
    assert!(
        code.contains("args[\"select\"] = _process_select_fields(select, _users_py_to_logical)"),
        "expected find_many() to forward select through the logical-name serializer:\n{code}"
    );
}

#[test]
fn test_python_find_many_exposes_chunk_size() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");

    assert!(
        code.contains("chunk_size: Optional[int] = None"),
        "expected generated Python find_many() to expose chunk_size:\n{code}"
    );
    assert!(
        code.contains("payload[\"chunkSize\"] = chunk_size"),
        "expected generated Python find_many() to forward chunk_size as protocol chunkSize:\n{code}"
    );
}

#[test]
fn test_python_async_delegate_exposes_stream_many() {
    let ir = validate(USER_SCHEMA);
    let async_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let sync_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let async_code = generated_python_file(&async_models, "user.py");
    let sync_code = generated_python_file(&sync_models, "user.py");

    assert!(
        async_code.contains("async def stream_many("),
        "expected generated async Python delegate to expose stream_many():\n{async_code}"
    );
    assert!(
        async_code.contains(") -> AsyncIterator[User]:"),
        "expected generated async Python stream_many() to return an AsyncIterator:\n{async_code}"
    );
    assert!(
        async_code.contains(
            "async for chunk in self._client._stream_rpc(\"query.findMany\", payload):"
        ),
        "expected generated async Python stream_many() to consume chunked RPC frames:\n{async_code}"
    );
    assert!(
        async_code.contains("\"chunkSize\": chunk_size"),
        "expected generated async Python stream_many() to force protocol chunking:\n{async_code}"
    );
    assert!(
        !sync_code.contains("def stream_many("),
        "stream_many should not be emitted for sync Python clients:\n{sync_code}"
    );
}

#[test]
fn test_python_single_row_finds_use_dedicated_engine_methods() {
    let ir = validate(USER_SCHEMA);
    let py_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");

    assert!(
        py_model.contains(r#""query.findFirst", payload"#),
        "expected generated Python find_first() to call query.findFirst:\n{py_model}"
    );
    assert!(
        py_model.contains("from .._internal.protocol import PROTOCOL_VERSION")
            && py_model.contains("\"protocolVersion\": PROTOCOL_VERSION"),
        "expected generated Python delegates to reuse the shared protocol version constant:\n{py_model}"
    );
    assert!(
        py_model.contains(r#""query.findUnique", payload"#),
        "expected generated Python find_unique() to call query.findUnique when possible:\n{py_model}"
    );
    assert!(
        py_model.contains("if select is not None or include is not None:"),
        "expected generated Python find_unique() to fall back to the single-row projection path when select/include are used:\n{py_model}"
    );
    assert!(
        !py_model.contains("rows = self.find_many(where=where, order_by=order_by, take=1, select=select, include=include)"),
        "generated Python find_first() should no longer delegate to find_many():\n{py_model}"
    );
    assert!(
        !py_model
            .contains("rows = self.find_many(where=where, take=1, select=select, include=include)"),
        "generated Python find_unique() should no longer delegate to find_many():\n{py_model}"
    );
}

#[test]
fn test_js_select_input_supports_projection_safe_models() {
    let ir = validate(USER_MAPPED_SCHEMA);
    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let (_, dts_code) = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .expect("user declaration missing");
    let (_, js_code) = js_models
        .iter()
        .find(|(name, _)| name == "user.js")
        .expect("user runtime missing");

    assert!(
        dts_code.contains("displayName: string;"),
        "expected generated JS models to keep required schema fields required:\n{dts_code}"
    );
    assert!(
        dts_code.contains("export type UserSelected<S extends UserSelectInput>"),
        "expected generated JS declarations to expose select result mapped types:\n{dts_code}"
    );
    assert!(
        dts_code.contains("export interface UserSelectInput {"),
        "expected a typed UserSelectInput to be generated:\n{dts_code}"
    );
    assert!(
        dts_code.contains("displayName?: boolean;"),
        "expected select input to expose logical field names:\n{dts_code}"
    );
    assert!(
        dts_code.contains("select?:   UserSelectInput;"),
        "expected select to be exposed on generated query methods:\n{dts_code}"
    );
    assert!(
        js_code.contains("if (args?.select   != null) rpcArgs['select']  = args.select;"),
        "expected runtime delegate to forward select to the engine:\n{js_code}"
    );
}

#[test]
fn test_js_find_many_exposes_chunk_size() {
    let ir = validate(USER_SCHEMA);
    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let (_, dts_code) = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .expect("user declaration missing");
    let (_, js_code) = js_models
        .iter()
        .find(|(name, _)| name == "user.js")
        .expect("user runtime missing");

    assert!(
        dts_code.contains("chunkSize?: number;"),
        "expected generated JS findMany() typings to expose chunkSize:\n{dts_code}"
    );
    assert!(
        js_code.contains("if (args?.chunkSize != null) request['chunkSize'] = args.chunkSize;"),
        "expected generated JS findMany() to forward chunkSize at the protocol level:\n{js_code}"
    );
}

#[test]
fn test_js_async_delegate_exposes_stream_many() {
    let ir = validate(USER_SCHEMA);
    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_code = generated_named_file(&js_models, "user.js");
    let dts_code = generated_named_file(&dts_models, "user.d.ts");

    assert!(
        dts_code.contains("streamMany(args?: Omit<UserFindManyArgs, 'select'>"),
        "expected generated JS typings to expose streamMany():\n{dts_code}"
    );
    assert!(
        dts_code.contains("): AsyncIterable<UserModel>;"),
        "expected generated JS streamMany() typings to return an AsyncIterable:\n{dts_code}"
    );
    assert!(
        js_code.contains("async *streamMany(args) {"),
        "expected generated JS delegate to expose streamMany():\n{js_code}"
    );
    assert!(
        js_code.contains(
            "for await (const chunk of this.client._streamRpc('query.findMany', payload)) {"
        ),
        "expected generated JS streamMany() to consume chunked RPC frames:\n{js_code}"
    );
    assert!(
        js_code.contains("chunkSize,"),
        "expected generated JS streamMany() to force protocol chunking:\n{js_code}"
    );
}

#[test]
fn test_js_single_row_finds_use_dedicated_engine_methods() {
    let ir = validate(USER_SCHEMA);
    let (js_models, _dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_model = generated_named_file(&js_models, "user.js");

    assert!(
        js_model.contains("this.client._rpc('query.findFirst', request)"),
        "expected generated JS findFirst() to call query.findFirst:\n{js_model}"
    );
    assert!(
        js_model.contains("import { PROTOCOL_VERSION } from '../_internal/_protocol.js';")
            && js_model.contains("protocolVersion: PROTOCOL_VERSION"),
        "expected generated JS delegates to reuse the shared protocol version constant:\n{js_model}"
    );
    assert!(
        js_model.contains("this.client._rpc('query.findUnique', request)"),
        "expected generated JS findUnique() to call query.findUnique when possible:\n{js_model}"
    );
    assert!(
        js_model.contains("if (args.select != null || args.include != null)"),
        "expected generated JS findUnique() to fall back to the single-row projection path when select/include are used:\n{js_model}"
    );
    assert!(
        !js_model.contains("const rows = await this.findMany({ where: args?.where, orderBy: args?.orderBy, take: 1, select: args?.select, include: args?.include });"),
        "generated JS findFirst() should no longer delegate to findMany():\n{js_model}"
    );
    assert!(
        !js_model.contains("const rows = await this.findMany({ where: args.where, take: 1, select: args.select, include: args.include });"),
        "generated JS findUnique() should no longer delegate to findMany():\n{js_model}"
    );
}

#[test]
fn test_js_runtime_exposes_engine_pool_options() {
    let ir = validate(USER_SCHEMA);
    let (_client_js, client_dts) = generate_js_client(&ir.models, "schema.nautilus")
        .expect("generate_js_client should succeed");
    let runtime = js_runtime_files();
    let client_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_client.d.ts")
        .expect("missing JS runtime client declarations")
        .1
        .as_str();
    let engine_runtime_dts = runtime
        .iter()
        .find(|(name, _)| name == "_engine.d.ts")
        .expect("missing JS runtime engine declarations")
        .1
        .as_str();
    let engine_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_engine.js")
        .expect("missing JS runtime engine")
        .1
        .as_str();

    assert!(
        client_dts.contains("constructor(options?: NautilusClientOptions);")
            && client_runtime.contains("pool?: EnginePoolOptions;"),
        "expected generated JS declarations to expose engine pool options:\n{client_dts}"
    );
    assert!(
        engine_runtime_dts.contains("export interface EnginePoolOptions")
            && engine_runtime.contains("--max-connections")
            && engine_runtime.contains("--disable-idle-timeout")
            && engine_runtime.contains("--test-before-acquire")
            && engine_runtime.contains("--statement-cache-capacity"),
        "expected JS runtime engine to forward pool options to the CLI:\n{engine_runtime}"
    );
}

/// The field list of a generated Rust struct, without its surrounding items.
fn struct_body<'a>(code: &'a str, name: &str) -> &'a str {
    let header = format!("pub struct {name} {{");
    let start = code
        .find(&header)
        .unwrap_or_else(|| panic!("missing generated struct '{name}'"))
        + header.len();
    let end = code[start..]
        .find("\n}")
        .unwrap_or_else(|| panic!("unterminated generated struct '{name}'"));
    &code[start..start + end]
}

const NESTED_WRITE_RUST_SCHEMA: &str = r#"
model Author {
  id    Int    @id @default(autoincrement())
  email String @unique
  books Book[]
}

model Book {
  id       Int     @id @default(autoincrement())
  title    String  @unique
  authorId Int?    @map("author_id")
  author   Author? @relation(fields: [authorId], references: [id])
}
"#;

#[test]
fn test_rust_create_input_carries_nested_writes_for_both_relation_sides() {
    let ir = validate(NESTED_WRITE_RUST_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let author = models.get("Author").expect("Author model missing");
    let book = models.get("Book").expect("Book model missing");

    assert!(
        author.contains("pub books: AuthorBooksCreateNested,"),
        "expected the create input to carry the relation"
    );
    assert!(
        author.contains("pub create_many: Vec<BookCreateInput>,")
            && author.contains("pub connect: Vec<nautilus_core::Expr>,"),
        "expected the inverse side to take lists of operations"
    );
    assert!(
        author.contains("pub set: Vec<nautilus_core::Expr>,")
            && author.contains("pub update_many: Vec<crate::NestedUpdate<BookUpdateInput>>,"),
        "expected the update-only operations only on the update input"
    );
    let create_nested = struct_body(author, "AuthorBooksCreateNested");
    assert!(
        !create_nested.contains("pub set:") && !create_nested.contains("pub delete_many:"),
        "expected the create input to stop short of the update-only operations"
    );

    assert!(
        book.contains("pub create: Option<Box<AuthorCreateInput>>,"),
        "expected the owning side to box its single related input"
    );
    assert!(
        book.contains("pub disconnect: bool,") && book.contains("pub delete: bool,"),
        "expected the owning side to take a flag where the inverse side takes filters"
    );
    assert!(
        book.contains("use super::AuthorCreateInput;")
            && book.contains("use super::AuthorUpdateInput;"),
        "expected the model file to import the target input types"
    );
}

#[test]
fn test_rust_nested_writes_route_through_the_engine() {
    let ir = validate(NESTED_WRITE_RUST_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let author = models.get("Author").expect("Author model missing");

    assert!(
        author.contains("data.has_nested_writes(),"),
        "expected create to tell the engine helper that it cannot fall back"
    );
    assert!(
        author.contains("args.data.has_nested_writes(),"),
        "expected update to tell the engine helper that it cannot fall back"
    );
    assert!(
        author.contains("return Err(crate::runtime::nested::writes_need_engine(\"Author\"));"),
        "expected the connector fallback to refuse a nested write"
    );
    assert!(
        author.contains("upsert on 'Author' does not accept nested writes"),
        "expected upsert to refuse nested writes"
    );
}

#[test]
fn test_rust_model_without_relations_has_no_nested_write_types() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let user = models.get("User").expect("User model missing");

    assert!(
        !user.contains("CreateNested"),
        "expected no nested-write types for a model with no relations"
    );
    assert!(
        user.contains("pub(crate) fn has_nested_writes(&self) -> bool {\n        false\n    }"),
        "expected the gate to fold to a constant"
    );
}

#[test]
fn test_java_dsl_exposes_nested_writes_for_both_relation_sides() {
    let ir = validate(
        r#"
generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
}

model Author {
  id    Int    @id @default(autoincrement())
  email String @unique
  books Book[]
}

model Book {
  id       Int     @id @default(autoincrement())
  title    String  @unique
  authorId Int?    @map("author_id")
  author   Author? @relation(fields: [authorId], references: [id])
}
"#,
    );
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let author_dsl = generated_java_file(&files, "dsl/AuthorDsl.java");
    let book_dsl = generated_java_file(&files, "dsl/BookDsl.java");

    assert!(
        author_dsl.contains("public CreateInput books(Consumer<BooksCreateNested> spec)")
            && author_dsl.contains("public UpdateInput books(Consumer<BooksUpdateNested> spec)"),
        "expected the inputs to take the relation builder"
    );
    assert!(
        author_dsl.contains("public BooksCreateNested create(Consumer<BookDsl.CreateInput> spec)"),
        "expected a nested create taking the target model input"
    );
    assert!(
        author_dsl.contains("public BooksUpdateNested deleteMany(Consumer<BookDsl.Where> spec)"),
        "expected the inverse side to expose the update-only operations"
    );
    assert!(
        !author_dsl.contains("public BooksCreateNested deleteMany(Consumer<BookDsl.Where> spec)"),
        "expected the create input to stop short of the update-only operations"
    );

    assert!(
        book_dsl.contains("public AuthorUpdateNested disconnect() {"),
        "expected the owning side to disconnect without a filter"
    );
    assert!(
        book_dsl.contains("this.node.set(\"create\", input(spec));"),
        "expected the owning side to set one operation instead of appending"
    );
}

#[test]
fn test_java_sync_generation_exposes_model_delegate_and_autoregister_accessor() {
    let ir = validate(
        r#"
generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

enum Role {
  ADMIN
  MEMBER
}

model User {
  id   Int    @id @default(autoincrement())
  name String
  role Role
}
"#,
    );
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let user_model = generated_java_file(&files, "model/User.java");
    let nautilus_client = generated_java_file(&files, "client/Nautilus.java");

    assert!(
        user_model.contains("public static UserDelegate nautilus()"),
        "expected generated Java model to expose static nautilus() accessor:\n{user_model}"
    );
    assert!(
        user_model.contains("GlobalNautilusRegistry.require()"),
        "expected generated Java model to resolve the auto-registered client:\n{user_model}"
    );
    assert!(
        nautilus_client.contains("GlobalNautilusRegistry.register(this);"),
        "expected generated Java client to auto-register itself when configured:\n{nautilus_client}"
    );

    assert_local_snapshot!("java_user_model_sync", user_model);
}

#[test]
fn test_java_async_generation_exposes_completable_future_transaction_api() {
    let ir = validate(JAVA_CLIENT_ASYNC_SCHEMA);
    let files =
        generate_java_client(&ir, "schema.nautilus", true).expect("generate_java_client failed");
    let delegate = generated_java_file(&files, "client/UserDelegate.java");
    let nautilus_client = generated_java_file(&files, "client/Nautilus.java");

    assert!(
        delegate.contains("CompletableFuture<List<User>> findMany()"),
        "expected generated Java async delegate to expose CompletableFuture APIs:\n{delegate}"
    );
    assert!(
        nautilus_client.contains(
            "public <T> CompletableFuture<T> transaction(Function<TransactionClient, CompletableFuture<T>> callback)"
        ),
        "expected generated Java async client to expose CompletableFuture transaction API:\n{nautilus_client}"
    );

    assert_local_snapshot!("java_nautilus_async", nautilus_client);
}

#[test]
fn test_java_generation_exposes_stream_many_over_chunked_rpc() {
    let ir = validate(JAVA_CLIENT_ASYNC_SCHEMA);
    let async_files =
        generate_java_client(&ir, "schema.nautilus", true).expect("generate_java_client failed");
    let sync_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");

    let async_delegate = generated_java_file(&async_files, "client/UserDelegate.java");
    let sync_delegate = generated_java_file(&sync_files, "client/UserDelegate.java");
    let dsl = generated_java_file(&async_files, "dsl/UserDsl.java");
    let rpc_caller = generated_java_file(&async_files, "internal/RpcCaller.java");
    let base_client = generated_java_file(&async_files, "internal/BaseNautilusClient.java");
    let base_tx_client = generated_java_file(&async_files, "internal/BaseTransactionClient.java");

    assert!(
        async_delegate.contains("public Stream<User> streamMany()")
            && sync_delegate.contains("public Stream<User> streamMany()"),
        "expected generated Java delegates to expose streamMany():\nasync:\n{async_delegate}\n\nsync:\n{sync_delegate}"
    );
    assert!(
        async_delegate.contains("DEFAULT_STREAM_CHUNK_SIZE = 128")
            && async_delegate.contains("streamMany chunkSize must be a positive integer")
            && async_delegate.contains(
                "return rows(streamRpc(\"query.findMany\", request), User::fromJsonNode);"
            ),
        "expected generated Java async delegate to stream chunked findMany rows:\n{async_delegate}"
    );
    assert!(
        rpc_caller.contains("Stream<JsonNode> streamRpc(String method, ObjectNode params);"),
        "expected Java RpcCaller to expose streamRpc():\n{rpc_caller}"
    );
    assert!(
        dsl.contains("public ObjectNode whereNode()")
            && dsl.contains("values.add(orderBy.node());"),
        "expected Java FindManyArgs to expose whereNode() and serialize orderBy as an array:\n{dsl}"
    );
    assert!(
        base_client
            .contains("private final Map<Long, StreamState> streams = new ConcurrentHashMap<>();")
            && base_client.contains("request.put(\"method\", \"request.cancel\");")
            && base_client.contains(
                "return StreamSupport.stream(spliterator, false).onClose(cursor::close);"
            ),
        "expected Java runtime to stream chunked responses and cancel early closes:\n{base_client}"
    );
    assert!(
        base_tx_client.contains("return this.parent.streamRpc(method, actual);"),
        "expected transaction clients to forward streamRpc() through the parent client:\n{base_tx_client}"
    );
}

#[test]
fn test_java_runtime_loads_dotenv_before_spawning_engine() {
    let ir = validate(JAVA_CLIENT_SYNC_SCHEMA);
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let engine_process = generated_java_file(&files, "internal/EngineProcess.java");

    assert!(
        engine_process.contains("loadDotenv(builder.environment(), schemaPath);"),
        "expected generated Java runtime to load .env before starting the engine:\n{engine_process}"
    );
    assert!(
        engine_process.contains("Path candidate = root.resolve(\".env\");"),
        "expected generated Java runtime to search for .env files near the schema:\n{engine_process}"
    );
    assert!(
        engine_process.contains("environment.putIfAbsent(key, value);"),
        "expected generated Java runtime to preserve pre-existing environment variables:\n{engine_process}"
    );
    assert!(
        engine_process.contains("Optional<String> localBinary = findLocalBinary(schemaPath);"),
        "expected generated Java runtime to prefer a local nautilus binary before PATH lookup:\n{engine_process}"
    );
}

#[test]
fn test_java_runtime_exposes_engine_pool_options() {
    let ir = validate(JAVA_CLIENT_SYNC_SCHEMA);
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let options = generated_java_file(&files, "client/NautilusOptions.java");
    let engine_process = generated_java_file(&files, "internal/EngineProcess.java");

    assert!(
        options.contains("public NautilusOptions maxConnections(Integer maxConnections)")
            && options
                .contains("public NautilusOptions disableIdleTimeout(boolean disableIdleTimeout)")
            && options.contains("public Boolean testBeforeAcquire()"),
        "expected generated Java options to expose engine pool settings:\n{options}"
    );
    assert!(
        engine_process.contains("command.add(\"--max-connections\");")
            && engine_process.contains("command.add(\"--disable-idle-timeout\");")
            && engine_process.contains("command.add(\"--test-before-acquire\");"),
        "expected generated Java runtime engine to forward pool options to the CLI:\n{engine_process}"
    );
}

#[test]
fn test_generated_clients_exclude_non_orderable_fields_from_order_by() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore, vector]
}

generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

model User {
  id      Int      @id @default(autoincrement())
  title   String
  active  Boolean
  meta    Hstore?
  payload Json?
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
    assert!(js_dts.contains("title?: SortOrder;"));
    assert!(!js_dts.contains("active?: SortOrder;"));
    assert!(!js_dts.contains("meta?: SortOrder;"));
    assert!(!js_dts.contains("payload?: SortOrder;"));
    assert!(!js_dts.contains("embedding?: SortOrder;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("title: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("active: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("meta: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("payload: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("embedding: NotRequired[Literal[\"asc\", \"desc\"]]"));

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let user_dsl = generated_java_file(&java_files, "dsl/UserDsl.java");
    assert!(user_dsl.contains("public OrderBy title(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy active(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy meta(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy payload(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy embedding(SortOrder order)"));
}

#[test]
fn test_generated_clients_type_composite_field_order_by_paths() {
    let ir = validate(
        r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

type DeliveryEstimate {
  etaMinutes      Int @map("eta_minutes_db")
  weekendDelivery Boolean
  carrierMetadata Json
}

model Shipment {
  id           Int              @id @default(autoincrement())
  trackingCode String           @unique
  delivery     DeliveryEstimate @store(json)
}
"#,
    );

    let rust_models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let rust_shipment = rust_models.get("Shipment").expect("Shipment model missing");
    assert!(rust_shipment
        .contains("pub fn delivery_eta_minutes(&self) -> nautilus_core::OrderField<i32>"));
    assert!(rust_shipment
        .contains("nautilus_core::OrderField::new(\"Shipment\", \"delivery.etaMinutes\")"));
    assert!(rust_shipment.contains("nautilus_core::JsonPathCast::Signed"));
    assert!(!rust_shipment
        .contains("pub fn delivery_weekend_delivery(&self) -> nautilus_core::OrderField"));
    assert!(!rust_shipment
        .contains("pub fn delivery_carrier_metadata(&self) -> nautilus_core::OrderField"));

    let (_, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_dts = dts_models
        .iter()
        .find(|(name, _)| name == "shipment.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("shipment declaration missing");
    assert!(js_dts.contains("'delivery.etaMinutes'?: SortOrder;"));
    assert!(!js_dts.contains("delivery?: SortOrder;"));
    assert!(!js_dts.contains("'delivery.weekendDelivery'?: SortOrder;"));
    assert!(!js_dts.contains("'delivery.carrierMetadata'?: SortOrder;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "shipment.py");
    assert!(py_model.contains("ShipmentOrderByInput = TypedDict("));
    assert!(py_model.contains("\"delivery.etaMinutes\": NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("delivery: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(
        !py_model.contains("\"delivery.weekendDelivery\": NotRequired[Literal[\"asc\", \"desc\"]]")
    );
    assert!(
        !py_model.contains("\"delivery.carrierMetadata\": NotRequired[Literal[\"asc\", \"desc\"]]")
    );

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let shipment_dsl = generated_java_file(&java_files, "dsl/ShipmentDsl.java");
    assert!(shipment_dsl.contains("public OrderBy deliveryEtaMinutes(SortOrder order)"));
    assert!(shipment_dsl.contains("this.node.put(\"delivery.etaMinutes\", order.wireValue());"));
    assert!(!shipment_dsl.contains("public OrderBy delivery(SortOrder order)"));
    assert!(!shipment_dsl.contains("public OrderBy deliveryWeekendDelivery(SortOrder order)"));
    assert!(!shipment_dsl.contains("public OrderBy deliveryCarrierMetadata(SortOrder order)"));
}

#[test]
fn test_generated_hstore_filters_are_typed_in_js_and_python() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore]
}

model User {
  id   Int     @id @default(autoincrement())
  meta Hstore?
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
    assert!(js_dts.contains("export interface HstoreFilter {"));
    assert!(js_dts.contains("export type HstoreValue = Record<string, string | null>;"));
    // With the `hstore` extension declared, filter inputs accept the generated
    // wrapper or the raw `HstoreValue` payload via the `HstoreInput` union.
    assert!(js_dts.contains("equals?: HstoreInput;"));
    assert!(js_dts.contains("not?:    HstoreInput;"));
    assert!(js_dts.contains("isNull?: boolean;"));
    assert!(js_dts.contains("meta?: HstoreInput | HstoreFilter | null;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("HstoreValue = Dict[str, Optional[str]]"));
    assert!(py_model.contains("class HstoreFilter(TypedDict, total=False):"));
    // With the `hstore` extension declared the filter accepts the wrapper too.
    assert!(py_model.contains("equals: NotRequired[HstoreInput]"));
    assert!(py_model.contains("not_: NotRequired[HstoreInput]"));
    assert!(py_model.contains("is_null: NotRequired[bool]"));
    assert!(py_model.contains("meta: NotRequired[Union[HstoreInput, HstoreFilter, None]]"));
}

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
fn test_extension_input_builders_are_generated_across_codegens() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext, hstore, ltree, postgis, vector]
}

model Example {
  id          Int        @id @default(autoincrement())
  email       Citext
  path        Ltree?
  meta        Hstore?
  footprint   Geometry?
  serviceArea Geography?
  embedding   Vector(3)
}
"#,
    );

    let extensions = ExtensionRegistry::from_schema(&ir);

    let py_ext_files = generate_python_extension_files(&extensions)
        .expect("generate_python_extension_files should succeed");
    let citext_py = generated_named_file(&py_ext_files, "citext/types.py");
    let hstore_py = generated_named_file(&py_ext_files, "hstore/types.py");
    let ltree_py = generated_named_file(&py_ext_files, "ltree/types.py");
    let postgis_py = generated_named_file(&py_ext_files, "postgis/types.py");
    let vector_py = generated_named_file(&py_ext_files, "vector/types.py");
    assert!(citext_py.contains("CitextInput = Union[\"Citext\", str, CitextBuilderInput]"));
    assert!(citext_py.contains("class CitextValueInput(TypedDict):"));
    assert!(ltree_py.contains("LtreeInput = Union[\"Ltree\", str, LtreeBuilderInput]"));
    assert!(hstore_py
        .contains("HstoreInput = Union[\"Hstore\", HstoreSource, HstoreEntriesBuilderInput]"));
    assert!(hstore_py.contains("class HstoreEntriesBuilderInput(TypedDict):"));
    assert!(postgis_py.contains("class GeometryPointInput(TypedDict, total=False):"));
    assert!(postgis_py.contains("class GeographyPointInput(TypedDict, total=False):"));
    assert!(postgis_py.contains("GeometryInput = Union[\"Geometry\", str, GeometryBuilderInput]"));
    assert!(
        postgis_py.contains("GeographyInput = Union[\"Geography\", str, GeographyBuilderInput]")
    );
    assert!(vector_py.contains("class VectorValuesInput(TypedDict):"));
    assert!(vector_py.contains("VectorInput = Union[\"Vector\", VectorSource, VectorValuesInput]"));

    let (_, js_ext_dts) = generate_js_extension_files(&extensions)
        .expect("generate_js_extension_files should succeed");
    let citext_dts = generated_named_file(&js_ext_dts, "extensions/citext/types.d.ts");
    let hstore_dts = generated_named_file(&js_ext_dts, "extensions/hstore/types.d.ts");
    let ltree_dts = generated_named_file(&js_ext_dts, "extensions/ltree/types.d.ts");
    let postgis_dts = generated_named_file(&js_ext_dts, "extensions/postgis/types.d.ts");
    let vector_dts = generated_named_file(&js_ext_dts, "extensions/vector/types.d.ts");
    assert!(citext_dts.contains("export interface CitextValueInput {"));
    assert!(citext_dts.contains("export type CitextInput = Citext | string | CitextBuilderInput;"));
    assert!(ltree_dts.contains("export type LtreeInput = Ltree | string | LtreeBuilderInput;"));
    assert!(hstore_dts.contains("export interface HstoreEntriesBuilderInput {"));
    assert!(hstore_dts.contains("export type HstoreInput = Hstore | HstoreBuilderInput;"));
    assert!(postgis_dts.contains("export interface GeometryPointInput {"));
    assert!(postgis_dts.contains("export interface GeographyPointInput {"));
    assert!(postgis_dts
        .contains("export type GeometryInput = Geometry | string | GeometryBuilderInput;"));
    assert!(postgis_dts
        .contains("export type GeographyInput = Geography | string | GeographyBuilderInput;"));
    assert!(vector_dts.contains("export interface VectorValuesInput {"));
    assert!(vector_dts.contains("export type VectorInput = Vector | VectorBuilderInput;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "example.py");
    assert!(py_model.contains("email: Required[CitextInput]"));
    assert!(py_model.contains("path: NotRequired[Optional[LtreeInput]]"));
    assert!(py_model.contains("meta: NotRequired[Optional[HstoreInput]]"));
    assert!(py_model.contains("footprint: NotRequired[Optional[GeometryInput]]"));
    assert!(py_model.contains("serviceArea: NotRequired[Optional[GeographyInput]]"));
    assert!(py_model.contains("embedding: Required[VectorInput]"));
    assert!(py_model.contains("footprint: NotRequired[Union[GeometryInput, StringFilter, None]]"));
    assert!(
        py_model.contains("serviceArea: NotRequired[Union[GeographyInput, StringFilter, None]]")
    );
    assert!(py_model.contains("embedding: NotRequired[Union[VectorInput, VectorFilter]]"));

    let (_, js_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_model = js_models
        .iter()
        .find(|(name, _)| name == "example.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("example.d.ts missing");
    assert!(js_model.contains("email: CitextInput;"));
    assert!(js_model.contains("path?: LtreeInput | null;"));
    assert!(js_model.contains("meta?: HstoreInput | null;"));
    assert!(js_model.contains("footprint?: GeometryInput | null;"));
    assert!(js_model.contains("serviceArea?: GeographyInput | null;"));
    assert!(js_model.contains("embedding: VectorInput;"));
    assert!(js_model.contains("footprint?: GeometryInput | StringFilter | null;"));
    assert!(js_model.contains("serviceArea?: GeographyInput | StringFilter | null;"));
    assert!(js_model.contains("embedding?: VectorInput | VectorFilter;"));

    let java_ext_files = generate_java_extension_files(&extensions, "com.acme.db")
        .expect("generate_java_extension_files should succeed");
    let geometry_java = generated_java_file(&java_ext_files, "Geometry.java");
    let geography_java = generated_java_file(&java_ext_files, "Geography.java");
    let hstore_java = generated_java_file(&java_ext_files, "Hstore.java");
    let vector_java = generated_java_file(&java_ext_files, "Vector.java");
    let citext_java = generated_java_file(&java_ext_files, "Citext.java");
    let ltree_java = generated_java_file(&java_ext_files, "Ltree.java");
    assert!(citext_java.contains("public static Citext of(String value)"));
    assert!(ltree_java.contains("public static Ltree of(String value)"));
    assert!(geometry_java.contains("public static Geometry point(double x, double y)"));
    assert!(geography_java.contains("public static Geography point(double lon, double lat)"));
    assert!(hstore_java
        .contains("public static Hstore ofEntries(Map.Entry<String, String>... entries)"));
    assert!(vector_java.contains("public static Vector of(double... values)"));

    let rust_ext_files = generate_rust_extension_files(&extensions)
        .expect("generate_rust_extension_files should succeed");
    let postgis_rust = generated_named_file(&rust_ext_files, "extensions/postgis/types.rs");
    let hstore_rust = generated_named_file(&rust_ext_files, "extensions/hstore/types.rs");
    let vector_rust = generated_named_file(&rust_ext_files, "extensions/vector/types.rs");
    let citext_rust = generated_named_file(&rust_ext_files, "extensions/citext/types.rs");
    let ltree_rust = generated_named_file(&rust_ext_files, "extensions/ltree/types.rs");
    assert!(citext_rust.contains("pub fn of(value: impl Into<String>) -> Self"));
    assert!(ltree_rust.contains("pub fn of(value: impl Into<String>) -> Self"));
    assert!(postgis_rust.contains("impl Geometry {"));
    assert!(postgis_rust
        .contains("pub fn point(x: impl std::fmt::Display, y: impl std::fmt::Display) -> Self"));
    assert!(postgis_rust.contains("impl Geography {"));
    assert!(postgis_rust.contains(
        "pub fn point(lon: impl std::fmt::Display, lat: impl std::fmt::Display) -> Self"
    ));
    assert!(hstore_rust.contains("pub fn from_entries<K, V, I>(entries: I) -> Self"));
    assert!(vector_rust.contains("pub fn of<I, N>(values: I) -> Self"));
}

#[test]
fn test_python_filter_operator_names_are_normalized_for_engine() {
    let ir = validate(
        r#"
model User {
  id    Int     @id @default(autoincrement())
  title String?
}
"#,
    );

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");

    assert!(py_model.contains("\"in_\": \"in\""));
    assert!(py_model.contains("\"not_\": \"not\""));
    assert!(py_model.contains("\"not_in\": \"notIn\""));
    assert!(py_model.contains("\"startswith\": \"startsWith\""));
    assert!(py_model.contains("\"endswith\": \"endsWith\""));
    assert!(py_model.contains("\"is_null\": \"isNull\""));
}

#[test]
fn test_generated_object_like_where_values_require_explicit_equals_in_js_and_python() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore]
}

model User {
  id      Int    @id @default(autoincrement())
  payload Jsonb?
  meta    Hstore?
}
"#,
    );

    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_model = js_models
        .iter()
        .find(|(name, _)| name == "user.js")
        .map(|(_, code)| code.as_str())
        .expect("user runtime missing");
    let js_dts = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("user declaration missing");
    assert!(js_model.contains("ObjectValueDbFields = new Set(["));
    assert!(js_model.contains("_objectEqualityRequiresExplicitEquals"));
    assert!(js_model.contains("Use { equals: ... } for object equality filters."));
    assert!(js_model.contains("const actualOp = op === 'equals' ? 'eq' : op;"));
    assert!(js_dts.contains("export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];"));
    assert!(js_dts.contains("export interface JsonFilter {"));
    assert!(js_dts.contains("equals?: JsonValue;"));
    assert!(js_dts.contains("payload?: JsonScalarOrArray | JsonFilter;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("JsonValue = Union[JsonPrimitive, Dict[str, Any], List[Any]]"));
    assert!(py_model.contains("_object_value_db_fields: frozenset = frozenset({"));
    assert!(py_model.contains("_object_equality_requires_explicit_equals"));
    assert!(py_model.contains("Use {'equals': ...} for object equality filters."));
    assert!(py_model.contains("\"equals\": \"eq\""));
    assert!(py_model.contains("class JsonFilter(TypedDict, total=False):"));
    assert!(py_model.contains("equals: NotRequired[JsonValue]"));
    assert!(py_model.contains("payload: NotRequired[Union[JsonScalarOrArray, JsonFilter]]"));
}

#[test]
fn test_java_single_row_finds_use_dedicated_engine_methods() {
    let ir = validate(JAVA_CLIENT_SCHEMA);
    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let delegate = generated_java_file(&java_files, "client/UserDelegate.java");

    assert!(
        delegate.contains("JsonNode result = rpc(\"query.findFirst\", request);"),
        "expected generated Java findFirst() to call query.findFirst:\n{delegate}"
    );
    assert!(
        delegate.contains("request.put(\"protocolVersion\", JsonSupport.PROTOCOL_VERSION);"),
        "expected generated Java delegates to reuse the shared protocol version constant:\n{delegate}"
    );
    assert!(
        delegate.contains("JsonNode result = rpc(\"query.findUnique\", request);"),
        "expected generated Java findUnique() to call query.findUnique when possible:\n{delegate}"
    );
    assert!(
        delegate.contains("if (node.size() == 1 && node.has(\"where\"))"),
        "expected generated Java findUnique() to gate the unique-only fast path conservatively:\n{delegate}"
    );
    assert!(
        !delegate.contains("return findFirst(spec);"),
        "generated Java findUnique() should no longer alias directly to findFirst():\n{delegate}"
    );
}

#[test]
fn test_java_select_uses_projection_api_instead_of_model_records() {
    let ir = validate(JAVA_CLIENT_SCHEMA);
    let sync_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let async_files =
        generate_java_client(&ir, "schema.nautilus", true).expect("generate_java_client failed");
    let sync_delegate = generated_java_file(&sync_files, "client/UserDelegate.java");
    let async_delegate = generated_java_file(&async_files, "client/UserDelegate.java");
    let projection = generated_java_file(&sync_files, "model/UserProjection.java");

    assert!(
        sync_delegate.contains("public List<UserProjection> findManySelect(")
            && sync_delegate.contains("public <R> List<R> findManySelect(")
            && sync_delegate.contains("public UserProjection findFirstSelect(")
            && sync_delegate.contains("public UserProjection findUniqueSelect(")
            && sync_delegate.contains("public Stream<UserProjection> streamManySelect(")
            && sync_delegate.contains("public List<JsonNode> findManySelectRaw("),
        "expected generated Java sync delegate to expose typed projection APIs and raw escape hatches:\n{sync_delegate}"
    );
    assert!(
        async_delegate.contains("public CompletableFuture<List<UserProjection>> findManySelect(")
            && async_delegate.contains("public <R> CompletableFuture<List<R>> findManySelect(")
            && async_delegate.contains("public CompletableFuture<UserProjection> findFirstSelect(")
            && async_delegate.contains("public CompletableFuture<UserProjection> findUniqueSelect(")
            && async_delegate.contains("public CompletableFuture<List<JsonNode>> findManySelectRaw("),
        "expected generated Java async delegate to expose CompletableFuture projection APIs:\n{async_delegate}"
    );
    assert!(
        sync_delegate.contains("select returns partial rows and cannot be decoded as a full User record; use findManySelect, findFirstSelect, or findUniqueSelect instead")
            && sync_delegate.contains("select projection APIs require select(...); use findMany/findFirst/findUnique for full User records"),
        "expected generated Java delegate to reject select on model APIs and require select on projection APIs:\n{sync_delegate}"
    );
    assert!(
        sync_delegate.contains("return rows(result, row -> actualMapper.apply(UserProjection.fromJsonNode(row)));")
            && sync_delegate.contains("return mapProjectionRow(JsonSupport.firstDataRow(result), mapper);"),
        "expected generated Java projection APIs to return typed projection rows or mapped values:\n{sync_delegate}"
    );
    assert!(
        projection.contains("public final class UserProjection implements WireSerializable")
            && projection.contains("public boolean hasId()")
            && projection.contains("public Integer id()")
            && projection.contains("public boolean hasName()")
            && projection.contains("public String name()")
            && projection.contains("return this.row.deepCopy();"),
        "expected generated Java projection class to expose typed getters plus presence checks:\n{projection}"
    );
}

#[test]
fn test_generated_java_hstore_uses_runtime_type_that_preserves_null_values() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore]
}

generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

model User {
  id   Int     @id @default(autoincrement())
  meta Hstore?
}
"#,
    );

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let user_model = generated_java_file(&java_files, "model/User.java");
    let json_support = generated_java_file(&java_files, "internal/JsonSupport.java");

    // With the `hstore` extension declared the model field uses the generated
    // `Hstore` wrapper class (which itself wraps `JsonSupport.Hstore` to
    // preserve null-aware key/value semantics on the wire).
    assert!(user_model.contains("Hstore meta"));
    assert!(user_model.contains("import com.acme.db.extensions.hstore.types.Hstore;"));
    assert!(json_support
        .contains("public static final class Hstore extends LinkedHashMap<String, String>"));
    assert!(json_support.contains("public static Hstore asHstore(JsonNode node)"));
}

/// A schema whose model pulls in several enums, composite types and relations
/// at once — the imports that used to be collected in a `HashSet`.
const MULTI_IMPORT_SCHEMA: &str = r#"
enum Status { Active Inactive }
enum Role { Admin Member }
enum Tier { Free Pro }

type Address {
  street String
  city   String
}

type Contact {
  email String
  phone String
}

model User {
  id      Int      @id @default(autoincrement())
  status  Status
  role    Role
  tier    Tier
  address Address
  contact Contact
  posts   Post[]
  notes   Note[]
}

model Post {
  id       Int  @id @default(autoincrement())
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}

model Note {
  id       Int  @id @default(autoincrement())
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}
"#;

#[test]
fn test_generated_imports_are_stable_across_runs() {
    let ir = validate(MULTI_IMPORT_SCHEMA);

    for _ in 0..8 {
        let rust = generate_all_models(&ir, false).expect("generate_all_models should succeed");
        assert_eq!(
            rust.get("User"),
            generate_all_models(&ir, false)
                .expect("generate_all_models should succeed")
                .get("User"),
            "Rust model output must not vary between runs"
        );

        let python = generate_all_python_models(&ir, false, 3)
            .expect("generate_all_python_models should succeed");
        assert_eq!(
            generated_python_file(&python, "user.py"),
            generated_python_file(
                &generate_all_python_models(&ir, false, 3)
                    .expect("generate_all_python_models should succeed"),
                "user.py"
            ),
            "Python model output must not vary between runs"
        );

        let (js, dts) = generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
        let (js_again, dts_again) =
            generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
        assert_eq!(js, js_again, "JS model output must not vary between runs");
        assert_eq!(dts, dts_again, "d.ts output must not vary between runs");
    }
}

#[test]
fn test_generated_imports_are_sorted() {
    let ir = validate(MULTI_IMPORT_SCHEMA);

    let rust = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let rust_user = rust.get("User").expect("User model missing");
    assert!(
        rust_user.contains(
            "use super::enums::Role;\nuse super::enums::Status;\nuse super::enums::Tier;"
        ),
        "Rust enum imports should be sorted:\n{rust_user}"
    );
    assert!(
        rust_user.contains("use super::types::Address;\nuse super::types::Contact;"),
        "Rust composite type imports should be sorted:\n{rust_user}"
    );
    assert!(
        rust_user.contains("use super::Note;\nuse super::Post;"),
        "Rust relation imports should be sorted:\n{rust_user}"
    );

    let python = generate_all_python_models(&ir, false, 3)
        .expect("generate_all_python_models should succeed");
    let python_user = generated_python_file(&python, "user.py");
    assert!(
        python_user.contains("from ..enums.enums import Role, Status, Tier"),
        "Python enum imports should be sorted:\n{python_user}"
    );
    assert!(
        python_user.contains("from ..types.types import Address, Contact"),
        "Python composite type imports should be sorted:\n{python_user}"
    );

    let (_, dts) = generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_user = generated_named_file(&dts, "user.d.ts");
    assert!(
        js_user.contains("import type { Role, Status, Tier } from '../enums.js';"),
        "TypeScript enum imports should be sorted:\n{js_user}"
    );
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

#[test]
fn test_js_client_calls_the_array_extension_coercer() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext]
}

model Doc {
  id   Int      @id @default(autoincrement())
  tags Citext[]
}
"#,
    );

    let (models, _) = generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let code = generated_named_file(&models, "doc.js");

    assert!(
        code.contains(
            "coerced = ((value) => Array.isArray(value) ? value.map(item => Citext.from(item)) : value)(value);"
        ),
        "the array coercer must be applied to the value, not assigned as a function:\n{code}"
    );
    assert!(
        !code.contains("coerced = (value) =>"),
        "assigning the coercer itself leaves a function on the model, which JSON.stringify \
         silently drops:\n{code}"
    );
}

#[test]
fn test_generated_clients_write_null_into_nullable_extension_columns() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext]
}

model Doc {
  id      Int     @id @default(autoincrement())
  altSlug Citext?
}
"#,
    );

    // Both the write path and the filter path need the guard, so pin each one
    // inside its own function: asserting on the whole file would let either
    // regress while the other kept the assertion green.
    let (js_models, _) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js = generated_named_file(&js_models, "doc.js");
    for function in [
        "function _serializeScalarInput",
        "function _serializeFilterInput",
    ] {
        let body = section_until(js, function, "\n}");
        assert!(
            body.contains("if (value == null || !serializer) return _toWireValue(value);"),
            "a null must bypass the extension coercer in {function}, which only knows how to \
             build a value:\n{body}"
        );
    }

    let py_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let py = generated_python_file(&py_models, "doc.py");
    for function in ["def _serialize_scalar_input", "def _serialize_filter_input"] {
        let body = section_until(py, function, "\n\ndef ");
        assert!(
            body.contains("if value is None or serializer is None:"),
            "the Python client must bypass the extension coercer for None in {function}:\n{body}"
        );
    }
}

#[test]
fn test_rust_citext_wrapper_tags_values_with_their_type() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext]
}

model Doc {
  id   Int    @id @default(autoincrement())
  slug Citext
}
"#,
    );

    let files = generate_rust_extension_files(&ExtensionRegistry::from_schema(&ir))
        .expect("generate_rust_extension_files should succeed");
    let citext = generated_named_file(&files, "extensions/citext/types.rs");

    assert!(
        citext.contains("nautilus_core::Value::Extension { value: value.into_inner(), type_name: \"citext\".to_string() }"),
        "a citext must carry its type name so the dialect can emit `$1::citext`; without the \
         cast PostgreSQL compares a citext column case sensitively:\n{citext}"
    );
}

/// An include node has the shape of a read's arguments and must get the same
/// preparation, against the model it loads rather than the one it hangs off.
///
/// A serializer that walks into the node's `where` instead rebuilds the values
/// inside it, which turns a `Date` into `{}` and leaves `equals` untranslated —
/// both silent, because neither reaches the engine as an error.
#[test]
fn test_js_and_python_prepare_include_nodes_against_the_included_model() {
    let ir = validate(
        r#"
model Author {
  id    Int    @id @default(autoincrement())
  posts Post[]
}

model Post {
  id       Int      @id @default(autoincrement())
  authorId Int      @map("author_id")
  author   Author   @relation(fields: [authorId], references: [id])
}
"#,
    );

    let (js_models, _) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let (_, author_js) = js_models
        .iter()
        .find(|(name, _)| name == "author.js")
        .expect("author runtime missing");
    let (_, post_js) = js_models
        .iter()
        .find(|(name, _)| name == "post.js")
        .expect("post runtime missing");

    assert!(
        post_js.contains("_processWhereFilters(spec.where, _PostFieldToDb)"),
        "an include node's where must go through the included model's own filter preparation:\n{post_js}"
    );
    assert!(
        post_js.contains(
            "node['orderBy']  = Array.isArray(spec.orderBy) ? spec.orderBy : [spec.orderBy];"
        ),
        "an include node's orderBy must reach the engine as a list:\n{post_js}"
    );
    assert!(
        author_js.contains("result[field] = _serializePostIncludeArgs_for_posts(spec);"),
        "each relation must be prepared by the model it loads:\n{author_js}"
    );

    let python = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, author_py) = python
        .iter()
        .find(|(name, _)| name == "author.py")
        .expect("author model missing");
    let (_, post_py) = python
        .iter()
        .find(|(name, _)| name == "post.py")
        .expect("post model missing");

    assert!(
        post_py.contains("_process_where_filters(value, _Post_py_to_db)"),
        "an include node's where must go through the included model's own filter preparation:\n{post_py}"
    );
    assert!(
        post_py.contains(r#"node["orderBy"] = [{fk: fv} for fk, fv in value.items()]"#),
        "an include node's order_by must reach the engine as a list:\n{post_py}"
    );
    assert!(
        author_py.contains("from .post import _serialize_post_include_args"),
        "each relation must be prepared by the model it loads:\n{author_py}"
    );
}
