//! Database connections: the per-backend client enum and the statement
//! primitives every engine path executes through.

use std::sync::Arc;

use nautilus_connector::{
    execute_all, Client, Executor, MysqlExecutor, PgExecutor, Row, RowStream, SqliteExecutor,
    SqlxErrorKind,
};
use nautilus_dialect::{Dialect, MysqlDialect, PostgresDialect, Sql, SqliteDialect};
use nautilus_migrate::DatabaseProvider;
use nautilus_protocol::{PoolMetrics, ProtocolError};

use crate::pool_options::EnginePoolOptions;

/// Convert a [`nautilus_connector::ConnectorError`] to the appropriate [`ProtocolError`],
/// mapping specific constraint violation kinds to their dedicated error codes.
pub(crate) fn connector_to_protocol(
    e: nautilus_connector::ConnectorError,
    context: &str,
) -> ProtocolError {
    let msg = format!("{}: {}", context, e);
    match e.sqlx_kind() {
        SqlxErrorKind::UniqueConstraint => ProtocolError::UniqueConstraintViolation(msg),
        SqlxErrorKind::ForeignKeyConstraint => ProtocolError::ForeignKeyConstraintViolation(msg),
        SqlxErrorKind::CheckConstraint => ProtocolError::CheckConstraintViolation(msg),
        SqlxErrorKind::NullConstraint => ProtocolError::NullConstraintViolation(msg),
        SqlxErrorKind::Deadlock => ProtocolError::Deadlock(msg),
        SqlxErrorKind::SerializationFailure => ProtocolError::SerializationFailure(msg),
        SqlxErrorKind::PoolTimedOut | SqlxErrorKind::PoolClosed => {
            ProtocolError::ConnectionFailed(msg)
        }
        _ => ProtocolError::DatabaseExecution(msg),
    }
}

/// Enum to hold different client types.
pub enum DatabaseClient {
    /// PostgreSQL client.
    Postgres(Client<PgExecutor>),
    /// MySQL client.
    Mysql(Client<MysqlExecutor>),
    /// SQLite client.
    Sqlite(Client<SqliteExecutor>),
}

/// Dispatch an expression across all [`DatabaseClient`] variants.
macro_rules! with_client {
    ($self:expr, $client:ident => $body:expr) => {
        match $self {
            DatabaseClient::Postgres($client) => $body,
            DatabaseClient::Mysql($client) => $body,
            DatabaseClient::Sqlite($client) => $body,
        }
    };
}

impl DatabaseClient {
    /// Connection-pool counters as reported by the driver.
    pub fn pool_metrics(&self) -> PoolMetrics {
        with_client!(self, client => PoolMetrics {
            size: client.executor().pool().size(),
            idle: client.executor().pool().num_idle(),
        })
    }

    /// Execute a rendered SQL query and return all result rows.
    pub async fn execute_query(&self, sql: &Sql, context: &str) -> Result<Vec<Row>, ProtocolError> {
        with_client!(self, client => {
            execute_all(client.executor(), sql)
                .await
                .map_err(|e| connector_to_protocol(e, context))
        })
    }

    /// Execute a rendered SQL query with sqlx statement persistence disabled.
    ///
    /// This is used only for raw/direct query paths that may run through
    /// PgBouncer-style transaction poolers.
    pub async fn execute_query_unprepared(
        &self,
        sql: &Sql,
        context: &str,
    ) -> Result<Vec<Row>, ProtocolError> {
        match self {
            DatabaseClient::Postgres(client) => client
                .executor()
                .execute_collect_unprepared(sql)
                .await
                .map_err(|e| connector_to_protocol(e, context)),
            DatabaseClient::Mysql(client) => execute_all(client.executor(), sql)
                .await
                .map_err(|e| connector_to_protocol(e, context)),
            DatabaseClient::Sqlite(client) => execute_all(client.executor(), sql)
                .await
                .map_err(|e| connector_to_protocol(e, context)),
        }
    }

    /// Execute a mutation SQL and return the number of affected rows.
    pub async fn execute_affected(&self, sql: &Sql, context: &str) -> Result<usize, ProtocolError> {
        with_client!(self, client => {
            client.executor()
                .execute_affected(sql)
                .await
                .map_err(|e| connector_to_protocol(e, context))
        })
    }

    /// Execute a raw DDL statement (no parameters, no result rows).
    pub async fn execute_raw(&self, stmt: &str) -> Result<(), Box<dyn std::error::Error>> {
        with_client!(self, client => client.executor().execute_raw(stmt).await?);
        Ok(())
    }

    /// Execute a rendered SQL query and return a row-by-row stream that owns
    /// its database connection.
    ///
    /// Unlike [`Self::execute_query`], which materialises the full result set,
    /// this path drives the underlying sqlx stream from a worker task. The
    /// returned [`RowStream`] is `'static` and can be moved between tasks; if
    /// the consumer drops it mid-iteration, the worker drains the remaining
    /// rows so the connection returns to the pool clean.
    pub fn execute_query_stream(&self, sql: Sql) -> RowStream<'static> {
        with_client!(self, client => client.executor().execute_owned(sql))
    }
}

/// Connect to a database and return a `(dialect, client)` pair.
pub(super) async fn build_client(
    provider: DatabaseProvider,
    url: &str,
    pool_options: EnginePoolOptions,
) -> Result<(Arc<dyn Dialect + Send + Sync>, DatabaseClient), Box<dyn std::error::Error>> {
    let connector_pool_options = pool_options.to_connector_pool_options();
    match provider {
        DatabaseProvider::Postgres => {
            let pg_client = Client::postgres_with_options(url, connector_pool_options).await?;
            let dialect: Arc<dyn Dialect + Send + Sync> = Arc::new(PostgresDialect);
            Ok((dialect, DatabaseClient::Postgres(pg_client)))
        }
        DatabaseProvider::Mysql => {
            let mysql_client = Client::mysql_with_options(url, connector_pool_options).await?;
            let dialect: Arc<dyn Dialect + Send + Sync> = Arc::new(MysqlDialect);
            Ok((dialect, DatabaseClient::Mysql(mysql_client)))
        }
        DatabaseProvider::Sqlite => {
            let sqlite_client = Client::sqlite_with_options(url, connector_pool_options).await?;
            let dialect: Arc<dyn Dialect + Send + Sync> = Arc::new(SqliteDialect);
            Ok((dialect, DatabaseClient::Sqlite(sqlite_client)))
        }
    }
}

/// Resolve database URL, handling env() references.
pub(super) fn resolve_database_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    nautilus_schema::resolve_env_url(url).map_err(|msg| msg.into())
}
