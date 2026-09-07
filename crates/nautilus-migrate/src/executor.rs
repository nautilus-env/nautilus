use crate::applier::DiffApplier;
use crate::apply::{plan_apply_phases, ApplyFailure, ApplyOutcome, ApplyPhase};
use crate::ddl::{DatabaseProvider, DdlGenerator};
use crate::diff::{order_changes_for_apply, Change};
use crate::error::{MigrationError, Result};
use crate::live::LiveSchema;
use crate::migration::Migration;
use crate::reverse::ChangeReverser;
use crate::tracker::MigrationTracker;
use nautilus_schema::ir::SchemaIr;
use sqlx::AnyPool;
use std::sync::Arc;
use std::time::Instant;

/// Executes schema migrations
pub struct MigrationExecutor {
    pool: Arc<AnyPool>,
    tracker: MigrationTracker,
    generator: DdlGenerator,
}

impl MigrationExecutor {
    /// Create a new migration executor
    pub fn new(pool: AnyPool, provider: DatabaseProvider) -> Self {
        let pool_arc = Arc::new(pool);
        Self {
            pool: pool_arc.clone(),
            tracker: MigrationTracker::new(pool_arc, provider),
            generator: DdlGenerator::new(provider),
        }
    }

    /// Initialize migration tracking (create _nautilus_migrations table)
    pub async fn init(&self) -> Result<()> {
        self.tracker.init().await
    }

    /// Generate a migration from a schema
    pub fn generate_migration_from_schema(
        &self,
        name: String,
        schema: &SchemaIr,
    ) -> Result<Migration> {
        let up_sql = self.generator.generate_create_tables(schema)?;
        let down_sql = self.generator.generate_drop_tables(schema)?;

        Ok(Migration::new(name, up_sql, down_sql))
    }

    /// Generate a migration from a pre-computed list of [`Change`]s.
    ///
    /// Up SQL is derived by running each change through [`DiffApplier`].
    /// Down SQL reverses the change groups in dependency order, reconstructing
    /// dropped objects from the live snapshot where supported. Unsupported
    /// reversals emit comment placeholders for manual completion.
    pub fn generate_migration_from_diff(
        &self,
        name: String,
        changes: &[Change],
        schema: &SchemaIr,
        live: &LiveSchema,
    ) -> Result<Migration> {
        let provider = self.generator.provider();
        let applier = DiffApplier::new(provider, &self.generator, schema, live);

        let mut up_sql: Vec<String> = Vec::new();
        let mut down_groups: Vec<Vec<String>> = Vec::new();

        let reverser = ChangeReverser::new(provider, live);
        let ordered_changes = order_changes_for_apply(changes, live);

        for change in &ordered_changes {
            let stmts = applier.sql_for(change)?;
            up_sql.extend(stmts);
            down_groups.push(reverser.reverse(change));
        }

        let down_sql: Vec<String> = down_groups.into_iter().rev().flatten().collect();

        Ok(Migration::new(name, up_sql, down_sql))
    }

    /// Apply a migration (run "up" direction).
    ///
    /// A failure part-way through leaves the earlier phases committed, so the
    /// error is [`MigrationError::PartiallyApplied`] rather than a plain
    /// database error. Use [`Self::apply_migration_reporting`] to inspect the
    /// same run as an [`ApplyOutcome`].
    pub async fn apply_migration(&self, migration: &Migration) -> Result<()> {
        let outcome = self.apply_migration_reporting(migration).await?;
        match outcome.failure {
            None => Ok(()),
            Some(failure) => Err(MigrationError::PartiallyApplied {
                name: migration.name.clone(),
                statement: failure.statement,
                message: failure.message,
                committed: outcome.committed,
                total: migration.up_sql.len(),
            }),
        }
    }

    /// Apply a migration and report what the database kept.
    ///
    /// The statements run in the order the migration lists them, grouped into
    /// as few transactions as the provider allows: a statement that cannot run
    /// inside a transaction block commits on its own, and on MySQL DDL commits
    /// implicitly whatever the transaction says. The migration is recorded only
    /// once every statement succeeded, so a stopped run stays unrecorded while
    /// its committed statements remain in the database.
    ///
    /// `Err` is reserved for the checks that run before any statement does.
    pub async fn apply_migration_reporting(&self, migration: &Migration) -> Result<ApplyOutcome> {
        if self.tracker.is_applied(&migration.name).await? {
            return Err(MigrationError::AlreadyApplied(migration.name.clone()));
        }

        if !migration.verify_checksum() {
            return Err(MigrationError::InvalidState(
                "Migration checksum verification failed".to_string(),
            ));
        }

        let start = Instant::now();
        let outcome = self.run_phases(&migration.up_sql).await?;
        if !outcome.succeeded() {
            return Ok(outcome);
        }

        let execution_time = start.elapsed().as_millis() as i64;
        let mut tx = self.begin().await?;
        self.tracker
            .record_migration_in_tx(&mut tx, migration, execution_time)
            .await?;
        commit(tx).await?;

        Ok(outcome)
    }

    /// Rollback a migration (run "down" direction).
    ///
    /// Down statements are phased like up statements, so a failure part-way
    /// leaves the earlier phases committed and the migration still recorded.
    pub async fn rollback_migration(&self, migration: &Migration) -> Result<()> {
        if !self.tracker.is_applied(&migration.name).await? {
            return Err(MigrationError::NotFound(format!(
                "Migration '{}' is not applied",
                migration.name
            )));
        }

        let outcome = self.run_phases(&migration.down_sql).await?;
        if let Some(failure) = outcome.failure {
            return Err(MigrationError::PartiallyApplied {
                name: migration.name.clone(),
                statement: failure.statement,
                message: failure.message,
                committed: outcome.committed,
                total: migration.down_sql.len(),
            });
        }

        let mut tx = self.begin().await?;
        self.tracker
            .remove_migration_in_tx(&mut tx, &migration.name)
            .await?;
        commit(tx).await?;

        Ok(())
    }

    /// Run `statements` phase by phase, stopping at the first failure.
    async fn run_phases(&self, statements: &[String]) -> Result<ApplyOutcome> {
        let provider = self.generator.provider();
        let total = statements.len();
        let mut committed = 0;

        for phase in plan_apply_phases(statements) {
            match phase {
                ApplyPhase::Standalone(sql) => {
                    if let Err(e) = self.execute_sql(&sql).await {
                        return Ok(ApplyOutcome::stopped(
                            total,
                            committed,
                            0,
                            provider,
                            ApplyFailure {
                                statement: sql,
                                message: e.to_string(),
                            },
                        ));
                    }
                    committed += 1;
                }
                ApplyPhase::Transaction(stmts) => {
                    let mut tx = self.begin().await?;
                    for (attempted, sql) in stmts.iter().enumerate() {
                        if let Err(e) = self.execute_sql_in_tx(&mut tx, sql).await {
                            return Ok(ApplyOutcome::stopped(
                                total,
                                committed,
                                attempted,
                                provider,
                                ApplyFailure {
                                    statement: sql.clone(),
                                    message: e.to_string(),
                                },
                            ));
                        }
                    }
                    commit(tx).await?;
                    committed += stmts.len();
                }
            }
        }

        Ok(ApplyOutcome::committed_all(total))
    }

    async fn begin(&self) -> Result<sqlx::Transaction<'_, sqlx::Any>> {
        self.pool
            .begin()
            .await
            .map_err(|e| MigrationError::Database(format!("Failed to begin transaction: {}", e)))
    }

    /// Apply all pending migrations
    pub async fn apply_pending(&self, migrations: &[Migration]) -> Result<usize> {
        let mut applied_count = 0;

        for migration in migrations {
            if !self.tracker.is_applied(&migration.name).await? {
                self.apply_migration(migration).await?;
                applied_count += 1;
            }
        }

        Ok(applied_count)
    }

    /// Get the status of all migrations
    pub async fn migration_status(&self, migrations: &[Migration]) -> Result<Vec<(String, bool)>> {
        let mut status = Vec::new();

        for migration in migrations {
            let is_applied = self.tracker.is_applied(&migration.name).await?;
            status.push((migration.name.clone(), is_applied));
        }

        Ok(status)
    }

    /// Execute a SQL statement within a transaction.
    async fn execute_sql_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        sql: &str,
    ) -> Result<()> {
        if is_comment_only(sql) {
            return Ok(());
        }

        sqlx::query(sql)
            .persistent(false)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Execute a SQL statement on its own connection, outside any transaction.
    async fn execute_sql(&self, sql: &str) -> Result<()> {
        if is_comment_only(sql) {
            return Ok(());
        }

        sqlx::query(sql)
            .persistent(false)
            .execute(self.pool.as_ref())
            .await?;
        Ok(())
    }
}

/// Commit `tx`, turning a driver failure into a [`MigrationError`].
async fn commit(tx: sqlx::Transaction<'_, sqlx::Any>) -> Result<()> {
    tx.commit()
        .await
        .map_err(|e| MigrationError::Database(format!("Failed to commit transaction: {}", e)))
}

/// Whether a statement is nothing but SQL comments or whitespace.
///
/// Down-migration files carry such statements where a change cannot be
/// automatically reversed.
fn is_comment_only(sql: &str) -> bool {
    sql.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .all(|line| line.starts_with("--"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_schema::{validate_schema, Lexer, Parser};

    fn parse(source: &str) -> crate::Result<nautilus_schema::ir::SchemaIr> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next_token().map_err(crate::MigrationError::Schema)?;
            let is_eof = matches!(token.kind, nautilus_schema::TokenKind::Eof);
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        let ast = Parser::new(&tokens, source)
            .parse_schema()
            .map_err(crate::MigrationError::Schema)?;
        validate_schema(ast).map_err(crate::MigrationError::Schema)
    }

    #[tokio::test]
    async fn diff_down_sql_drops_child_table_before_parent() {
        let source = r#"
datasource db {
  provider = "postgresql"
  url      = "postgresql://localhost/test"
}

model User {
  id    Int    @id
  posts Post[]
}

model Post {
  id       Int  @id
  authorId Int
  author   User @relation(fields: [authorId], references: [id])
}
"#;
        let schema = parse(source).unwrap();
        let live = LiveSchema::default();
        let changes = crate::diff::SchemaDiff::compute(&live, &schema, DatabaseProvider::Postgres);
        sqlx::any::install_default_drivers();
        let pool = AnyPool::connect("sqlite::memory:").await.unwrap();
        let executor = MigrationExecutor::new(pool, DatabaseProvider::Postgres);

        let migration = executor
            .generate_migration_from_diff("init".to_string(), &changes, &schema, &live)
            .unwrap();

        let up_user = migration
            .up_sql
            .iter()
            .position(|s| s.contains("CREATE TABLE") && s.contains("\"User\""))
            .expect("up should create User");
        let up_post = migration
            .up_sql
            .iter()
            .position(|s| s.contains("CREATE TABLE") && s.contains("\"Post\""))
            .expect("up should create Post");
        assert!(up_user < up_post, "up must create User before Post");

        let down_post = migration
            .down_sql
            .iter()
            .position(|s| s.contains("DROP TABLE") && s.contains("\"Post\""))
            .expect("down should drop Post");
        let down_user = migration
            .down_sql
            .iter()
            .position(|s| s.contains("DROP TABLE") && s.contains("\"User\""))
            .expect("down should drop User");
        assert!(
            down_post < down_user,
            "down must drop the child Post before the parent User: {:?}",
            migration.down_sql
        );

        assert!(
            migration
                .down_sql
                .iter()
                .all(|s| s.contains("DROP TABLE") && s.contains("CASCADE")),
            "Postgres down drops should use CASCADE: {:?}",
            migration.down_sql
        );
    }

    #[tokio::test]
    #[ignore = "Requires database connection"]
    async fn test_migration_lifecycle() {
        let source = r#"
model User {
  id Int @id
  name String
}
"#;
        let schema = parse(source).unwrap();

        let pool = AnyPool::connect("sqlite::memory:").await.unwrap();
        let executor = MigrationExecutor::new(pool, DatabaseProvider::Sqlite);

        executor.init().await.unwrap();

        let migration = executor
            .generate_migration_from_schema("001_initial".to_string(), &schema)
            .unwrap();

        executor.apply_migration(&migration).await.unwrap();

        let status = executor
            .migration_status(std::slice::from_ref(&migration))
            .await
            .unwrap();
        assert_eq!(status.len(), 1);
        assert!(status[0].1);
    }
}
