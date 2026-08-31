mod common;

use common::parse_schema as parse;
use nautilus_schema::ir::{
    ManyToManyJoinIr, PrimaryKeyIr, RelationIr, ResolvedFieldType, ScalarType, SchemaIr,
};
use nautilus_schema::{validate_schema, SchemaError};

const POST_AND_TAG: &str = r#"
model Post {
  id    Int    @id @default(autoincrement())
  title String @unique
  tags  Tag[]
}

model Tag {
  id    Int    @id @default(autoincrement())
  label String @unique
  posts Post[]
}
"#;

fn ir_of(source: &str) -> SchemaIr {
    validate_schema(parse(source).unwrap()).unwrap()
}

fn validation_error(source: &str) -> String {
    match validate_schema(parse(source).unwrap()).unwrap_err() {
        SchemaError::Validation(message, _) => message,
        other => panic!("Expected a validation error, got {other:?}"),
    }
}

fn relation<'a>(ir: &'a SchemaIr, model: &str, field: &str) -> &'a RelationIr {
    match &ir
        .get_model(model)
        .unwrap_or_else(|| panic!("model '{model}'"))
        .find_field(field)
        .unwrap_or_else(|| panic!("field '{model}.{field}'"))
        .field_type
    {
        ResolvedFieldType::Relation(relation) => relation,
        other => panic!("'{model}.{field}' is not a relation: {other:?}"),
    }
}

fn join<'a>(ir: &'a SchemaIr, model: &str, field: &str) -> &'a ManyToManyJoinIr {
    relation(ir, model, field)
        .join
        .as_ref()
        .unwrap_or_else(|| panic!("'{model}.{field}' has no join table"))
}

#[test]
fn an_array_on_both_sides_synthesises_a_join_table() {
    let ir = ir_of(POST_AND_TAG);

    let table = ir.get_model("_PostToTag").expect("the join table");
    assert!(table.is_join_table);
    assert_eq!(table.db_name, "_PostToTag");
    assert_eq!(
        table.primary_key,
        PrimaryKeyIr::Composite(vec!["A".to_string(), "B".to_string()])
    );

    let column_names: Vec<&str> = table
        .scalar_fields()
        .map(|field| field.db_name.as_str())
        .collect();
    assert_eq!(column_names, vec!["A", "B"]);
}

#[test]
fn the_join_columns_take_the_type_of_the_keys_they_point_at() {
    let ir = ir_of(
        r#"
model Post {
  id    String @id
  tags  Tag[]
}

model Tag {
  id    Int    @id @default(autoincrement())
  posts Post[]
}
"#,
    );

    let table = ir.get_model("_PostToTag").unwrap();
    assert_eq!(
        table.find_field("A").unwrap().field_type,
        ResolvedFieldType::Scalar(ScalarType::String)
    );
    assert_eq!(
        table.find_field("B").unwrap().field_type,
        ResolvedFieldType::Scalar(ScalarType::Int)
    );
}

#[test]
fn each_side_learns_which_join_column_is_its_own() {
    let ir = ir_of(POST_AND_TAG);

    let from_post = join(&ir, "Post", "tags");
    assert_eq!(from_post.table, "_PostToTag");
    assert_eq!(from_post.self_column, "A");
    assert_eq!(from_post.target_column, "B");
    assert_eq!(from_post.self_reference, "id");

    let from_tag = join(&ir, "Tag", "posts");
    assert_eq!(from_tag.table, "_PostToTag");
    assert_eq!(from_tag.self_column, "B");
    assert_eq!(from_tag.target_column, "A");
}

#[test]
fn the_join_table_carries_a_cascading_foreign_key_to_each_side() {
    let ir = ir_of(POST_AND_TAG);
    let table = ir.get_model("_PostToTag").unwrap();

    let targets: Vec<(&str, Vec<String>, Vec<String>)> = table
        .relation_fields()
        .map(|field| match &field.field_type {
            ResolvedFieldType::Relation(relation) => {
                assert_eq!(
                    relation.on_delete,
                    Some(nautilus_schema::ast::ReferentialAction::Cascade)
                );
                (
                    relation.target_model.as_str(),
                    relation.fields.clone(),
                    relation.references.clone(),
                )
            }
            other => panic!("expected a relation, got {other:?}"),
        })
        .collect();

    assert_eq!(
        targets,
        vec![
            ("Post", vec!["A".to_string()], vec!["id".to_string()]),
            ("Tag", vec!["B".to_string()], vec!["id".to_string()]),
        ]
    );
}

#[test]
fn a_named_relation_names_the_join_table_after_itself() {
    let ir = ir_of(
        r#"
model Post {
  id   Int   @id
  tags Tag[] @relation(name: "Labelling")
}

model Tag {
  id    Int    @id
  posts Post[] @relation(name: "Labelling")
}
"#,
    );

    assert!(ir.get_model("_Labelling").unwrap().is_join_table);
    assert!(ir.get_model("_PostToTag").is_none());
    assert_eq!(join(&ir, "Post", "tags").table, "_Labelling");
}

#[test]
fn a_model_can_hold_a_many_to_many_with_itself() {
    let ir = ir_of(
        r#"
model Post {
  id       Int    @id
  related  Post[] @relation(name: "RelatedPost")
  relating Post[] @relation(name: "RelatedPost")
}
"#,
    );

    let table = ir.get_model("_RelatedPost").expect("the join table");
    assert!(table.is_join_table);

    // The field that sorts first is the `A` side, so the two directions stay
    // apart however the schema happens to be read back.
    assert_eq!(join(&ir, "Post", "related").self_column, "A");
    assert_eq!(join(&ir, "Post", "relating").self_column, "B");
}

#[test]
fn a_one_to_many_is_left_alone() {
    let ir = ir_of(
        r#"
model Author {
  id    Int    @id
  posts Post[]
}

model Post {
  id       Int    @id
  authorId Int
  author   Author @relation(fields: [authorId], references: [id])
}
"#,
    );

    assert!(relation(&ir, "Author", "posts").join.is_none());
    assert_eq!(ir.models.len(), 2);
}

#[test]
fn generation_prunes_the_join_table_but_keeps_the_relation() {
    let ir = ir_of(POST_AND_TAG);
    let pruned = ir.without_join_tables();

    assert!(pruned.get_model("_PostToTag").is_none());
    assert_eq!(pruned.models.len(), 2);
    assert!(pruned
        .get_model("Post")
        .unwrap()
        .find_field("tags")
        .is_some());
}

#[test]
fn a_composite_key_cannot_be_reached_through_a_join_table() {
    let message = validation_error(
        r#"
model Post {
  left  Int
  right Int
  tags  Tag[]
  @@id([left, right])
}

model Tag {
  id    Int    @id
  posts Post[]
}
"#,
    );

    assert!(
        message.contains("single-field primary key on 'Post'"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_model_occupying_the_join_table_name_is_reported() {
    let message = validation_error(
        r#"
model Post {
  id   Int   @id
  tags Tag[]
}

model Tag {
  id    Int    @id
  posts Post[]
}

model _PostToTag {
  id Int @id
}
"#,
    );

    assert!(
        message.contains("join table called '_PostToTag'"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_many_to_many_with_a_view_is_refused() {
    let message = validation_error(
        r#"
model Post {
  id   Int   @id
  tags Tag[]
}

view Tag {
  id    Int    @id
  posts Post[]
}
"#,
    );

    assert!(
        message.contains("view 'Tag'"),
        "unexpected message: {message}"
    );
}

#[test]
fn two_unnamed_self_relations_pair_with_each_other() {
    let ir = ir_of(
        r#"
model Post {
  id      Int    @id
  mirror  Post[]
  related Post[]
}
"#,
    );

    assert!(ir.get_model("_PostToPost").unwrap().is_join_table);
    assert_eq!(join(&ir, "Post", "mirror").self_column, "A");
    assert_eq!(join(&ir, "Post", "related").self_column, "B");
}

#[test]
fn more_than_two_candidate_ends_have_to_be_named() {
    let message = validation_error(
        r#"
model Post {
  id      Int    @id
  mirror  Post[]
  related Post[]
  cited   Post[]
}
"#,
    );

    assert!(
        message.contains("could close a many-to-many with more than one field"),
        "unexpected message: {message}"
    );
}
