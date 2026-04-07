use nautilus_connector::{execute_all, ConnectorResult, Executor, MysqlExecutor};
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
    std::env::var("MYSQL_URL")
        .unwrap_or_else(|_| "mysql://nautilus:nautilus@localhost/nautilus_test".to_string())
}

pub async fn setup_executor() -> ConnectorResult<MysqlExecutor> {
    MysqlExecutor::new(&database_url()).await
}

pub async fn setup_test_users_table(executor: &MysqlExecutor) -> ConnectorResult<()> {
    execute_sql(
        executor,
        r#"
            CREATE TABLE IF NOT EXISTS test_users (
                id BIGINT PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT,
                age INT,
                score DOUBLE,
                active BOOLEAN,
                data BLOB
            )
        "#,
    )
    .await
}
