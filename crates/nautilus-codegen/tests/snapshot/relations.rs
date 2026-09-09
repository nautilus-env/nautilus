use super::support::{assert_codegen_snapshot, python_runtime_codec, section_until, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

const BLOG_RELATIONS_SCHEMA: &str = include_str!("../fixtures/schemas/blog_relations.nautilus");

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
    assert_codegen_snapshot!("rust_user_with_posts_relation", user_code);
    assert_codegen_snapshot!("rust_post_with_author_relation", post_code);
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
        user_code.contains("crate::runtime::EngineOnly::Include"),
        "expected the fallback path to name include among the engine-only features:\n{user_code}"
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

    let post_include_args = section_until(post_py, "def _serialize_post_include_args", "\n\ndef ");
    assert!(
        post_include_args.contains("_process_where_filters,")
            && post_include_args.contains("_Post_py_to_db,"),
        "an include node's where must go through the included model's own filter preparation:\n{post_include_args}"
    );
    assert!(
        python_runtime_codec()
            .contains(r#"node["orderBy"] = [{fk: fv} for fk, fv in value.items()]"#),
        "an include node's order_by must reach the engine as a list"
    );
    assert!(
        author_py.contains("from .post import _serialize_post_include_args"),
        "each relation must be prepared by the model it loads:\n{author_py}"
    );
}
