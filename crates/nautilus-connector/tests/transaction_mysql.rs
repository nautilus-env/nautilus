#[path = "common/mysql_isolation.rs"]
mod mysql_isolation;

use std::time::Duration;

use nautilus_connector::{
    execute_all, Client, ConnectorError, ConnectorPoolOptions, ConnectorResult, IsolationLevel,
    MysqlExecutor, TransactionExecutor, TransactionOptions,
};
use nautilus_dialect::Sql;

async fn insert_probe(tx: &TransactionExecutor) -> ConnectorResult<()> {
    execute_all(
        tx,
        &Sql {
            text: "INSERT INTO isolation_probe VALUES (1)".to_string(),
            params: vec![],
        },
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires dedicated MySQL with performance_schema access via MYSQL_URL"]
async fn isolation_override_is_effective_and_does_not_leak_on_connection_reuse() {
    let observer = mysql_isolation::observer().await;
    let client = Client::mysql_with_options(
        &mysql_isolation::database_url(),
        ConnectorPoolOptions::new().max_connections(1),
    )
    .await
    .unwrap();
    let pool = client.executor().pool();
    let id = mysql_isolation::connection_id(pool).await;
    client
        .executor()
        .execute_raw("CREATE TEMPORARY TABLE isolation_probe (value INT) ENGINE=InnoDB")
        .await
        .unwrap();

    for (index, level) in [
        IsolationLevel::ReadUncommitted,
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ]
    .into_iter()
    .enumerate()
    {
        let observer = &observer;
        client
            .transaction(
                TransactionOptions {
                    isolation_level: Some(level),
                    ..Default::default()
                },
                |tx| async move {
                    insert_probe(tx.executor()).await?;
                    mysql_isolation::assert_level(observer, id, level.as_sql()).await;
                    Ok(())
                },
            )
            .await
            .unwrap();
        assert_eq!(mysql_isolation::connection_id(pool).await, id);
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM isolation_probe")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(rows, (index + 1) as i64);
        client
            .transaction(TransactionOptions::default(), |_tx| async {
                mysql_isolation::assert_level(observer, id, "REPEATABLE READ").await;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(mysql_isolation::connection_id(pool).await, id);
    }
}

async fn assert_rolled_back(client: &Client<MysqlExecutor>, observer: &sqlx::MySqlPool, id: u64) {
    let pool = client.executor().pool();
    assert_eq!(mysql_isolation::connection_id(pool).await, id);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM isolation_probe")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "failed/cancelled callbacks must roll back their writes"
    );
    client
        .transaction(TransactionOptions::default(), |_tx| async {
            mysql_isolation::assert_level(observer, id, "REPEATABLE READ").await;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires dedicated MySQL with performance_schema access via MYSQL_URL"]
async fn callback_error_timeout_and_cancellation_roll_back_without_leaking_isolation() {
    let observer = mysql_isolation::observer().await;
    let client = Client::mysql_with_options(
        &mysql_isolation::database_url(),
        ConnectorPoolOptions::new().max_connections(1),
    )
    .await
    .unwrap();
    client
        .executor()
        .execute_raw("CREATE TEMPORARY TABLE isolation_probe (value INT) ENGINE=InnoDB")
        .await
        .unwrap();
    let id = mysql_isolation::connection_id(client.executor().pool()).await;
    let options = TransactionOptions {
        timeout: Duration::from_millis(100),
        isolation_level: Some(IsolationLevel::Serializable),
    };
    for callback_error in [true, false] {
        let result: ConnectorResult<()> = client
            .transaction(options.clone(), |tx| async move {
                insert_probe(tx.executor()).await?;
                if callback_error {
                    Err(ConnectorError::database_msg("intentional callback failure"))
                } else {
                    std::future::pending().await
                }
            })
            .await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains(if callback_error {
            "intentional callback failure"
        } else {
            "Transaction timed out"
        }));
        assert_rolled_back(&client, &observer, id).await;
    }

    let worker_client = client.clone();
    let (inserted, ready) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        worker_client
            .transaction(
                TransactionOptions {
                    timeout: Duration::ZERO,
                    ..options
                },
                |tx| async move {
                    insert_probe(tx.executor()).await?;
                    inserted.send(()).unwrap();
                    std::future::pending::<ConnectorResult<()>>().await
                },
            )
            .await
    });
    ready.await.unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    assert_rolled_back(&client, &observer, id).await;
}
