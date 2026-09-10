//! PostgreSQL executor implementation.
//!
//! The executor itself lives here; parameter binding is in [`bind`] and the
//! decoding of a result row is split by type family under [`decode`],
//! [`decode_plan`], [`arrays`], [`composite`], [`binary`] and [`vector`].

mod arrays;
mod binary;
mod bind;
mod composite;
mod decode;
mod decode_plan;
mod stream;
mod vector;

pub use stream::PgRowStream;

pub(crate) use bind::bind_value;
pub(crate) use stream::{decode_row_internal, decode_rows, streaming_decoder};

use std::time::Duration;

use crate::error::{ConnectorError as Error, Result};
use crate::single_row::{fetch_single_row, SingleRowExpectation};
use crate::{ConnectorPoolOptions, Executor, Row};
use futures::future::BoxFuture;
use nautilus_dialect::Sql;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

/// PostgreSQL executor using sqlx.
///
/// Manages a connection pool and executes queries against PostgreSQL databases.
///
/// ## Example
///
/// ```no_run
/// use nautilus_connector::{ConnectorResult, PgExecutor};
///
/// # async fn example() -> ConnectorResult<()> {
/// let executor = PgExecutor::new("postgres://user:pass@localhost/mydb").await?;
/// # let _ = executor;
/// # Ok(())
/// # }
/// ```
pub struct PgExecutor {
    pool: PgPool,
}

impl PgExecutor {
    /// Create a new PostgreSQL executor with a connection pool.
    ///
    /// ## Parameters
    ///
    /// - `url`: PostgreSQL connection URL (e.g., `postgres://user:pass@localhost/dbname`)
    ///
    /// ## Errors
    ///
    /// Returns `ConnectorError::Connection` if the pool cannot be created or if
    /// an initial connection test fails.
    pub async fn new(url: &str) -> Result<Self> {
        Self::new_with_options(url, ConnectorPoolOptions::default()).await
    }

    /// Create a new PostgreSQL executor with explicit pool overrides.
    ///
    /// Any override not provided keeps the same default used by [`Self::new`].
    pub async fn new_with_options(url: &str, pool_options: ConnectorPoolOptions) -> Result<Self> {
        let connect_options = pool_options.apply_to_postgres_connect_options(
            url.parse::<PgConnectOptions>()
                .map_err(|e| Error::connection(e, "Invalid PostgreSQL connection options"))?,
        );
        let pool = pool_options
            .apply_to(
                PgPoolOptions::new()
                    .max_connections(10)
                    .min_connections(1)
                    .acquire_timeout(Duration::from_secs(10))
                    .idle_timeout(Duration::from_secs(300))
                    .test_before_acquire(true),
            )
            .connect_with(connect_options.clone())
            .await;

        let pool = match pool {
            Ok(pool) => pool,
            Err(error) => return Err(Self::connect_error(error, &connect_options).await),
        };

        Ok(Self { pool })
    }

    /// Turn a pool-creation failure into an error that names the real cause.
    ///
    /// sqlx reports a failure to open the pool's first connection as
    /// `PoolTimedOut`, which hides whatever actually went wrong — a wrong
    /// password, an unreachable host, slow name resolution — behind a message
    /// that only says the acquire timeout elapsed. Opening one connection
    /// directly recovers the underlying error.
    async fn connect_error(error: sqlx::Error, options: &PgConnectOptions) -> Error {
        if !matches!(error, sqlx::Error::PoolTimedOut) {
            return Error::connection(error, "Failed to connect to database");
        }

        match <sqlx::PgConnection as sqlx::Connection>::connect_with(options).await {
            Err(cause) => Error::connection(cause, "Failed to connect to database"),
            Ok(_) => Error::connection(
                error,
                "Failed to connect to database within the 10s acquire timeout",
            ),
        }
    }

    /// Get a reference to the underlying connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Execute a raw SQL statement with no result rows (e.g., DDL).
    pub async fn execute_raw(&self, sql: &str) -> Result<()> {
        sqlx::query(sql)
            .persistent(false)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| Error::database(e, "DDL error"))
    }

    fn execute_collect_internal_with_persistence<'conn>(
        &'conn self,
        sql: &'conn Sql,
        persistent: bool,
    ) -> BoxFuture<'conn, Result<Vec<Row>>> {
        Box::pin(async move {
            let mut conn = self
                .pool
                .acquire()
                .await
                .map_err(|e| Error::connection(e, "Failed to acquire connection"))?;

            let mut query = sqlx::query(&sql.text).persistent(persistent);
            for param in &sql.params {
                query = bind_value(query, param)?;
            }

            // Fetch ALL rows at once so the connection completes the full
            // PostgreSQL extended-query cycle (portal close + ReadyForQuery)
            // before being returned to the pool. The previous streaming
            // approach (`query.fetch`) could leave the connection with an
            // open portal when the stream was dropped mid-iteration, causing
            // sqlx to discard the "dirty" connection and eventually exhaust
            // the pool.
            let pg_rows = query
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| Error::database(e, "Query execution failed"))?;

            drop(conn);

            crate::postgres::decode_rows(&pg_rows)
        })
    }

    fn execute_collect_internal<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Vec<Row>>> {
        self.execute_collect_internal_with_persistence(sql, true)
    }

    /// Execute a SQL query with sqlx statement persistence disabled.
    ///
    /// This is reserved for raw/direct query paths that must stay compatible
    /// with poolers such as PgBouncer transaction pooling.
    pub async fn execute_collect_unprepared(&self, sql: &Sql) -> Result<Vec<Row>> {
        self.execute_collect_internal_with_persistence(sql, false)
            .await
    }

    fn execute_and_fetch_collect_internal<'conn>(
        &'conn self,
        mutation: &'conn Sql,
        fetch: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Vec<Row>>> {
        Box::pin(async move {
            use sqlx::Executor as _;

            let mut conn = self
                .pool
                .acquire()
                .await
                .map_err(|e| Error::connection(e, "Failed to acquire connection"))?;

            let mut mutation_query = sqlx::query(&mutation.text);
            for param in &mutation.params {
                mutation_query = bind_value(mutation_query, param)?;
            }

            (&mut *conn)
                .execute(mutation_query)
                .await
                .map_err(|e| Error::database(e, "Mutation failed"))?;

            let mut fetch_query = sqlx::query(&fetch.text);
            for param in &fetch.params {
                fetch_query = bind_value(fetch_query, param)?;
            }

            let pg_rows = fetch_query
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| Error::database(e, "Fetch failed"))?;

            drop(conn);

            crate::postgres::decode_rows(&pg_rows)
        })
    }

    impl_execute_affected!();
}

/// [`Executor`] implementation backed by a PostgreSQL connection pool.
impl Executor for PgExecutor {
    type Row<'conn>
        = Row
    where
        Self: 'conn;
    type RowStream<'conn>
        = PgRowStream<'conn>
    where
        Self: 'conn;

    fn execute<'conn>(&'conn self, sql: &'conn Sql) -> Self::RowStream<'conn> {
        crate::streaming::spawn_streaming_query(crate::streaming::StreamingQuery::<
            sqlx::Postgres,
            _,
            _,
        > {
            pool: self.pool.clone(),
            sql_text: sql.text.clone(),
            params: sql.params.clone(),
            bind: bind_value,
            decode: crate::postgres::streaming_decoder(),
            query_context: "Query execution failed",
            persistent: true,
        })
    }

    fn execute_owned(&self, sql: Sql) -> crate::row_stream::RowStream<'static> {
        crate::streaming::spawn_streaming_query(crate::streaming::StreamingQuery::<
            sqlx::Postgres,
            _,
            _,
        > {
            pool: self.pool.clone(),
            sql_text: sql.text,
            params: sql.params,
            bind: bind_value,
            decode: crate::postgres::streaming_decoder(),
            query_context: "Query execution failed",
            persistent: true,
        })
    }

    fn execute_and_fetch<'conn>(
        &'conn self,
        mutation: &'conn Sql,
        fetch: &'conn Sql,
    ) -> Self::RowStream<'conn> {
        PgRowStream::from_rows_future(self.execute_and_fetch_collect_internal(mutation, fetch))
    }

    fn execute_collect<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Vec<Self::Row<'conn>>>>
    where
        Self: 'conn,
    {
        self.execute_collect_internal(sql)
    }

    fn execute_one<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Self::Row<'conn>>>
    where
        Self: 'conn,
    {
        Box::pin(async move {
            let mut conn = self
                .pool
                .acquire()
                .await
                .map_err(|e| Error::connection(e, "Failed to acquire connection"))?;

            let row = fetch_single_row::<sqlx::Postgres, _, _, _>(
                &mut *conn,
                &sql.text,
                &sql.params,
                bind_value,
                crate::postgres::decode_row_internal,
                "Query execution failed",
                SingleRowExpectation::ExactlyOne,
            )
            .await?;

            drop(conn);
            // `ExactlyOne` already validated row_count == 1, so `row` is always
            // `Some` here; the fallback keeps this a graceful error, never a panic.
            row.ok_or_else(|| Error::database_msg("Expected exactly one row, got 0"))
        })
    }

    fn execute_optional<'conn>(
        &'conn self,
        sql: &'conn Sql,
    ) -> BoxFuture<'conn, Result<Option<Self::Row<'conn>>>>
    where
        Self: 'conn,
    {
        Box::pin(async move {
            let mut conn = self
                .pool
                .acquire()
                .await
                .map_err(|e| Error::connection(e, "Failed to acquire connection"))?;

            let row = fetch_single_row::<sqlx::Postgres, _, _, _>(
                &mut *conn,
                &sql.text,
                &sql.params,
                bind_value,
                crate::postgres::decode_row_internal,
                "Query execution failed",
                SingleRowExpectation::ZeroOrOne,
            )
            .await?;

            drop(conn);
            Ok(row)
        })
    }
}
