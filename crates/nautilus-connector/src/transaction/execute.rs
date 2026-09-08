//! Running one statement on a live transaction.
//!
//! The generic runners below hold what every backend does — bind the
//! parameters, take the connection out of the handle, run the statement and
//! decode the answer — while the macros beneath them name each backend's
//! binder and decoder once, so a query method does not repeat the three arms.

use std::ops::DerefMut;

use futures::future::BoxFuture;
use nautilus_core::Value;

use super::handle::TxHandle;
use crate::error::{ConnectorError as Error, Result};
use crate::single_row::{fetch_single_row, SingleRowExpectation};
use crate::Row;

fn bind_query<'q, DB, Bind>(
    sql_text: &'q str,
    params: &'q [Value],
    persistent: bool,
    bind: Bind,
) -> Result<sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>>
where
    DB: sqlx::Database + sqlx::database::HasStatementCache,
    for<'q2> <DB as sqlx::Database>::Arguments<'q2>: sqlx::IntoArguments<'q2, DB>,
    Bind: Fn(
        sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>,
        &'q Value,
    ) -> Result<sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>>,
{
    let mut query = sqlx::query(sql_text).persistent(persistent);
    for param in params {
        query = bind(query, param)?;
    }
    Ok(query)
}

pub(super) fn affected_on<DB, Tx, Bind, RowsAffected>(
    tx_arc: TxHandle<Tx>,
    sql_text: String,
    params: Vec<Value>,
    persistent: bool,
    bind: Bind,
    rows_affected: RowsAffected,
) -> BoxFuture<'static, Result<usize>>
where
    Tx: DerefMut<Target = DB::Connection> + Send + 'static,
    DB: sqlx::Database + sqlx::database::HasStatementCache + Send + 'static,
    for<'c> &'c mut <DB as sqlx::Database>::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> <DB as sqlx::Database>::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    for<'q> Bind: Fn(
            sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>,
            &'q Value,
        ) -> Result<sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>>
        + Copy
        + Send
        + 'static,
    RowsAffected: Fn(<DB as sqlx::Database>::QueryResult) -> u64 + Copy + Send + 'static,
{
    Box::pin(async move {
        let query = bind_query::<DB, Bind>(&sql_text, &params, persistent, bind)?;
        let mut guard = tx_arc.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| Error::database_msg("Transaction already closed"))?;

        use sqlx::Executor as _;
        let result = (&mut **tx)
            .execute(query)
            .await
            .map_err(|e| Error::database(e, "Mutation failed"))?;
        Ok(rows_affected(result) as usize)
    })
}

pub(super) fn collect_on<DB, Tx, Bind, Decode>(
    tx_arc: TxHandle<Tx>,
    sql_text: String,
    params: Vec<Value>,
    persistent: bool,
    bind: Bind,
    decode: Decode,
    query_context: &'static str,
) -> BoxFuture<'static, Result<Vec<Row>>>
where
    Tx: DerefMut<Target = DB::Connection> + Send + 'static,
    DB: sqlx::Database + sqlx::database::HasStatementCache + Send + 'static,
    for<'c> &'c mut <DB as sqlx::Database>::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> <DB as sqlx::Database>::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    for<'q> Bind: Fn(
            sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>,
            &'q Value,
        ) -> Result<sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>>
        + Copy
        + Send
        + 'static,
    Decode: Fn(&[<DB as sqlx::Database>::Row]) -> Result<Vec<Row>> + Send + 'static,
{
    Box::pin(async move {
        let query = bind_query::<DB, Bind>(&sql_text, &params, persistent, bind)?;
        let mut guard = tx_arc.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| Error::database_msg("Transaction already closed"))?;

        use sqlx::Executor as _;
        let rows = (&mut **tx)
            .fetch_all(query)
            .await
            .map_err(|e| Error::database(e, query_context))?;
        drop(guard);

        decode(&rows)
    })
}

pub(super) fn and_fetch_on<DB, Tx, Bind, Decode>(
    tx_arc: TxHandle<Tx>,
    mutation_text: String,
    mutation_params: Vec<Value>,
    fetch_text: String,
    fetch_params: Vec<Value>,
    bind: Bind,
    decode: Decode,
) -> BoxFuture<'static, Result<Vec<Row>>>
where
    Tx: DerefMut<Target = DB::Connection> + Send + 'static,
    DB: sqlx::Database + sqlx::database::HasStatementCache + Send + 'static,
    for<'c> &'c mut <DB as sqlx::Database>::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> <DB as sqlx::Database>::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    for<'q> Bind: Fn(
            sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>,
            &'q Value,
        ) -> Result<sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>>
        + Copy
        + Send
        + 'static,
    Decode: Fn(&[<DB as sqlx::Database>::Row]) -> Result<Vec<Row>> + Send + 'static,
{
    Box::pin(async move {
        let mutation_query = bind_query::<DB, Bind>(&mutation_text, &mutation_params, true, bind)?;
        let fetch_query = bind_query::<DB, Bind>(&fetch_text, &fetch_params, true, bind)?;
        let mut guard = tx_arc.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| Error::database_msg("Transaction already closed"))?;

        use sqlx::Executor as _;
        (&mut **tx)
            .execute(mutation_query)
            .await
            .map_err(|e| Error::database(e, "Mutation failed"))?;

        let rows = (&mut **tx)
            .fetch_all(fetch_query)
            .await
            .map_err(|e| Error::database(e, "Fetch failed"))?;
        drop(guard);

        decode(&rows)
    })
}

pub(super) fn single_on<DB, Tx, Bind, Decode>(
    tx_arc: TxHandle<Tx>,
    sql_text: String,
    params: Vec<Value>,
    bind: Bind,
    decode: Decode,
    query_context: &'static str,
    expectation: SingleRowExpectation,
) -> BoxFuture<'static, Result<Option<Row>>>
where
    Tx: DerefMut<Target = DB::Connection> + Send + 'static,
    DB: sqlx::Database + Send + 'static,
    for<'c> &'c mut <DB as sqlx::Database>::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> <DB as sqlx::Database>::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    for<'q> Bind: Fn(
            sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>,
            &'q Value,
        ) -> Result<sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments<'q>>>
        + Copy
        + Send
        + 'static,
    Decode: Fn(<DB as sqlx::Database>::Row) -> Result<Row> + Copy + Send + 'static,
{
    Box::pin(async move {
        let mut guard = tx_arc.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| Error::database_msg("Transaction already closed"))?;

        fetch_single_row::<DB, _, _, _>(
            &mut **tx,
            &sql_text,
            &params,
            bind,
            decode,
            query_context,
            expectation,
        )
        .await
    })
}

/// Run a mutation and answer with the number of rows it affected.
macro_rules! tx_affected {
    ($inner:expr, $text:expr, $params:expr) => {
        match $inner {
            TransactionInner::Postgres(handle) => execute::affected_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                true,
                crate::postgres::bind_value,
                |result: sqlx::postgres::PgQueryResult| result.rows_affected(),
            ),
            TransactionInner::Mysql(handle) => execute::affected_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                true,
                crate::mysql::bind_value,
                |result: sqlx::mysql::MySqlQueryResult| result.rows_affected(),
            ),
            TransactionInner::Sqlite(handle) => execute::affected_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                true,
                crate::sqlite::bind_value,
                |result: sqlx::sqlite::SqliteQueryResult| result.rows_affected(),
            ),
        }
    };
}

/// Run a query and answer with every row it returned.
macro_rules! tx_collect {
    ($inner:expr, $text:expr, $params:expr, $persistent:expr, $context:expr) => {
        match $inner {
            TransactionInner::Postgres(handle) => execute::collect_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                $persistent,
                crate::postgres::bind_value,
                crate::postgres::decode_rows,
                $context,
            ),
            TransactionInner::Mysql(handle) => execute::collect_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                $persistent,
                crate::mysql::bind_value,
                crate::mysql_stream::decode_rows,
                $context,
            ),
            TransactionInner::Sqlite(handle) => execute::collect_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                $persistent,
                crate::sqlite::bind_value,
                crate::sqlite_stream::decode_rows,
                $context,
            ),
        }
    };
}

/// Run a mutation and then a fetch on the same connection.
macro_rules! tx_and_fetch {
    ($inner:expr, $mutation:expr, $fetch:expr) => {
        match $inner {
            TransactionInner::Postgres(handle) => execute::and_fetch_on(
                std::sync::Arc::clone(handle),
                $mutation.text.clone(),
                $mutation.params.clone(),
                $fetch.text.clone(),
                $fetch.params.clone(),
                crate::postgres::bind_value,
                crate::postgres::decode_rows,
            ),
            TransactionInner::Mysql(handle) => execute::and_fetch_on(
                std::sync::Arc::clone(handle),
                $mutation.text.clone(),
                $mutation.params.clone(),
                $fetch.text.clone(),
                $fetch.params.clone(),
                crate::mysql::bind_value,
                crate::mysql_stream::decode_rows,
            ),
            TransactionInner::Sqlite(handle) => execute::and_fetch_on(
                std::sync::Arc::clone(handle),
                $mutation.text.clone(),
                $mutation.params.clone(),
                $fetch.text.clone(),
                $fetch.params.clone(),
                crate::sqlite::bind_value,
                crate::sqlite_stream::decode_rows,
            ),
        }
    };
}

/// Run a query that must answer with at most one row.
macro_rules! tx_single {
    ($inner:expr, $text:expr, $params:expr, $context:expr, $expectation:expr) => {
        match $inner {
            TransactionInner::Postgres(handle) => execute::single_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                crate::postgres::bind_value,
                crate::postgres::decode_row_internal,
                $context,
                $expectation,
            ),
            TransactionInner::Mysql(handle) => execute::single_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                crate::mysql::bind_value,
                crate::mysql_stream::decode_row_internal,
                $context,
                $expectation,
            ),
            TransactionInner::Sqlite(handle) => execute::single_on(
                std::sync::Arc::clone(handle),
                $text,
                $params,
                crate::sqlite::bind_value,
                crate::sqlite_stream::decode_row_internal,
                $context,
                $expectation,
            ),
        }
    };
}

pub(super) use {tx_affected, tx_and_fetch, tx_collect, tx_single};
