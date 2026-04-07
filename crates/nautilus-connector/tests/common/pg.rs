use nautilus_connector::{execute_all, ConnectorResult, Executor, PgExecutor};
use nautilus_dialect::Sql;

async fn execute_sql<E>(executor: &E, text: &str) -> ConnectorResult<()>
where
    E: Executor + ?Sized,
{
    let sql = Sql {
        text: text.to_string(),
        params: vec![],
    };
    execute_all(executor, &sql).await?;
    Ok(())
}

pub fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://nautilus:nautilus@localhost/nautilus_test".to_string())
}

pub async fn setup_executor() -> ConnectorResult<PgExecutor> {
    PgExecutor::new(&database_url()).await
}

pub async fn setup_test_users_table(executor: &PgExecutor) -> ConnectorResult<()> {
    execute_sql(
        executor,
        r#"
            CREATE TABLE IF NOT EXISTS test_users (
                id BIGINT PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT,
                age INT,
                score FLOAT8,
                active BOOLEAN,
                data BYTEA
            )
        "#,
    )
    .await?;
    execute_sql(executor, "TRUNCATE TABLE test_users").await
}

pub async fn teardown_test_users_table(executor: &PgExecutor) -> ConnectorResult<()> {
    execute_sql(executor, "DROP TABLE IF EXISTS test_users").await
}
