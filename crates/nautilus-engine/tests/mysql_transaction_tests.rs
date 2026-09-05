//! Verify MySQL isolation through the engine's transaction registry.
//!
//! Each protocol isolation level is checked on the server for both commit and
//! rollback. Reusing one pooled connection verifies that subsequent transactions
//! return to default isolation while preserving only committed writes.

#[path = "../../nautilus-connector/tests/common/mysql_isolation.rs"]
mod mysql_isolation;

use std::time::Duration;

use nautilus_connector::ConnectorPoolOptions;
use nautilus_dialect::Sql;
use nautilus_engine::{state::DatabaseClient, EngineState};
use nautilus_protocol::IsolationLevel;

#[tokio::test]
#[ignore = "requires dedicated MySQL with performance_schema access via MYSQL_URL"]
async fn engine_mysql_isolation_is_effective_and_connection_is_reused() {
    let observer = mysql_isolation::observer().await;
    let schema = nautilus_schema::validate_schema_source(
        r#"
datasource db {
  provider = "mysql"
  url = env("DATABASE_URL")
}
model Probe {
  id Int @id
}
"#,
    )
    .unwrap()
    .ir;
    let state = EngineState::new_with_pool_options(
        schema,
        mysql_isolation::database_url(),
        None,
        ConnectorPoolOptions::new().max_connections(1),
    )
    .await
    .unwrap();
    let DatabaseClient::Mysql(client) = &state.client else {
        panic!("expected MySQL client");
    };
    let pool = client.executor().pool();
    let id = mysql_isolation::connection_id(pool).await;
    client
        .executor()
        .execute_raw("CREATE TEMPORARY TABLE isolation_probe (value INT) ENGINE=InnoDB")
        .await
        .unwrap();
    let mut committed = 0i64;

    for level in [
        IsolationLevel::ReadUncommitted,
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ] {
        for commit in [true, false] {
            state
                .begin_transaction("override".to_string(), Duration::from_secs(10), Some(level))
                .await
                .unwrap();
            mysql_isolation::assert_level(&observer, id, level.as_sql()).await;
            state
                .execute_query_on(
                    &Sql {
                        text: "INSERT INTO isolation_probe VALUES (1)".to_string(),
                        params: vec![],
                    },
                    "isolation probe",
                    Some("override"),
                )
                .await
                .unwrap();
            if commit {
                state.commit_transaction("override").await.unwrap();
                committed += 1;
            } else {
                state.rollback_transaction("override").await.unwrap();
            }
            assert_eq!(mysql_isolation::connection_id(pool).await, id);
            let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM isolation_probe")
                .fetch_one(pool)
                .await
                .unwrap();
            assert_eq!(rows, committed);
            state
                .begin_transaction("default".to_string(), Duration::from_secs(10), None)
                .await
                .unwrap();
            mysql_isolation::assert_level(&observer, id, "REPEATABLE READ").await;
            state.rollback_transaction("default").await.unwrap();
            assert_eq!(mysql_isolation::connection_id(pool).await, id);
        }
    }
}
