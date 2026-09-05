//! Shared database connection helpers used by all `nautilus db` subcommands.

use anyhow::{bail, Context};
use nautilus_migrate::{
    order_changes_for_apply, plan_apply_phases, ApplyFailure, ApplyOutcome, ApplyPhase, Change,
    ChangeRisk, DatabaseProvider, DiffApplier, GroupStatus, LiveSchema, SchemaInspector,
};
use nautilus_schema::{discover_schema_paths_in_current_dir, ir::SchemaIr, SchemaSet};
use std::path::{Path, PathBuf};

use crate::tui;

/// Locate the `.nautilus` schema file.
///
/// Priority: explicit `--schema` argument -> first `.nautilus` file in the
/// current directory. Returns an error if neither is available.
pub fn resolve_schema_path(schema_arg: Option<String>) -> anyhow::Result<PathBuf> {
    maybe_resolve_schema_path(schema_arg.as_deref())?.context(
        "Schema file not found. Pass --schema <path> or create a .nautilus \
         file in the current directory.",
    )
}

/// Resolve the schema path when it is optional for the caller.
pub(crate) fn maybe_resolve_schema_path(
    schema_arg: Option<&str>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = schema_arg {
        return Ok(Some(PathBuf::from(path)));
    }

    let nautilus_files = discover_schema_paths_in_current_dir()
        .context("Failed to inspect current directory for .nautilus schema files")?;
    let schema_path = nautilus_files.first().cloned();

    if let Some(path) = &schema_path {
        if nautilus_files.len() > 1 {
            tui::eprint_warning(&format!(
                "multiple .nautilus files found, using: {}",
                path.display()
            ));
        }
    }

    Ok(schema_path)
}

/// Lex, parse, and validate a schema, returning the [`SchemaIr`].
///
/// `path` may name a single `.nautilus` file or a directory holding several, in
/// which case they are assembled into one schema.
pub fn parse_and_validate_schema(path: &std::path::Path) -> anyhow::Result<SchemaIr> {
    let set = SchemaSet::load_path(path)
        .with_context(|| format!("Cannot read schema: {}", path.display()))?;

    set.validate()
        .map(|validated| validated.ir)
        .map_err(|e| anyhow::anyhow!("{}", set.format_error(&e)))
}

/// Resolve an admin/database-tooling URL from (in order): explicit flag,
/// datasource `direct_url`, `DATABASE_URL` env var, or datasource `url`.
///
/// This mirrors Prisma-style behavior where CLI/admin flows can prefer a
/// direct connection while runtime traffic continues to use the pooled `url`.
pub fn resolve_db_url(db_url_arg: Option<String>, schema_ir: &SchemaIr) -> anyhow::Result<String> {
    if let Some(raw) = db_url_arg.as_deref() {
        return resolve_url(raw);
    }

    let datasource = schema_ir.datasource.as_ref();

    if let Some(raw) = datasource.and_then(|ds| ds.direct_url.as_deref()) {
        if let Ok(url) = resolve_url(raw) {
            return Ok(url);
        }
    }

    if let Ok(raw) = std::env::var("DATABASE_URL") {
        return resolve_url(&raw);
    }

    if let Some(raw) = datasource
        .filter(|ds| !ds.url.is_empty())
        .map(|ds| ds.url.as_str())
    {
        return resolve_url(raw);
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
    /// DDL. Batches of generated DDL go through [`Self::apply_statements`],
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
    /// Used for the statements a transaction cannot carry — see
    /// [`nautilus_migrate::requires_own_transaction`].
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
    /// A statement listed by [`nautilus_migrate::requires_own_transaction`]
    /// commits on its own and splits the run it sits in, rather than being
    /// hoisted ahead of the statements it depends on. On MySQL most DDL commits
    /// implicitly, so a phase there is a transaction in name only; the returned
    /// [`ApplyOutcome`] accounts for both.
    pub async fn apply_statements(
        &self,
        statements: &[String],
        provider: DatabaseProvider,
    ) -> ApplyOutcome {
        let total = statements.len();
        let mut committed = 0;

        for phase in plan_apply_phases(statements) {
            let failure = match &phase {
                ApplyPhase::Standalone(sql) => self
                    .execute_each(std::slice::from_ref(sql))
                    .await
                    .err()
                    .map(|e| PhaseFailure {
                        attempted: 0,
                        statement: sql.clone(),
                        message: format!("{e:#}"),
                    }),
                ApplyPhase::Transaction(stmts) => self.execute_transaction_phase(stmts).await.err(),
            };

            match failure {
                None => committed += phase.len(),
                Some(failure) => {
                    return ApplyOutcome::stopped(
                        total,
                        committed,
                        failure.attempted,
                        provider,
                        ApplyFailure {
                            statement: failure.statement,
                            message: failure.message,
                        },
                    )
                }
            }
        }

        ApplyOutcome::committed_all(total)
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

/// Unwrap `env(VAR)` syntax; otherwise return the URL as-is.
pub fn resolve_url(raw: &str) -> anyhow::Result<String> {
    nautilus_schema::resolve_env_url(raw).map_err(|msg| anyhow::anyhow!(msg))
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

/// Everything a `nautilus db` subcommand typically needs after loading the
/// schema, resolving the URL, and connecting to the database.
pub struct DbContext {
    pub schema_ir: SchemaIr,
    pub database_url: String,
    pub provider: DatabaseProvider,
    pub conn: Connection,
}

impl DbContext {
    /// An inspector scoped to the schemas this datasource declares.
    pub fn inspector(&self) -> SchemaInspector {
        inspector_for(self.provider, &self.database_url, &self.schema_ir)
    }

    /// Parse a schema file, resolve the database URL, connect, and inspect the
    /// provider — the shared preamble of `push`, `status`, and `reset`.
    pub async fn build(
        schema_arg: Option<String>,
        db_url_arg: Option<String>,
    ) -> anyhow::Result<Self> {
        let schema_path = resolve_schema_path(schema_arg)?;

        load_dotenv_for_schema(&schema_path);

        let sp = tui::spinner("Parsing schema…");
        let schema_ir = parse_and_validate_schema(&schema_path)?;

        let model_count = schema_ir.models.len();
        let provider_name = schema_ir
            .datasource
            .as_ref()
            .map(|ds| ds.provider.clone())
            .unwrap_or_else(|| "unknown".to_string());

        tui::spinner_ok(
            sp,
            &format!(
                "Schema parsed  ({} model{}, {})",
                model_count,
                if model_count == 1 { "" } else { "s" },
                provider_name,
            ),
        );

        let database_url = resolve_db_url(db_url_arg, &schema_ir)?;

        let sp = tui::spinner("Connecting to database…");
        let provider = detect_provider(&database_url)?;
        let conn = Connection::connect(&database_url, provider)
            .await
            .with_context(|| format!("Failed to connect to {}", database_url))?;
        tui::spinner_ok(sp, &format!("Connected  {}", obfuscate_url(&database_url)));

        Ok(DbContext {
            schema_ir,
            database_url,
            provider,
            conn,
        })
    }
}

/// Short human-readable label for a [`Change`] (used in progress lines).
pub fn change_display_name(change: &Change) -> String {
    match change {
        Change::NewTable(m) => m.db_name.clone(),
        Change::DroppedTable { name } => name.to_string(),
        Change::AddedColumn { table, field } => format!("{}.{}", table, field.db_name),
        Change::DroppedColumn { table, column }
        | Change::TypeChanged { table, column, .. }
        | Change::NullabilityChanged { table, column, .. }
        | Change::DefaultChanged { table, column, .. }
        | Change::AutoIncrementChanged { table, column, .. }
        | Change::ComputedExprChanged { table, column, .. } => format!("{}.{}", table, column),
        Change::CheckChanged {
            table,
            column: Some(col),
            ..
        } => format!("{}.{}", table, col),
        Change::CheckChanged {
            table,
            column: None,
            ..
        } => format!("{} (CHECK)", table),
        Change::PrimaryKeyChanged { table } => format!("{} (PK)", table),
        Change::IndexAdded { table, columns, .. } | Change::IndexDropped { table, columns, .. } => {
            format!("{} ({})", table, columns.join(","))
        }
        Change::CreateCompositeType { name }
        | Change::DropCompositeType { name }
        | Change::AlterCompositeType { name, .. } => format!("type:{}", name),
        Change::CreateEnum { name, .. }
        | Change::DropEnum { name }
        | Change::AlterEnum { name, .. } => format!("enum:{}", name),
        Change::CreateExtension { name, .. } | Change::DropExtension { name } => {
            format!("ext:{}", name)
        }
        Change::CreateSchema { name } => format!("schema:{}", name),
        Change::ForeignKeyAdded { table, columns, .. } => {
            format!("{} (fk:{})", table, columns.join(","))
        }
        Change::ForeignKeyDropped {
            table,
            constraint_name,
        } => format!("{} (fk:{})", table, constraint_name),
    }
}

/// What applying a set of changes left in the database.
pub struct AppliedChanges {
    /// Changes whose statements are all committed.
    pub applied: usize,
    /// Changes that did not survive: rolled back, stopped on, or never reached.
    pub failed: usize,
    /// Whether the database kept part of a batch that then failed, so it
    /// matches neither the previous schema nor the requested one.
    pub partial: bool,
}

/// Apply a list of classified changes through the given [`DiffApplier`].
///
/// The generated SQL runs in dependency order, in as few transactions as the
/// provider allows. It is **not** one atomic unit: `ALTER TYPE ... ADD VALUE`
/// has to commit on its own, and MySQL commits implicitly around most DDL. A
/// failure therefore rolls back at most the phase it happened in, and the
/// report says which changes the database kept.
pub async fn apply_changes(
    classified: &[(Change, ChangeRisk)],
    applier: &DiffApplier<'_>,
    live: &LiveSchema,
    conn: &Connection,
    provider: DatabaseProvider,
) -> anyhow::Result<AppliedChanges> {
    let ordered_changes = order_changes_for_apply(
        &classified
            .iter()
            .map(|(change, _risk)| change.clone())
            .collect::<Vec<_>>(),
        live,
    );
    let mut change_stmts: Vec<(String, Vec<String>)> = Vec::new();
    for change in &ordered_changes {
        let label = change_display_name(change);
        let stmts = applier
            .sql_for(change)
            .map_err(|e| anyhow::anyhow!("SQL generation failed for {}: {}", label, e))?;
        change_stmts.push((label, stmts));
    }

    let all_stmts: Vec<String> = change_stmts
        .iter()
        .flat_map(|(_, stmts)| stmts.iter().cloned())
        .collect();
    let group_sizes: Vec<usize> = change_stmts.iter().map(|(_, s)| s.len()).collect();

    let sp = tui::spinner("Applying…");
    let outcome = conn.apply_statements(&all_stmts, provider).await;
    let statuses = outcome.classify_groups(&group_sizes);

    let Some(failure) = &outcome.failure else {
        tui::spinner_ok(sp, "All changes committed");
        for (label, _) in &change_stmts {
            tui::print_ok(label);
        }
        return Ok(AppliedChanges {
            applied: change_stmts.len(),
            failed: 0,
            partial: false,
        });
    };

    tui::spinner_err(sp, phase_summary(&outcome));

    let mut applied = 0;
    for ((label, stmts), status) in change_stmts.iter().zip(&statuses) {
        match status {
            GroupStatus::Applied => {
                applied += 1;
                tui::print_ok(label);
            }
            GroupStatus::RolledBack => tui::print_err_line(&format!("{label} (rolled back)")),
            GroupStatus::NotAttempted => tui::print_err_line(&format!("{label} (not attempted)")),
            GroupStatus::Failed { committed } => {
                tui::print_err_line(&format!("{label} ({})", failed_change_note(*committed)));
                for sql in stmts {
                    if *sql == failure.statement {
                        eprintln!("  [sql] {}   <- stopped here", sql);
                    } else {
                        eprintln!("  [sql] {}", sql);
                    }
                }
            }
        }
    }

    tui::print_table_err("Statement", &failure.message);

    Ok(AppliedChanges {
        applied,
        failed: change_stmts.len() - applied,
        partial: outcome.left_partial_state(),
    })
}

/// One line describing how much of the batch the database kept.
fn phase_summary(outcome: &nautilus_migrate::ApplyOutcome) -> &'static str {
    if outcome.left_partial_state() {
        "Stopped part-way — earlier statements are committed"
    } else {
        "Stopped — the failed transaction rolled back"
    }
}

/// How to describe the change the apply stopped on.
fn failed_change_note(committed: usize) -> String {
    if committed == 0 {
        "failed".to_string()
    } else {
        format!("failed after {committed} committed statement(s)")
    }
}

/// Load a `.env` file and inject its entries into the process environment.
///
/// Search order (first file found wins):
///   1. Directory containing the schema file.
///   2. Current working directory.
///
/// Already-set variables are never overwritten (shell exports take priority).
/// Supports `KEY=VALUE` and `KEY="VALUE"` / `KEY='VALUE'`; `#` comments; blank
/// lines. No variable-expansion is performed.
pub(crate) fn load_dotenv_for_schema(schema_path: &Path) {
    let schema_dir = if schema_path.is_dir() {
        schema_path.to_path_buf()
    } else {
        schema_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let search_dirs = [
        schema_dir,
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    ];

    for dir in search_dirs {
        let candidate = dir.join(".env");
        if !candidate.is_file() {
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(&candidate) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let mut value = value.trim();
                    if value.len() >= 2 {
                        let (first, last) =
                            (value.as_bytes()[0], value.as_bytes()[value.len() - 1]);
                        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                            value = &value[1..value.len() - 1];
                        }
                    }
                    if !key.is_empty() && std::env::var(key).is_err() {
                        // SAFETY: single-threaded context (before async spawn)
                        #[allow(clippy::disallowed_methods)]
                        std::env::set_var(key, value);
                    }
                }
            }
        }
        return; // first file found wins
    }
}

#[cfg(test)]
mod tests {
    use super::{postgres_connect_options, resolve_db_url, resolve_schema_path};
    use crate::test_support::{lock_process_env, lock_working_dir, CurrentDirGuard, EnvVarGuard};
    use nautilus_schema::validate_schema_source;
    use tempfile::TempDir;

    fn parse_schema_ir(source: &str) -> nautilus_schema::ir::SchemaIr {
        validate_schema_source(source)
            .expect("schema should validate")
            .ir
    }

    #[test]
    fn resolve_schema_path_auto_detects_first_nautilus_file() {
        let _cwd_lock = lock_working_dir();
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());

        std::fs::write(
            temp_dir.path().join("custom.nautilus"),
            "model User { id Int @id }\n",
        )
        .expect("failed to write custom schema");
        std::fs::write(
            temp_dir.path().join("alpha.nautilus"),
            "model Post { id Int @id }\n",
        )
        .expect("failed to write alpha schema");

        let resolved = resolve_schema_path(None).expect("schema should auto-resolve");
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("alpha.nautilus")
        );
    }

    #[test]
    fn resolve_schema_path_errors_when_no_nautilus_files_exist() {
        let _cwd_lock = lock_working_dir();
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());

        let err = resolve_schema_path(None).expect_err("missing schema should fail");
        assert!(err
            .to_string()
            .contains("Schema file not found. Pass --schema <path> or create a .nautilus file"));
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
        let _env_lock = lock_process_env();
        let _env_guard = EnvVarGuard::unset("DATABASE_URL");
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

        let url = resolve_db_url(None, &schema_ir).expect("expected database url");
        assert_eq!(url, "postgres://direct/admin");
    }

    #[test]
    fn resolve_db_url_falls_back_to_runtime_url_when_direct_url_missing() {
        let _env_lock = lock_process_env();
        let _env_guard = EnvVarGuard::unset("DATABASE_URL");
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

        let url = resolve_db_url(None, &schema_ir).expect("expected database url");
        assert_eq!(url, "postgres://pooled/runtime");
    }

    #[test]
    fn resolve_db_url_falls_back_to_runtime_url_when_direct_url_env_is_unset() {
        let _env_lock = lock_process_env();
        let _env_guard = EnvVarGuard::unset("DATABASE_URL");
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

        let url = resolve_db_url(None, &schema_ir).expect("expected database url");
        assert_eq!(url, "postgres://pooled/runtime");
    }
}
