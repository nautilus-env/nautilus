//! Committed snapshot baselines for one canonical schema, across all four
//! generated clients.
//!
//! The broad baselines in `snapshot_tests.rs` are local-only on purpose, so
//! this file is the workspace's only guard against accidental changes to the
//! generated public API. It covers a single small schema — three models, an
//! enum, a composite type and a one-to-many relation — so the baselines stay
//! small enough to review in a diff.
//!
//! Baselines live in `tests/canonical_snapshots/` (committed, unlike
//! `tests/snapshots/`). Refresh them with
//! `INSTA_UPDATE=always cargo test -p nautilus-orm-codegen --test canonical_snapshot_tests`
//! and review the diff: any change here is a change to what users compile
//! against.

use nautilus_codegen::{
    enum_gen::generate_all_enums,
    generator::generate_all_models,
    java::generate_java_client,
    js::{generate_all_js_models, generate_js_client},
    python::{
        generate_all_python_models, generate_python_client, generate_python_composite_types,
        generate_python_enums,
    },
};
use nautilus_schema::{ir::SchemaIr, validate_schema_source};

const CANONICAL_SCHEMA: &str = r#"
enum Role {
  ADMIN
  MEMBER
}

type Address {
  street String
  city   String
  zip    String?
}

model User {
  id        Int       @id @default(autoincrement())
  email     String    @unique
  name      String?
  role      Role      @default(MEMBER)
  address   Address?
  createdAt DateTime  @default(now())
  posts     Post[]

  @@map("users")
}

model Post {
  id        Int      @id @default(autoincrement())
  title     String
  body      String?
  published Boolean  @default(false)
  views     Int      @default(0)
  authorId  Int      @map("author_id")
  author    User     @relation(fields: [authorId], references: [id])
  tags      Tag[]

  @@index([authorId])
  @@map("posts")
}

model Tag {
  id     Int    @id @default(autoincrement())
  label  String @unique
  postId Int    @map("post_id")
  post   Post   @relation(fields: [postId], references: [id])
}
"#;

/// The Java backend refuses to generate without a `generator` block, since it
/// needs the package and Maven coordinates. It is kept out of
/// [`CANONICAL_SCHEMA`] so the other three backends snapshot the plain schema.
const JAVA_GENERATOR_BLOCK: &str = r#"
generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}
"#;

const SCHEMA_PATH: &str = "schema.nautilus";

fn canonical_ir() -> SchemaIr {
    ir_of(CANONICAL_SCHEMA)
}

fn canonical_java_ir() -> SchemaIr {
    ir_of(&format!("{JAVA_GENERATOR_BLOCK}{CANONICAL_SCHEMA}"))
}

fn ir_of(source: &str) -> SchemaIr {
    validate_schema_source(source)
        .expect("the canonical schema must stay valid")
        .ir
}

macro_rules! assert_canonical_snapshot {
    ($name:expr, $value:expr $(,)?) => {{
        insta::with_settings!({ snapshot_path => "canonical_snapshots" }, {
            insta::assert_snapshot!($name, $value);
        });
    }};
}

fn named_file<'a>(files: &'a [(String, String)], name: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path == name)
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated file '{name}'"))
}

fn java_file<'a>(files: &'a [(String, String)], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path.ends_with(suffix))
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated Java file ending with '{suffix}'"))
}

#[test]
fn canonical_rust_client() {
    let ir = canonical_ir();
    let models = generate_all_models(&ir, true).expect("Rust model generation should succeed");

    assert_canonical_snapshot!(
        "rust_user",
        models.get("User").expect("User model should be generated")
    );
    assert_canonical_snapshot!(
        "rust_post",
        models.get("Post").expect("Post model should be generated")
    );
    assert_canonical_snapshot!(
        "rust_enums",
        generate_all_enums(&ir.enums).expect("Rust enum generation should succeed")
    );
}

#[test]
fn canonical_python_client() {
    let ir = canonical_ir();
    let models =
        generate_all_python_models(&ir, true, 0).expect("Python model generation should succeed");

    assert_canonical_snapshot!("python_user", named_file(&models, "user.py"));
    assert_canonical_snapshot!("python_post", named_file(&models, "post.py"));
    assert_canonical_snapshot!(
        "python_enums",
        generate_python_enums(&ir.enums).expect("Python enum generation should succeed")
    );
    assert_canonical_snapshot!(
        "python_types",
        generate_python_composite_types(&ir.composite_types)
            .expect("Python composite type generation should succeed")
            .expect("the canonical schema declares a composite type")
    );
    assert_canonical_snapshot!(
        "python_client",
        generate_python_client(&ir.models, SCHEMA_PATH, true)
            .expect("Python client generation should succeed")
    );
}

#[test]
fn canonical_js_client() {
    let ir = canonical_ir();
    let (models, declarations) =
        generate_all_js_models(&ir).expect("JS model generation should succeed");

    assert_canonical_snapshot!("js_user", named_file(&models, "user.js"));
    assert_canonical_snapshot!("js_user_dts", named_file(&declarations, "user.d.ts"));
    assert_canonical_snapshot!("js_post_dts", named_file(&declarations, "post.d.ts"));

    let (client, client_dts) =
        generate_js_client(&ir.models, SCHEMA_PATH).expect("JS client generation should succeed");
    assert_canonical_snapshot!("js_client", client);
    assert_canonical_snapshot!("js_client_dts", client_dts);
}

#[test]
fn canonical_java_client() {
    let ir = canonical_java_ir();
    let files = generate_java_client(&ir, SCHEMA_PATH, false)
        .expect("Java client generation should succeed");

    assert_canonical_snapshot!("java_user_model", java_file(&files, "model/User.java"));
    assert_canonical_snapshot!(
        "java_user_delegate",
        java_file(&files, "client/UserDelegate.java")
    );
    assert_canonical_snapshot!("java_user_dsl", java_file(&files, "dsl/UserDsl.java"));
    assert_canonical_snapshot!("java_client", java_file(&files, "client/Nautilus.java"));
}
