//! Which database a command talks to, and how it talks to it.
//!
//! The URL policy is written once here: an explicit flag wins, then the
//! datasource's `direct_url`, then `DATABASE_URL`, then the datasource's
//! runtime `url` — so an admin command can prefer a direct connection while
//! runtime traffic keeps the pooled one. [`Connection`] is the pool that URL
//! opens, whichever provider it names.

use anyhow::{bail, Context};
use nautilus_migrate::{
    ApplyFailure, ApplyOutcome, ApplyPlan, DatabaseProvider, SchemaInspector,
    TransactionRequirement,
};
use nautilus_schema::ir::SchemaIr;

use crate::context::environment::CommandEnv;

/// Resolve an admin/database-tooling URL from (in order): explicit flag,
/// datasource `direct_url`, `DATABASE_URL` env var, or datasource `url`.
///
/// This mirrors Prisma-style behavior where CLI/admin flows can prefer a
/// direct connection while runtime traffic continues to use the pooled `url`.
pub fn resolve_db_url(
    db_url_arg: Option<String>,
    schema_ir: &SchemaIr,
    env: &CommandEnv,
) -> anyhow::Result<String> {
    if let Some(raw) = db_url_arg.as_deref() {
        return resolve_url(raw, env);
    }

    let datasource = schema_ir.datasource.as_ref();

    if let Some(raw) = datasource.and_then(|ds| ds.direct_url.as_deref()) {
        if let Ok(url) = resolve_url(raw, env) {
            return Ok(url);
        }
    }

    if let Some(raw) = env.var("DATABASE_URL") {
        return resolve_url(&raw, env);
    }

    if let Some(raw) = datasource
        .filter(|ds| !ds.url.is_empty())
        .map(|ds| ds.url.as_str())
    {
        return resolve_url(raw, env);
    }

    bail!(
        "No database URL found. Use --database-url, set datasource direct_url/url, \
         or set DATABASE_URL."
    )
}

/// How far one transaction phase got before it stopped.
struct PhaseFailure {
    /// Statements of the phase that had already run.
    attempted: usize,
    /// The statement blamed for the failure.
    statement: String,
    /// The error as the database or driver reported it.
    message: String,
}

/// Tri-variant connection wrapper around sqlx pool types.
///
/// Each `nautilus db` subcommand resolves a database URL and uses this enum to
/// execute raw SQL against SQLite, PostgreSQL, or MySQL without the caller
/// needing to know the concrete driver.
pub enum Connection {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
    Mysql(sqlx::MySqlPool),
}

/// Execute `$body` against the inner pool of every [`Connection`] variant.
///
/// `$self` must be a `&Connection`, `$pool` is the binding name for the
/// inner pool reference. The macro expands a `match` arm for each variant.
macro_rules! with_pool {
    ($self:expr, $pool:ident => $body:expr) => {
        match $self {
            Connection::Sqlite($pool) => $body,
            Connection::Postgres($pool) => $body,
            Connection::Mysql($pool) => $body,
        }
    };
}

impl Connection {
    /// Open a connection pool for the given `provider`.
    ///
    /// For SQLite the database file is created if it does not exist.
    pub async fn connect(url: &str, provider: DatabaseProvider) -> anyhow::Result<Self> {
        match provider {
            DatabaseProvider::Sqlite => {
                use sqlx::sqlite::SqliteConnectOptions;
                use std::str::FromStr;
                let opts = SqliteConnectOptions::from_str(url)
                    .context("Invalid SQLite URL")?
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(opts)
                    .await
                    .context("SQLite connection failed")?;
                Ok(Connection::Sqlite(pool))
            }
            DatabaseProvider::Postgres => {
                let pool = sqlx::PgPool::connect_with(postgres_connect_options(url)?)
                    .await
                    .context("PostgreSQL connection failed")?;
                Ok(Connection::Postgres(pool))
            }
            DatabaseProvider::Mysql => {
                let pool = sqlx::MySqlPool::connect(url)
                    .await
                    .context("MySQL connection failed")?;
                Ok(Connection::Mysql(pool))
            }
        }
    }

    /// Execute multiple SQL statements inside a single transaction.
    ///
    /// On any error the transaction is rolled back and the error is returned —
    /// as far as the provider allows, since MySQL commits implicitly around most
    /// DDL. Batches of generated DDL go through [`Self::apply_plan`],
    /// which phases them and reports what survived.
    pub async fn execute_in_transaction(&self, stmts: &[String]) -> anyhow::Result<()> {
        with_pool!(self, pool => {
            let mut tx = pool.begin().await.context("begin transaction")?;
            for sql in stmts {
                sqlx::query(sql)
                    .persistent(false)
                    .execute(&mut *tx)
                    .await
                    .context("transaction error")?;
            }
            tx.commit().await.context("commit transaction")?;
        });
        Ok(())
    }

    /// Execute SQL statements one at a time, each committing on its own.
    ///
    /// Used for standalone phases in an [`ApplyPlan`].
    pub async fn execute_each(&self, stmts: &[String]) -> anyhow::Result<()> {
        with_pool!(self, pool => {
            for sql in stmts {
                sqlx::query(sql)
                    .persistent(false)
                    .execute(pool)
                    .await
                    .context("statement error")?;
            }
        });
        Ok(())
    }

    /// Execute `stmts` in a single transaction, reporting how far it got.
    ///
    /// The index tells the caller how many statements had already run when the
    /// transaction stopped, which is what decides whether their effect is still
    /// in the database.
    async fn execute_transaction_phase(&self, stmts: &[String]) -> Result<(), PhaseFailure> {
        with_pool!(self, pool => {
            let mut tx = pool.begin().await.map_err(|e| PhaseFailure {
                attempted: 0,
                statement: stmts.first().cloned().unwrap_or_default(),
                message: format!("begin transaction: {e}"),
            })?;
            for (index, sql) in stmts.iter().enumerate() {
                sqlx::query(sql)
                    .persistent(false)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| PhaseFailure {
                        attempted: index,
                        statement: sql.clone(),
                        message: e.to_string(),
                    })?;
            }
            tx.commit().await.map_err(|e| PhaseFailure {
                attempted: stmts.len(),
                statement: stmts.last().cloned().unwrap_or_default(),
                message: format!("commit transaction: {e}"),
            })?;
        });
        Ok(())
    }

    /// Run `statements` in the order given, opening a transaction around each
    /// run of statements that can share one.
    ///
    /// The plan carries transaction boundaries and provider rollback behavior.
    pub async fn apply_plan(&self, plan: &ApplyPlan) -> ApplyOutcome {
        for phase in plan.phases() {
            let stmts = &plan.statements()[phase.statement_range()];
            let failure = match phase.transaction() {
                TransactionRequirement::Standalone => {
                    self.execute_each(stmts).await.err().map(|e| PhaseFailure {
                        attempted: 0,
                        statement: stmts[0].clone(),
                        message: format!("{e:#}"),
                    })
                }
                TransactionRequirement::Shared => self.execute_transaction_phase(stmts).await.err(),
            };

            match failure {
                None => {}
                Some(failure) => {
                    return plan.stopped(
                        phase,
                        failure.attempted,
                        ApplyFailure {
                            statement: failure.statement,
                            message: failure.message,
                        },
                    )
                }
            }
        }

        ApplyOutcome::committed_all(plan.statements().len())
    }

    /// Execute a raw SQL script inside a single transaction.
    ///
    /// The script is passed through to the database driver unchanged so the
    /// server, rather than the CLI, determines real statement boundaries.
    pub async fn execute_script_in_transaction(&self, script: &str) -> anyhow::Result<()> {
        with_pool!(self, pool => {
            let mut tx = pool.begin().await.context("begin transaction")?;
            sqlx::raw_sql(script)
                .execute(&mut *tx)
                .await
                .context("transaction error")?;
            tx.commit().await.context("commit transaction")?;
        });
        Ok(())
    }
}

fn postgres_connect_options(url: &str) -> anyhow::Result<sqlx::postgres::PgConnectOptions> {
    use std::str::FromStr;

    // Disable SQLx's persistent statement cache for CLI/admin Postgres commands.
    // This keeps `nautilus db *` compatible with PgBouncer transaction pooling
    // and similar proxies that reject reusing named prepared statements.
    sqlx::postgres::PgConnectOptions::from_str(url)
        .map(|options| options.statement_cache_capacity(0))
        .context("Invalid PostgreSQL URL")
}

/// Unwrap `env(VAR)` syntax against the command environment; otherwise return
/// the URL as-is.
pub fn resolve_url(raw: &str, env: &CommandEnv) -> anyhow::Result<String> {
    nautilus_schema::resolve_env_url_with(raw, |key| env.var(key))
        .map_err(|msg| anyhow::anyhow!(msg))
}

/// Infer the [`DatabaseProvider`] from a connection URL prefix.
pub fn detect_provider(url: &str) -> anyhow::Result<DatabaseProvider> {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Ok(DatabaseProvider::Postgres)
    } else if url.starts_with("mysql://") {
        Ok(DatabaseProvider::Mysql)
    } else if url.starts_with("sqlite:") {
        Ok(DatabaseProvider::Sqlite)
    } else {
        bail!("Cannot detect database provider from URL: {}", url)
    }
}

/// Replace the password/token segment of a URL with `***` for safe display.
pub fn obfuscate_url(url: &str) -> String {
    if let Some(at) = url.rfind('@') {
        if let Some(scheme_end) = url.find("://") {
            let scheme = &url[..scheme_end + 3];
            let host_onwards = &url[at..];
            return format!("{}***{}", scheme, host_onwards);
        }
    }
    url.to_string()
}

/// An inspector that scans exactly the schemas the datasource declares.
///
/// Without the list the inspector reads `current_schema()` alone and leaves
/// every table unqualified, which is what a single-schema datasource wants.
pub fn inspector_for(
    provider: DatabaseProvider,
    database_url: &str,
    schema_ir: &SchemaIr,
) -> SchemaInspector {
    let schemas = schema_ir
        .datasource
        .as_ref()
        .map(|ds| ds.schemas.clone())
        .unwrap_or_default();
    SchemaInspector::new(provider, database_url).with_schemas(schemas)
}

#[cfg(test)]
mod tests {
    use super::{postgres_connect_options, resolve_db_url};
    use crate::context::environment::CommandEnv;
    use nautilus_schema::validate_schema_source;

    fn parse_schema_ir(source: &str) -> nautilus_schema::ir::SchemaIr {
        validate_schema_source(source)
            .expect("schema should validate")
            .ir
    }

    #[test]
    fn postgres_connect_options_disable_statement_cache() {
        let options = postgres_connect_options("postgres://user:pass@localhost/db")
            .expect("expected valid PostgreSQL options");

        let rendered = format!("{options:?}");
        assert!(rendered.contains("statement_cache_capacity: 0"));
    }

    #[test]
    fn resolve_db_url_prefers_direct_url_for_admin_flows() {
        let schema_ir = parse_schema_ir(
            r#"
datasource db {
  provider   = "postgresql"
  url        = "postgres://pooled/runtime"
  direct_url = "postgres://direct/admin"
}

model User {
  id Int @id
}
"#,
        );

        let url = resolve_db_url(None, &schema_ir, &CommandEnv::fixed("."))
            .expect("expected database url");
        assert_eq!(url, "postgres://direct/admin");
    }

    #[test]
    fn resolve_db_url_falls_back_to_runtime_url_when_direct_url_missing() {
        let schema_ir = parse_schema_ir(
            r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://pooled/runtime"
}

model User {
  id Int @id
}
"#,
        );

        let url = resolve_db_url(None, &schema_ir, &CommandEnv::fixed("."))
            .expect("expected database url");
        assert_eq!(url, "postgres://pooled/runtime");
    }

    #[test]
    fn resolve_db_url_falls_back_to_runtime_url_when_direct_url_env_is_unset() {
        let schema_ir = parse_schema_ir(
            r#"
datasource db {
  provider   = "postgresql"
  url        = "postgres://pooled/runtime"
  direct_url = env("__NAUTILUS_TEST_UNSET_DIRECT_URL__")
}

model User {
  id Int @id
}
"#,
        );

        let url = resolve_db_url(None, &schema_ir, &CommandEnv::fixed("."))
            .expect("expected database url");
        assert_eq!(url, "postgres://pooled/runtime");
    }
}
