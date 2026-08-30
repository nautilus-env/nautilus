mod common;

use common::{call_rpc_json, call_rpc_response, sqlite_state};
use nautilus_protocol::{
    PROTOCOL_VERSION, QUERY_CREATE, QUERY_FIND_MANY, QUERY_UPDATE, TRANSACTION_ROLLBACK,
    TRANSACTION_START,
};
use serde_json::{json, Value};

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id      Int      @id @default(autoincrement())
  email   String   @unique
  name    String
  profile Profile?
  posts   Post[]
}

model Profile {
  id     Int    @id @default(autoincrement())
  bio    String
  userId Int    @unique @map("user_id")
  user   User   @relation(fields: [userId], references: [id])
}

model Post {
  id         Int       @id @default(autoincrement())
  title      String    @unique
  published  Boolean   @default(false)
  authorId   Int?      @map("author_id")
  author     User?     @relation(fields: [authorId], references: [id])
  categoryId Int?      @map("category_id")
  category   Category? @relation(fields: [categoryId], references: [id])
}

model Category {
  id    Int    @id @default(autoincrement())
  name  String @unique
  posts Post[]
}
"#
}

async fn rows(state: &nautilus_engine::EngineState, model: &str, filter: Value) -> Vec<Value> {
    let args = if filter.as_object().is_some_and(|obj| obj.is_empty()) {
        json!({})
    } else {
        json!({ "where": filter })
    };
    let result = call_rpc_json(
        state,
        QUERY_FIND_MANY,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": model, "args": args }),
    )
    .await;
    result["data"].as_array().cloned().unwrap_or_default()
}

async fn column(
    state: &nautilus_engine::EngineState,
    model: &str,
    filter: Value,
    column: &str,
) -> Value {
    let rows = rows(state, model, filter).await;
    assert_eq!(rows.len(), 1, "expected exactly one {model} row");
    rows[0][format!("{model}__{column}")].clone()
}

/// SQLite hands booleans back as 0/1, so the scenario compares truthiness
/// rather than the JSON shape the backend happened to choose.
fn truthy(value: &Value) -> bool {
    value.as_bool().unwrap_or_else(|| value.as_i64() == Some(1))
}

async fn titles(state: &nautilus_engine::EngineState, filter: Value) -> Vec<String> {
    let mut found: Vec<String> = rows(state, "Post", filter)
        .await
        .into_iter()
        .map(|row| row["Post__title"].as_str().unwrap_or_default().to_string())
        .collect();
    found.sort();
    found
}

async fn create_ada(state: &nautilus_engine::EngineState) -> i64 {
    let result = call_rpc_json(
        state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": {
                "email": "ada@example.com",
                "name": "Ada",
                "profile": { "create": { "bio": "counts things" } },
                "posts": { "create": [{ "title": "on engines" }, { "title": "on looms" }] },
            },
        }),
    )
    .await;
    result["data"][0]["User__id"]
        .as_i64()
        .expect("create should return the parent key")
}

#[tokio::test]
async fn create_writes_children_and_links_them_to_the_new_row() {
    let (state, _dir) = sqlite_state("nested-create", schema_source()).await;

    let ada = create_ada(&state).await;

    assert_eq!(
        column(
            &state,
            "Profile",
            json!({ "bio": "counts things" }),
            "user_id"
        )
        .await,
        json!(ada)
    );
    assert_eq!(
        titles(&state, json!({ "author_id": ada })).await,
        vec!["on engines", "on looms"]
    );
    assert!(
        !truthy(&column(&state, "Post", json!({ "title": "on looms" }), "published").await),
        "a child still gets the column defaults of its own model"
    );
}

#[tokio::test]
async fn connect_resolves_the_foreign_key_of_the_side_that_holds_it() {
    let (state, _dir) = sqlite_state("nested-connect", schema_source()).await;

    let ada = create_ada(&state).await;
    call_rpc_json(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": {
                "title": "on difference",
                "author": { "connect": { "email": "ada@example.com" } },
            },
        }),
    )
    .await;

    assert_eq!(
        column(
            &state,
            "Post",
            json!({ "title": "on difference" }),
            "author_id"
        )
        .await,
        json!(ada)
    );
}

#[tokio::test]
async fn connect_or_create_creates_only_when_nothing_matches() {
    let (state, _dir) = sqlite_state("nested-connect-or-create", schema_source()).await;

    for title in ["on notes", "on tables"] {
        call_rpc_json(
            &state,
            QUERY_CREATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Post",
                "data": {
                    "title": title,
                    "category": {
                        "connectOrCreate": {
                            "where": { "name": "mathematics" },
                            "create": { "name": "mathematics" },
                        }
                    },
                },
            }),
        )
        .await;
    }

    assert_eq!(rows(&state, "Category", json!({})).await.len(), 1);
    assert_eq!(
        column(
            &state,
            "Post",
            json!({ "title": "on notes" }),
            "category_id"
        )
        .await,
        column(
            &state,
            "Post",
            json!({ "title": "on tables" }),
            "category_id"
        )
        .await
    );
}

#[tokio::test]
async fn update_writes_columns_and_relations_in_one_call() {
    let (state, _dir) = sqlite_state("nested-update", schema_source()).await;

    let ada = create_ada(&state).await;
    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": { "email": "ada@example.com" },
            "data": {
                "name": "Ada L.",
                "posts": { "create": { "title": "on notation" } },
            },
        }),
    )
    .await;

    assert_eq!(
        column(
            &state,
            "User",
            json!({ "email": "ada@example.com" }),
            "name"
        )
        .await,
        json!("Ada L.")
    );
    assert_eq!(
        titles(&state, json!({ "author_id": ada })).await,
        vec!["on engines", "on looms", "on notation"]
    );
}

#[tokio::test]
async fn update_many_and_delete_many_cannot_reach_another_parents_rows() {
    let (state, _dir) = sqlite_state("nested-scoping", schema_source()).await;

    let ada = create_ada(&state).await;
    call_rpc_json(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": { "title": "on nobody" },
        }),
    )
    .await;

    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": { "email": "ada@example.com" },
            "data": { "posts": { "updateMany": { "where": {}, "data": { "published": true } } } },
        }),
    )
    .await;

    assert!(truthy(
        &column(
            &state,
            "Post",
            json!({ "title": "on engines" }),
            "published"
        )
        .await
    ));
    assert!(
        !truthy(&column(&state, "Post", json!({ "title": "on nobody" }), "published").await),
        "a row outside the relation must not be touched"
    );

    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": { "email": "ada@example.com" },
            "data": { "posts": { "deleteMany": true } },
        }),
    )
    .await;

    assert!(titles(&state, json!({ "author_id": ada })).await.is_empty());
    assert_eq!(
        rows(&state, "Post", json!({ "title": "on nobody" }))
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn set_replaces_the_members_of_a_relation() {
    let (state, _dir) = sqlite_state("nested-set", schema_source()).await;

    let ada = create_ada(&state).await;
    call_rpc_json(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": { "title": "on notes" },
        }),
    )
    .await;

    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": { "email": "ada@example.com" },
            "data": { "posts": { "set": [{ "title": "on notes" }] } },
        }),
    )
    .await;

    assert_eq!(
        titles(&state, json!({ "author_id": ada })).await,
        vec!["on notes"]
    );
    assert_eq!(
        rows(&state, "Post", json!({})).await.len(),
        3,
        "set only detaches the rows it drops"
    );
}

#[tokio::test]
async fn a_failing_child_rolls_the_whole_write_back() {
    let (state, _dir) = sqlite_state("nested-atomic", schema_source()).await;

    create_ada(&state).await;

    let response = call_rpc_response(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": {
                "email": "hedy@example.com",
                "name": "Hedy",
                "posts": { "create": [{ "title": "on frequencies" }, { "title": "on engines" }] },
            },
        }),
    )
    .await;

    assert!(
        response.error.is_some(),
        "a duplicate child must fail the write"
    );
    assert!(rows(&state, "User", json!({ "email": "hedy@example.com" }))
        .await
        .is_empty());
    assert!(rows(&state, "Post", json!({ "title": "on frequencies" }))
        .await
        .is_empty());
}

#[tokio::test]
async fn a_callers_transaction_still_owns_the_commit() {
    let (state, _dir) = sqlite_state("nested-transaction", schema_source()).await;

    let started = call_rpc_json(
        &state,
        TRANSACTION_START,
        json!({ "protocolVersion": PROTOCOL_VERSION, "timeoutMs": 20000 }),
    )
    .await;
    let tx = started["id"].as_str().expect("transaction id").to_string();

    call_rpc_json(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": {
                "email": "rolled@example.com",
                "name": "Rolled",
                "posts": { "create": { "title": "on rollbacks" } },
            },
            "transactionId": tx,
        }),
    )
    .await;

    call_rpc_json(
        &state,
        TRANSACTION_ROLLBACK,
        json!({ "protocolVersion": PROTOCOL_VERSION, "id": tx }),
    )
    .await;

    assert!(
        rows(&state, "User", json!({ "email": "rolled@example.com" }))
            .await
            .is_empty()
    );
    assert!(rows(&state, "Post", json!({ "title": "on rollbacks" }))
        .await
        .is_empty());
}

#[tokio::test]
async fn a_nested_write_needs_a_filter_that_names_one_row() {
    let (state, _dir) = sqlite_state("nested-ambiguous", schema_source()).await;

    create_ada(&state).await;
    call_rpc_json(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": { "email": "grace@example.com", "name": "Grace" },
        }),
    )
    .await;

    let response = call_rpc_response(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": { "id": { "gt": 0 } },
            "data": { "posts": { "create": { "title": "on ambiguity" } } },
        }),
    )
    .await;

    let error = response.error.expect("an ambiguous parent must be refused");
    assert!(
        error.message.contains("exactly one row"),
        "unexpected error: {}",
        error.message
    );
    assert!(rows(&state, "Post", json!({ "title": "on ambiguity" }))
        .await
        .is_empty());
}

#[tokio::test]
async fn unknown_and_update_only_operations_are_refused() {
    let (state, _dir) = sqlite_state("nested-operations", schema_source()).await;

    let unknown = call_rpc_response(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": { "email": "a@example.com", "name": "A", "posts": { "attach": { "id": 1 } } },
        }),
    )
    .await;
    assert!(unknown
        .error
        .expect("unknown operation")
        .message
        .contains("Unknown nested-write operation"));

    let update_only = call_rpc_response(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": { "email": "b@example.com", "name": "B", "posts": { "disconnect": true } },
        }),
    )
    .await;
    assert!(update_only
        .error
        .expect("update-only operation")
        .message
        .contains("only available on update"));
}

#[tokio::test]
async fn connecting_a_row_that_does_not_exist_is_refused() {
    let (state, _dir) = sqlite_state("nested-missing", schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": {
                "title": "on nothing",
                "author": { "connect": { "email": "nobody@example.com" } },
            },
        }),
    )
    .await;

    assert!(response
        .error
        .expect("connect must not silently pass")
        .message
        .contains("matched no"));
    assert!(rows(&state, "Post", json!({ "title": "on nothing" }))
        .await
        .is_empty());
}
