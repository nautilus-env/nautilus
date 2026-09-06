//! Transaction lifetime, plan-cache and statement-execution tests that need
//! a real SQLite-backed [`EngineState`].

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use nautilus_connector::{Client, TransactionExecutor};
use nautilus_core::Value;
use nautilus_dialect::{Sql, SqliteDialect};
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::SchemaIr;
use nautilus_schema::validate_schema_source;
use tempfile::TempDir;

use super::{DatabaseClient, EngineState};

fn parse_ir(source: &str) -> SchemaIr {
    validate_schema_source(source)
        .expect("validation failed")
        .ir
}

fn test_db_url() -> (String, TempDir) {
    let dir = tempfile::Builder::new()
        .prefix("transaction-timeout-state-tests")
        .tempdir()
        .expect("failed to create sqlite test directory");

    let path = dir.path().join("test.db");
    fs::File::create(&path).expect("failed to create sqlite test file");
    let url = format!("sqlite:///{}", path.to_string_lossy().replace('\\', "/"));
    (url, dir)
}

async fn sqlite_state(schema_source: &str) -> (EngineState, TempDir) {
    let schema = parse_ir(schema_source);
    let (database_url, temp_dir) = test_db_url();
    let state = EngineState::new(schema.clone(), database_url, None)
        .await
        .expect("failed to create engine state");

    let ddl = DdlGenerator::new(DatabaseProvider::Sqlite)
        .generate_create_tables(&schema)
        .expect("failed to build ddl");
    state
        .execute_ddl_sql(ddl)
        .await
        .expect("failed to apply ddl");

    (state, temp_dir)
}

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id   Int    @id @default(autoincrement())
  name String
}
"#
}

fn insert_user_sql(name: &str) -> Sql {
    Sql {
        text: r#"INSERT INTO "User" ("name") VALUES (?)"#.to_string(),
        params: vec![Value::String(name.to_string())],
    }
}

fn long_running_sql(iterations: usize) -> Sql {
    Sql {
        text: format!(
            "WITH RECURSIVE cnt(x) AS (SELECT 0 UNION ALL SELECT x + 1 FROM cnt WHERE x < {iterations}) SELECT MAX(x) AS value FROM cnt"
        ),
        params: vec![],
    }
}

async fn count_users(state: &EngineState) -> usize {
    let sql = Sql {
        text: r#"SELECT "id" FROM "User""#.to_string(),
        params: vec![],
    };
    state
        .execute_query_on(&sql, "count users", None)
        .await
        .expect("count query should succeed")
        .len()
}

#[tokio::test]
async fn commit_after_timeout_returns_timeout_and_rolls_back() {
    let (state, temp_dir) = sqlite_state(schema_source()).await;
    let tx_id = "commit-timeout".to_string();

    state
        .begin_transaction(tx_id.clone(), Duration::from_millis(10), None)
        .await
        .expect("transaction should start");
    state
        .execute_affected_on(&insert_user_sql("Alice"), "insert user", Some(&tx_id))
        .await
        .expect("insert inside tx should succeed");

    tokio::time::sleep(Duration::from_millis(30)).await;

    let err = state
        .commit_transaction(&tx_id)
        .await
        .expect_err("commit should time out");
    assert!(matches!(err, ProtocolError::TransactionTimeout(_)));
    assert_eq!(count_users(&state).await, 0);

    let lookup_err = state
        .commit_transaction(&tx_id)
        .await
        .expect_err("late commit should keep surfacing timeout");
    assert!(matches!(lookup_err, ProtocolError::TransactionTimeout(_)));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn rollback_after_timeout_returns_timeout() {
    let (state, temp_dir) = sqlite_state(schema_source()).await;
    let tx_id = "rollback-timeout".to_string();

    state
        .begin_transaction(tx_id.clone(), Duration::from_millis(10), None)
        .await
        .expect("transaction should start");

    tokio::time::sleep(Duration::from_millis(30)).await;

    let err = state
        .rollback_transaction(&tx_id)
        .await
        .expect_err("rollback should time out");
    assert!(matches!(err, ProtocolError::TransactionTimeout(_)));

    let lookup_err = state
        .rollback_transaction(&tx_id)
        .await
        .expect_err("late rollback should keep surfacing timeout");
    assert!(matches!(lookup_err, ProtocolError::TransactionTimeout(_)));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn reaping_idle_transactions_rolls_back_uncommitted_changes() {
    let (state, temp_dir) = sqlite_state(schema_source()).await;
    let tx_id = "idle-timeout".to_string();

    state
        .begin_transaction(tx_id.clone(), Duration::from_millis(10), None)
        .await
        .expect("transaction should start");
    state
        .execute_affected_on(&insert_user_sql("Bob"), "insert user", Some(&tx_id))
        .await
        .expect("insert inside tx should succeed");

    tokio::time::sleep(Duration::from_millis(30)).await;
    state.reap_expired_transactions().await;

    assert_eq!(count_users(&state).await, 0);
    let err = state
        .execute_affected_on(&insert_user_sql("Carol"), "insert user", Some(&tx_id))
        .await
        .expect_err("expired tx should now reject further work");
    assert!(matches!(err, ProtocolError::TransactionTimeout(_)));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn registered_external_transaction_exposes_uncommitted_rows_to_engine_queries() {
    let (state, temp_dir) = sqlite_state(schema_source()).await;

    let tx_client = match &state.client {
        DatabaseClient::Sqlite(client) => {
            let sqlx_tx = client
                .executor()
                .pool()
                .begin()
                .await
                .expect("sqlite transaction should start");
            let tx_exec = TransactionExecutor::sqlite(sqlx_tx);
            Client::new(SqliteDialect, tx_exec)
        }
        _ => panic!("expected sqlite engine state"),
    };

    let tx_id = "external-tx".to_string();
    state
        .register_external_transaction(tx_id.clone(), tx_client.clone(), Duration::from_secs(5))
        .await;

    state
        .execute_affected_on(&insert_user_sql("Dora"), "insert user", Some(&tx_id))
        .await
        .expect("engine should execute writes on registered external transaction");

    let rows = state
        .execute_query_on(
            &Sql {
                text: r#"SELECT "name" FROM "User""#.to_string(),
                params: vec![],
            },
            "select users in tx",
            Some(&tx_id),
        )
        .await
        .expect("engine should read uncommitted rows on registered external transaction");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("name"),
        Some(&Value::String("Dora".to_string())),
    );
    assert_eq!(count_users(&state).await, 0);

    state.unregister_external_transaction(&tx_id).await;
    tx_client
        .executor()
        .rollback()
        .await
        .expect("rollback should succeed");

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn long_running_transaction_query_does_not_block_new_transactions() {
    let (state, temp_dir) = sqlite_state(schema_source()).await;
    let state = Arc::new(state);
    let tx_id = "long-query".to_string();

    state
        .begin_transaction(tx_id.clone(), Duration::from_secs(30), None)
        .await
        .expect("transaction should start");

    let query_state = Arc::clone(&state);
    let query_tx_id = tx_id.clone();
    let long_query = tokio::spawn(async move {
        query_state
            .execute_query_on(
                &long_running_sql(5_000_000),
                "long-running transaction query",
                Some(&query_tx_id),
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(20)).await;

    let second_tx_id = "independent-tx".to_string();
    tokio::time::timeout(
        Duration::from_millis(100),
        state.begin_transaction(second_tx_id.clone(), Duration::from_secs(5), None),
    )
    .await
    .expect("independent transaction start should not wait on another transaction query")
    .expect("second transaction should start successfully");

    state
        .rollback_transaction(&second_tx_id)
        .await
        .expect("second transaction rollback should succeed");

    let rows = long_query
        .await
        .expect("long query task should join")
        .expect("long query should succeed");
    assert_eq!(rows.len(), 1);

    state
        .rollback_transaction(&tx_id)
        .await
        .expect("long-query transaction rollback should succeed");

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn find_unique_typed_caches_simple_eq_plans_per_shape() {
    use nautilus_core::{Expr, FindUniqueArgs};

    let (state, temp_dir) = sqlite_state(schema_source()).await;
    for name in ["Alice", "Bob"] {
        state
            .execute_affected_on(&insert_user_sql(name), "insert user", None)
            .await
            .expect("seed insert should succeed");
    }

    assert_eq!(state.plan_cache().find_unique_len(), 0);

    let by_id_args = FindUniqueArgs::new(Expr::column("User__id").eq(Expr::param(Value::I64(1))));
    let rows = crate::handlers::handle_find_unique_typed(&state, "User", &by_id_args, None)
        .await
        .expect("first findUnique should succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(state.plan_cache().find_unique_len(), 1);

    let by_id_other = FindUniqueArgs::new(Expr::column("User__id").eq(Expr::param(Value::I64(2))));
    let rows = crate::handlers::handle_find_unique_typed(&state, "User", &by_id_other, None)
        .await
        .expect("second findUnique should reuse the cached plan");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        state.plan_cache().find_unique_len(),
        1,
        "identical filter shape should not grow the cache",
    );

    let by_name_args = FindUniqueArgs::new(
        Expr::column("User__name").eq(Expr::param(Value::String("Alice".to_string()))),
    );
    let rows = crate::handlers::handle_find_unique_typed(&state, "User", &by_name_args, None)
        .await
        .expect("differently shaped findUnique should also succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(state.plan_cache().find_unique_len(), 2);

    let by_id_gt = FindUniqueArgs::new(Expr::column("User__id").gt(Expr::param(Value::I64(0))));
    let _ = crate::handlers::handle_find_unique_typed(&state, "User", &by_id_gt, None)
        .await
        .expect("non-cacheable filter should still execute");
    assert_eq!(state.plan_cache().find_unique_len(), 2);

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn find_many_typed_caches_parametric_plans_per_shape() {
    use nautilus_core::{Expr, FindManyArgs};

    let (state, temp_dir) = sqlite_state(schema_source()).await;
    for name in ["Alice", "Bob", "Cara"] {
        state
            .execute_affected_on(&insert_user_sql(name), "insert user", None)
            .await
            .expect("seed insert should succeed");
    }

    assert_eq!(state.plan_cache().find_many_len(), 0);

    let first = FindManyArgs {
        where_: Some(Expr::column("User__id").gt(Expr::param(Value::I64(0)))),
        take: Some(2),
        ..Default::default()
    };
    let rows = crate::handlers::handle_find_many_typed(&state, "User", &first, None)
        .await
        .expect("first findMany should succeed");
    assert_eq!(rows.len(), 2);
    assert_eq!(state.plan_cache().find_many_len(), 1);

    let other_value = FindManyArgs {
        where_: Some(Expr::column("User__id").gt(Expr::param(Value::I64(2)))),
        take: Some(2),
        ..Default::default()
    };
    let rows = crate::handlers::handle_find_many_typed(&state, "User", &other_value, None)
        .await
        .expect("second findMany should reuse the cached plan");
    assert_eq!(
        rows.len(),
        1,
        "replayed plan must bind the fresh parameter value"
    );
    assert_eq!(state.plan_cache().find_many_len(), 1);

    let other_take = FindManyArgs {
        where_: Some(Expr::column("User__id").gt(Expr::param(Value::I64(0)))),
        take: Some(1),
        ..Default::default()
    };
    let rows = crate::handlers::handle_find_many_typed(&state, "User", &other_take, None)
        .await
        .expect("different take should also succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(state.plan_cache().find_many_len(), 2);

    let in_list = FindManyArgs {
        where_: Some(
            Expr::column("User__id")
                .in_list(vec![Expr::param(Value::I64(1)), Expr::param(Value::I64(2))]),
        ),
        ..Default::default()
    };
    let rows = crate::handlers::handle_find_many_typed(&state, "User", &in_list, None)
        .await
        .expect("non-cacheable filter should still execute");
    assert_eq!(rows.len(), 2);
    assert_eq!(state.plan_cache().find_many_len(), 2);

    drop(state);
    drop(temp_dir);
}
