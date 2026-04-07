//! Executor trait for database query execution.

use crate::error::Result;
use futures::stream::Stream;
use nautilus_core::RowAccess;
use nautilus_dialect::Sql;

/// Trait for executing SQL queries against a database.
///
/// This trait uses Generic Associated Types (GAT) to enable:
/// - Database-specific row types with lifetime support
/// - A uniform row stream interface across backends
/// - Zero-copy optimizations where applicable
///
/// Implementors are responsible for:
/// - Managing database connections (pooling, lifecycle)
/// - Binding parameters from `Sql.params` to the query
/// - Executing the query and returning a buffered stream of results
/// - Decoding database rows into types implementing `RowAccess`
/// - Mapping database errors to `nautilus_core::Error`
///
/// ## Thread Safety
///
/// Executors must be `Send + Sync` to allow sharing across async tasks.
///
/// ## Example
///
/// ```rust,ignore
/// use nautilus_connector::{execute_all, Executor, PgExecutor};
/// use nautilus_dialect::{Dialect, PostgresDialect};
/// use nautilus_core::select::SelectBuilder;
/// use futures::stream::StreamExt;
///
/// async fn example() -> nautilus_core::Result<()> {
///     let executor = PgExecutor::new("postgres://localhost/mydb").await?;
///     let dialect = PostgresDialect;
///     
///     let select = SelectBuilder::new("users")
///         .columns(vec!["id", "name"])
///         .build()?;
///     
///     let sql = dialect.render_select(&select)?;
///     
///     // Buffered stream API
///     let mut stream = executor.execute(&sql);
///     while let Some(row) = stream.next().await {
///         let row = row?;
///         println!("{:?}", row);
///     }
///     
///     // Or materialize all rows
///     let rows = execute_all(&executor, &sql).await?;
///     for row in rows {
///         println!("{:?}", row);
///     }
///     
///     Ok(())
/// }
/// ```
pub trait Executor: Send + Sync {
    /// The row type returned by this executor.
    ///
    /// This associated type allows database-specific row implementations
    /// that can borrow data or provide specialized access methods.
    type Row<'conn>: RowAccess<'conn> + Send
    where
        Self: 'conn;

    /// The stream type yielding rows from query execution.
    ///
    /// Current executors eagerly fetch all rows and then yield them through this
    /// stream interface so call sites can stay uniform across backends.
    type RowStream<'conn>: Stream<Item = Result<Self::Row<'conn>>> + Send
    where
        Self: 'conn;

    /// Execute a SQL query and return a stream of rows.
    ///
    /// ## Parameters
    ///
    /// - `sql`: The SQL query with placeholders and bound parameters
    ///
    /// ## Returns
    ///
    /// A buffered stream that yields already-fetched rows one at a time.
    /// Current implementations complete the database fetch before the first
    /// item is yielded.
    ///
    /// ## Errors
    ///
    /// Individual stream items may be `Err` if:
    /// - `ConnectorError::Database`: Query execution failed
    /// - `ConnectorError::RowDecode`: Failed to decode a database value
    fn execute<'conn>(&'conn self, sql: &'conn Sql) -> Self::RowStream<'conn>;

    /// Execute a mutation SQL, drain its results, then execute a fetch SQL
    /// **on the same database connection** and return the fetch rows.
    ///
    /// This is required for databases like MySQL where session-scoped state
    /// such as `LAST_INSERT_ID()` must be read on the connection that
    /// performed the INSERT.
    ///
    /// ## Parameters
    ///
    /// - `mutation`: The INSERT / UPDATE / DELETE statement to execute first
    /// - `fetch`: The SELECT statement whose rows are returned
    ///
    /// ## Returns
    ///
    /// A buffered stream of rows produced by the `fetch` query.
    fn execute_and_fetch<'conn>(
        &'conn self,
        mutation: &'conn Sql,
        fetch: &'conn Sql,
    ) -> Self::RowStream<'conn>;
}

/// Execute a SQL query and materialize all rows into a Vec.
///
/// This is a convenience helper that collects the stream into a vector
/// for cases where you need all rows immediately or want random access.
///
/// ## Parameters
///
/// - `executor`: The executor to run the query against
/// - `sql`: The SQL query with placeholders and bound parameters
///
/// ## Returns
///
/// - `Ok(Vec<E::Row<'conn>>)`: All rows successfully fetched and decoded
/// - `Err(Error)`: Connection, execution, or decoding error
///
/// ## Errors
///
/// - `ConnectorError::Connection`: Failed to acquire database connection
/// - `ConnectorError::Database`: Query execution failed
/// - `ConnectorError::RowDecode`: Failed to decode a row
///
/// ## Example
///
/// ```rust,ignore
/// use nautilus_connector::{execute_all, PgExecutor};
/// use nautilus_dialect::Sql;
///
/// async fn example(executor: &PgExecutor, sql: &Sql) -> nautilus_core::Result<()> {
///     let rows = execute_all(executor, sql).await?;
///     for row in rows {
///         println!("Row: {:?}", row);
///     }
///     Ok(())
/// }
/// ```
pub async fn execute_all<'conn, E>(
    executor: &'conn E,
    sql: &'conn Sql,
) -> Result<Vec<E::Row<'conn>>>
where
    E: Executor + ?Sized,
{
    use futures::stream::StreamExt;

    let stream = executor.execute(sql);
    futures::pin_mut!(stream);

    let mut rows = Vec::new();

    while let Some(result) = stream.next().await {
        rows.push(result?);
    }

    Ok(rows)
}
