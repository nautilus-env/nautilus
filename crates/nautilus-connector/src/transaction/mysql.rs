use std::ops::{Deref, DerefMut};

use sqlx::{
    mysql::MySqlTransactionManager, pool::PoolConnection, MySql, MySqlConnection, MySqlPool,
    TransactionManager,
};

use super::IsolationLevel;
use crate::error::{ConnectorError as Error, Result};

pub(super) enum MysqlTransaction {
    Sqlx(sqlx::Transaction<'static, MySql>),
    Pooled(PooledMysqlTransaction),
}

enum State {
    Preparing,
    Open,
    Closed,
}

pub(super) struct PooledMysqlTransaction {
    connection: PoolConnection<MySql>,
    state: State,
}

impl PooledMysqlTransaction {
    async fn prepare(pool: &MySqlPool, isolation: Option<IsolationLevel>) -> Result<Self> {
        let mut tx = Self {
            connection: pool
                .acquire()
                .await
                .map_err(|e| Error::connection(e, "Failed to acquire transaction connection"))?,
            state: State::Preparing,
        };
        if let Some(isolation) = isolation {
            sqlx::query(&format!(
                "SET TRANSACTION ISOLATION LEVEL {}",
                isolation.as_sql()
            ))
            .persistent(false)
            .execute(&mut *tx.connection)
            .await
            .map_err(|e| Error::database(e, "SET ISOLATION failed"))?;
        }
        Ok(tx)
    }

    async fn commit(mut self) -> std::result::Result<(), sqlx::Error> {
        MySqlTransactionManager::commit(&mut self.connection).await?;
        self.state = State::Closed;
        Ok(())
    }

    async fn rollback(mut self) -> std::result::Result<(), sqlx::Error> {
        MySqlTransactionManager::rollback(&mut self.connection).await?;
        self.state = State::Closed;
        Ok(())
    }
}

impl Drop for PooledMysqlTransaction {
    fn drop(&mut self) {
        match self.state {
            // SET affects the next transaction, and BEGIN may have reached the
            // server without being acknowledged. Neither state may enter the pool.
            State::Preparing => self.connection.close_on_drop(),
            State::Open => MySqlTransactionManager::start_rollback(&mut self.connection),
            State::Closed => {}
        }
    }
}

impl MysqlTransaction {
    pub(super) async fn begin(pool: &MySqlPool, isolation: Option<IsolationLevel>) -> Result<Self> {
        let mut tx = PooledMysqlTransaction::prepare(pool, isolation).await?;
        // Keep ownership outside SQLx's begin future so cancellation can discard
        // this connection. Successful transactions retain normal pool reuse.
        MySqlTransactionManager::begin(&mut tx.connection, None)
            .await
            .map_err(|e| Error::connection(e, "Failed to begin transaction"))?;
        tx.state = State::Open;
        Ok(Self::Pooled(tx))
    }

    pub(super) async fn commit(self) -> std::result::Result<(), sqlx::Error> {
        match self {
            Self::Sqlx(tx) => tx.commit().await,
            Self::Pooled(tx) => tx.commit().await,
        }
    }

    pub(super) async fn rollback(self) -> std::result::Result<(), sqlx::Error> {
        match self {
            Self::Sqlx(tx) => tx.rollback().await,
            Self::Pooled(tx) => tx.rollback().await,
        }
    }
}

impl Deref for MysqlTransaction {
    type Target = MySqlConnection;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Sqlx(tx) => tx,
            Self::Pooled(tx) => &tx.connection,
        }
    }
}

impl DerefMut for MysqlTransaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Sqlx(tx) => tx,
            Self::Pooled(tx) => &mut tx.connection,
        }
    }
}

#[cfg(test)]
mod tests;
