mod common;

use common::parse_schema as parse;
use nautilus_schema::{format_schema, validate_schema, SchemaError};

const DATASOURCE: &str = r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
  schemas  = ["public", "analytics"]
}
"#;

fn validation_error(source: &str) -> String {
    let ast = parse(source).unwrap();
    match validate_schema(ast).unwrap_err() {
        SchemaError::Validation(message, _) => message,
        other => panic!("Expected a validation error, got {other:?}"),
    }
}

#[test]
fn schema_attribute_lands_on_the_model_ir() {
    let source = format!(
        r#"{DATASOURCE}
model User {{
  id Int @id
  @@schema("public")
}}

model Event {{
  id Int @id
  @@map("events")
  @@schema("analytics")
}}
"#
    );

    let ir = validate_schema(parse(&source).unwrap()).unwrap();
    assert_eq!(
        ir.datasource.as_ref().unwrap().schemas,
        vec!["public".to_string(), "analytics".to_string()]
    );
    assert_eq!(
        ir.get_model("User").unwrap().schema.as_deref(),
        Some("public")
    );
    let event = ir.get_model("Event").unwrap();
    assert_eq!(event.db_name, "events");
    assert_eq!(event.schema.as_deref(), Some("analytics"));
}

#[test]
fn two_models_may_share_a_table_name_across_schemas() {
    let source = format!(
        r#"{DATASOURCE}
model PublicEvent {{
  id Int @id
  @@map("events")
  @@schema("public")
}}

model AnalyticsEvent {{
  id Int @id
  @@map("events")
  @@schema("analytics")
}}
"#
    );

    let ir = validate_schema(parse(&source).unwrap()).unwrap();
    assert_eq!(ir.get_model("PublicEvent").unwrap().db_name, "events");
    assert_eq!(ir.get_model("AnalyticsEvent").unwrap().db_name, "events");
}

#[test]
fn a_view_carries_its_schema_too() {
    let source = format!(
        r#"{DATASOURCE}
view ActiveUser {{
  id Int @id
  @@map("active_users")
  @@schema("analytics")
}}
"#
    );

    let ir = validate_schema(parse(&source).unwrap()).unwrap();
    let view = ir.get_model("ActiveUser").unwrap();
    assert!(view.is_view);
    assert_eq!(view.schema.as_deref(), Some("analytics"));
}

#[test]
fn schema_attribute_without_a_declared_list_is_rejected() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

model User {
  id Int @id
  @@schema("analytics")
}
"#;
    assert!(validation_error(source).contains("no 'schemas' list"));
}

#[test]
fn an_undeclared_schema_is_rejected() {
    let source = format!(
        r#"{DATASOURCE}
model User {{
  id Int @id
  @@schema("reporting")
}}
"#
    );
    let message = validation_error(&source);
    assert!(message.contains("'reporting'"), "{message}");
    assert!(message.contains("public, analytics"), "{message}");
}

#[test]
fn every_model_must_name_its_schema_in_multi_schema_mode() {
    let source = format!(
        r#"{DATASOURCE}
model User {{
  id Int @id
}}
"#
    );
    let message = validation_error(&source);
    assert!(message.contains("must declare @@schema"), "{message}");
}

#[test]
fn schemas_is_postgres_only() {
    let source = r#"
datasource db {
  provider = "mysql"
  url      = env("DATABASE_URL")
  schemas  = ["public"]
}

model User {
  id Int @id
}
"#;
    assert!(validation_error(source).contains("only supported for the 'postgresql' provider"));
}

#[test]
fn an_empty_schemas_list_is_rejected() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
  schemas  = []
}

model User {
  id Int @id
}
"#;
    assert!(validation_error(source).contains("at least one schema"));
}

#[test]
fn a_duplicate_schema_is_rejected() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
  schemas  = ["public", "public"]
}

model User {
  id Int @id
  @@schema("public")
}
"#;
    assert!(validation_error(source).contains("Duplicate schema 'public'"));
}

#[test]
fn the_formatter_round_trips_the_attribute() {
    let source = format!(
        r#"{DATASOURCE}
model User {{
  id Int @id
  @@schema("public")
}}
"#
    );
    let formatted = format_schema(&parse(&source).unwrap(), &source);
    assert!(formatted.contains("@@schema(\"public\")"), "{formatted}");
    assert!(
        formatted.contains("schemas  = [\"public\", \"analytics\"]"),
        "{formatted}"
    );
}

#[test]
fn a_join_table_inherits_the_schema_of_the_first_side() {
    let source = format!(
        r#"{DATASOURCE}
model Post {{
  id   Int   @id
  tags Tag[]
  @@schema("analytics")
}}

model Tag {{
  id    Int    @id
  posts Post[]
  @@schema("analytics")
}}
"#
    );

    let ir = validate_schema(parse(&source).unwrap()).unwrap();
    let join = ir.get_model("_PostToTag").expect("synthesised join table");
    assert!(join.is_join_table);
    assert_eq!(join.schema.as_deref(), Some("analytics"));
}
