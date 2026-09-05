//! Verify cleanup after isolation is set but before `BEGIN` consumes the override.
//!
//! Real MySQL connections make pending isolation and connection replacement
//! observable after cancellation or an error during preparation.

#[path = "../../../tests/common/mysql_isolation.rs"]
mod mysql_isolation;

use super::{MysqlTransaction, PooledMysqlTransaction};
use crate::IsolationLevel;
use sqlx::mysql::MySqlPoolOptions;

/// Verify that preparation discarded the old connection and its pending override.
///
/// A successful transaction must reuse the replacement with default isolation.
async fn assert_clean_replacement(pool: &sqlx::MySqlPool, observer: &sqlx::MySqlPool, old_id: u64) {
    let new_id = mysql_isolation::connection_id(pool).await;
    assert_ne!(
        new_id, old_id,
        "an incompletely prepared connection must be discarded"
    );
    let tx = MysqlTransaction::begin(pool, None).await.unwrap();
    mysql_isolation::assert_level(observer, new_id, "REPEATABLE READ").await;
    tx.rollback().await.unwrap();
    assert_eq!(mysql_isolation::connection_id(pool).await, new_id);
}

#[tokio::test]
#[ignore = "requires dedicated MySQL with performance_schema access via MYSQL_URL"]
async fn cancelled_isolation_preparation_discards_the_connection() {
    let observer = mysql_isolation::observer().await;
    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect(&mysql_isolation::database_url())
        .await
        .unwrap();
    let old_id = mysql_isolation::connection_id(&pool).await;
    let worker_pool = pool.clone();
    let (prepared, ready) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        let tx = PooledMysqlTransaction::prepare(&worker_pool, Some(IsolationLevel::ReadCommitted))
            .await
            .unwrap();
        prepared.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(tx);
    });
    ready.await.unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    assert_clean_replacement(&pool, &observer, old_id).await;
}

#[tokio::test]
#[ignore = "requires dedicated MySQL with performance_schema access via MYSQL_URL"]
async fn failed_isolation_preparation_discards_the_connection() {
    let observer = mysql_isolation::observer().await;
    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect(&mysql_isolation::database_url())
        .await
        .unwrap();
    let old_id = mysql_isolation::connection_id(&pool).await;
    let result: Result<(), sqlx::Error> = async {
        let mut tx = PooledMysqlTransaction::prepare(&pool, Some(IsolationLevel::Serializable))
            .await
            .unwrap();
        // Fail after SET has succeeded but before a transaction can consume it.
        sqlx::query("INVALID TRANSACTION STATEMENT")
            .execute(&mut *tx.connection)
            .await?;
        Ok(())
    }
    .await;
    assert!(result.is_err());
    assert_clean_replacement(&pool, &observer, old_id).await;
}
