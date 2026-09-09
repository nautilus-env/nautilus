use super::support::{generated_java_file, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::java::generate_java_client;

const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");
const NESTED_WRITE_SCHEMA: &str = include_str!("../fixtures/schemas/nested_writes.nautilus");

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
fn test_rust_create_input_carries_nested_writes_for_both_relation_sides() {
    let ir = validate(NESTED_WRITE_SCHEMA);
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
    let ir = validate(NESTED_WRITE_SCHEMA);
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
    let ir = validate(concat!(
        r#"
generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
}
"#,
        include_str!("../fixtures/schemas/nested_writes.nautilus")
    ));
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
