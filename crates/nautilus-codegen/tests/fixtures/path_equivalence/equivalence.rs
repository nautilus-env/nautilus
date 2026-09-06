mod client;
mod common;
mod engine;

use nautilus_client::{Client, User, UserCreateInput};
use nautilus_connector::SqliteExecutor;
use nautilus_engine::EngineState;
use serde_json::{json, Value};
use tempfile::TempDir;

const SCHEMA: &str = include_str!("schema.nautilus");

async fn fixture() -> (EngineState, Client<SqliteExecutor>, TempDir) {
    let (state, dir) = common::sqlite_state("path-equivalence", SCHEMA).await;
    let url = format!(
        "sqlite:///{}",
        dir.path()
            .join("test.db")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let client = Client::sqlite(&url).await.unwrap();
    (state, client, dir)
}

fn data() -> Value {
    json!({
        "email": "o'hara@example.com", "name": null, "role": "ADMIN",
        "balance": "1234.50", "joined": "2026-09-05T10:20:30Z",
        "metadata": {"literal": "It's \"fine\"", "empty": null, "tags": ["A", "b"]}
    })
}

fn input() -> UserCreateInput {
    UserCreateInput {
        email: Some("o'hara@example.com".into()),
        name: Some(None),
        role: Some(nautilus_client::Role::ADMIN),
        balance: Some("1234.50".parse().unwrap()),
        joined: Some("2026-09-05T10:20:30".parse().unwrap()),
        metadata: Some(data()["metadata"].clone()),
        ..Default::default()
    }
}

fn user_value(user: &User) -> Value {
    json!({
        "id": user.id, "email": user.email, "name": user.name, "role": user.role,
        "balance": user.balance.to_string(), "joined": user.joined.to_string(),
        "metadata": user.metadata
    })
}

fn assert_user(user: &User) {
    assert_eq!(user.id, 1);
    assert_eq!(user.email, "o'hara@example.com");
    assert_eq!(user.name, None);
    assert_eq!(user.role, nautilus_client::Role::ADMIN);
    assert_eq!(user.balance, "1234.50".parse().unwrap());
    assert_eq!(
        user.joined,
        "2026-09-05T10:20:30"
            .parse::<chrono::NaiveDateTime>()
            .unwrap()
    );
    assert_eq!(user.metadata, data()["metadata"]);
}
