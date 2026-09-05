//! Shared observation helpers for connector and engine MySQL isolation tests.
//!
//! Tests require a dedicated MySQL instance with default `REPEATABLE READ`
//! isolation. The configured user must be able to enable transaction
//! instrumentation and read active transactions in `performance_schema`.

use sqlx::mysql::{MySqlPool, MySqlPoolOptions};

/// Read `MYSQL_URL`, falling back to the local test instance's root credentials.
pub fn database_url() -> String {
    std::env::var("MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:nautilus_root@localhost/nautilus_test".to_string())
}

/// Enable transaction instrumentation and open a separate observation pool.
///
/// This changes server instrumentation settings and panics if access is missing;
/// isolation tests must run against a dedicated instance.
pub async fn observer() -> MySqlPool {
    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect(
            "isolation tests require a dedicated MySQL instance with performance_schema access",
        );
    for sql in [
        "UPDATE performance_schema.setup_consumers SET ENABLED = 'YES' WHERE NAME = 'events_transactions_current'",
        "UPDATE performance_schema.setup_instruments SET ENABLED = 'YES' WHERE NAME = 'transaction'",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool
}

/// Read the server connection ID to detect physical connection reuse.
///
/// Callers use a pool limited to one connection and release transactions first.
pub async fn connection_id(pool: &MySqlPool) -> u64 {
    sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Assert the effective isolation of an active transaction on a known connection.
///
/// Reads transaction instrumentation because `@@session.transaction_isolation`
/// does not expose an override that applies only to the next transaction.
pub async fn assert_level(observer: &MySqlPool, connection_id: u64, expected: &str) {
    let actual: String = sqlx::query_scalar(
        "SELECT e.ISOLATION_LEVEL FROM performance_schema.events_transactions_current e \
         JOIN performance_schema.threads t ON t.THREAD_ID = e.THREAD_ID \
         WHERE t.PROCESSLIST_ID = ? AND e.STATE = 'ACTIVE'",
    )
    .bind(connection_id)
    .fetch_one(observer)
    .await
    .expect("the transaction should be active and instrumented");
    assert_eq!(actual, expected);
}
