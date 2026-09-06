//! Statement execution: every read and write reaches the database through one
//! of these entry points, which pick the pooled, direct or transaction-scoped
//! connection and time what they run.

use nautilus_connector::{execute_all, Executor, Row, RowStream};
use nautilus_dialect::Sql;
use nautilus_protocol::ProtocolError;

use crate::observability::StatementTimer;
use crate::state::database::connector_to_protocol;
use crate::state::EngineState;

impl EngineState {
    /// Execute raw DDL SQL statements against the database.
    ///
    /// Used for running migrations (CREATE TABLE, etc.).
    pub async fn execute_ddl_sql(
        &self,
        statements: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for stmt in &statements {
            if stmt.trim().is_empty() {
                continue;
            }
            self.client.execute_raw(stmt).await?;
        }
        Ok(())
    }

    /// Start timing a statement for the slow-statement log.
    ///
    /// Only the paths that materialise their result set are timed: the
    /// streaming path hands the connection to the consumer, so its duration
    /// belongs to the caller draining the rows rather than to the statement.
    fn time_statement<'a>(&self, sql: &'a Sql, context: &'a str) -> StatementTimer<'a> {
        StatementTimer::start(self.slow_query_threshold, context, &sql.text)
    }

    /// Execute a SQL query, optionally inside a transaction.
    ///
    /// If `tx_id` is `Some`, the query runs on the transaction's connection;
    /// otherwise it runs on the pool-backed default connection.
    pub async fn execute_query_on(
        &self,
        sql: &Sql,
        context: &str,
        tx_id: Option<&str>,
    ) -> Result<Vec<Row>, ProtocolError> {
        let timer = self.time_statement(sql, context);
        let rows = match tx_id {
            None => self.client.execute_query(sql, context).await,
            Some(id) => {
                let tx_client = self.transaction_client_for_request(id).await?;
                execute_all(tx_client.executor(), sql)
                    .await
                    .map_err(|e| connector_to_protocol(e, context))
            }
        };
        timer.finish();
        rows
    }

    /// Execute a SQL query and return a row-by-row stream, optionally inside a
    /// transaction.
    ///
    /// Unlike [`Self::execute_query_on`], which buffers the full result set,
    /// this path keeps memory bounded for large reads by streaming each row
    /// through a worker-owned connection. The returned stream is `'static`,
    /// so the caller can move it between tasks; dropping the stream
    /// mid-iteration releases the connection cleanly.
    ///
    /// Used by the chunked `findMany` IPC path so partial responses can be
    /// emitted as rows arrive from the database, without first materialising
    /// the whole `Vec<Row>`.
    pub async fn execute_query_stream_on(
        &self,
        sql: Sql,
        tx_id: Option<&str>,
    ) -> Result<RowStream<'static>, ProtocolError> {
        match tx_id {
            None => Ok(self.client.execute_query_stream(sql)),
            Some(id) => {
                let tx_client = self.transaction_client_for_request(id).await?;
                Ok(tx_client.executor().execute_owned(sql))
            }
        }
    }

    /// Execute a SQL query using the direct connection when available, otherwise the pooled one.
    ///
    /// Raw SQL queries should use this so they bypass connection poolers
    /// (e.g. PgBouncer) when possible and disable sqlx statement persistence.
    /// If a `tx_id` is provided the query always runs on the transaction's
    /// connection regardless.
    pub async fn execute_direct_query_on(
        &self,
        sql: &Sql,
        context: &str,
        tx_id: Option<&str>,
    ) -> Result<Vec<Row>, ProtocolError> {
        let timer = self.time_statement(sql, context);
        let rows = match tx_id {
            Some(tx_id) => {
                let tx_client = self.transaction_client_for_request(tx_id).await?;
                tx_client
                    .executor()
                    .execute_collect_unprepared(sql)
                    .await
                    .map_err(|e| connector_to_protocol(e, context))
            }
            None => match &self.direct_client {
                Some(direct) => direct.execute_query_unprepared(sql, context).await,
                None => self.client.execute_query_unprepared(sql, context).await,
            },
        };
        timer.finish();
        rows
    }

    /// Execute a mutation SQL and return the affected-row count, optionally
    /// inside a transaction.
    ///
    /// Use this when `return_data = false` so no RETURNING clause is emitted.
    pub async fn execute_affected_on(
        &self,
        sql: &Sql,
        context: &str,
        tx_id: Option<&str>,
    ) -> Result<usize, ProtocolError> {
        let timer = self.time_statement(sql, context);
        let affected = match tx_id {
            None => self.client.execute_affected(sql, context).await,
            Some(id) => {
                let tx_client = self.transaction_client_for_request(id).await?;
                tx_client
                    .executor()
                    .execute_affected(sql)
                    .await
                    .map_err(|e| connector_to_protocol(e, context))
            }
        };
        timer.finish();
        affected
    }
}
