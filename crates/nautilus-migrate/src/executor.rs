use crate::applier::DiffApplier;
use crate::ddl::{DatabaseProvider, DdlGenerator};
use crate::diff::{order_changes_for_apply, Change};
use crate::error::{MigrationError, Result};
use crate::live::{LiveIndex, LiveSchema, LiveTable};
use crate::migration::Migration;
use crate::provider::{CreateIndex, ProviderStrategy};
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
    /// Down SQL contains best-effort reversals: safe changes (new table,
    /// added column, added index) are fully reversed; destructive changes
    /// (dropped table/column, type/PK change) emit a comment placeholder.
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
    pub async fn apply_migration(&self, migration: &Migration) -> Result<()> {
        if self.tracker.is_applied(&migration.name).await? {
            return Err(MigrationError::AlreadyApplied(migration.name.clone()));
        }

        if !migration.verify_checksum() {
            return Err(MigrationError::InvalidState(
                "Migration checksum verification failed".to_string(),
            ));
        }

        let start = Instant::now();

        let mut tx =
            self.pool.begin().await.map_err(|e| {
                MigrationError::Database(format!("Failed to begin transaction: {}", e))
            })?;

        for sql in &migration.up_sql {
            self.execute_sql_in_tx(&mut tx, sql).await?;
        }

        let execution_time = start.elapsed().as_millis() as i64;

        self.tracker
            .record_migration_in_tx(&mut tx, migration, execution_time)
            .await?;

        tx.commit().await.map_err(|e| {
            MigrationError::Database(format!("Failed to commit transaction: {}", e))
        })?;

        Ok(())
    }

    /// Rollback a migration (run "down" direction)
    pub async fn rollback_migration(&self, migration: &Migration) -> Result<()> {
        if !self.tracker.is_applied(&migration.name).await? {
            return Err(MigrationError::NotFound(format!(
                "Migration '{}' is not applied",
                migration.name
            )));
        }

        let mut tx =
            self.pool.begin().await.map_err(|e| {
                MigrationError::Database(format!("Failed to begin transaction: {}", e))
            })?;

        for sql in &migration.down_sql {
            self.execute_sql_in_tx(&mut tx, sql).await?;
        }

        self.tracker
            .remove_migration_in_tx(&mut tx, &migration.name)
            .await?;

        tx.commit().await.map_err(|e| {
            MigrationError::Database(format!("Failed to commit transaction: {}", e))
        })?;

        Ok(())
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
    ///
    /// Statements that consist entirely of SQL comments (`--`) or whitespace
    /// are silently skipped — they appear in down-migration files when a change
    /// cannot be automatically reversed.
    async fn execute_sql_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        sql: &str,
    ) -> Result<()> {
        let is_comment_only = sql
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .all(|l| l.starts_with("--"));

        if is_comment_only {
            return Ok(());
        }

        sqlx::query(sql)
            .persistent(false)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}

/// Builds best-effort down-SQL for a single [`Change`].
///
/// Reversal is deliberately partial: a destructive change carries only the
/// information needed to apply it forward, so arms that cannot be recovered
/// from the diff plus the live snapshot emit a comment placeholder rather than
/// SQL that would silently lose data.
struct ChangeReverser<'a> {
    provider: DatabaseProvider,
    strategy: ProviderStrategy,
    live: &'a LiveSchema,
}

impl<'a> ChangeReverser<'a> {
    fn new(provider: DatabaseProvider, live: &'a LiveSchema) -> Self {
        Self {
            provider,
            strategy: ProviderStrategy::new(provider),
            live,
        }
    }

    fn reverse(&self, change: &Change) -> Vec<String> {
        match change {
            Change::NewTable(model) => self.reverse_new_table(&model.db_name),
            Change::DroppedTable { name } => self.reverse_dropped_table(name),
            Change::PrimaryKeyChanged { table } => self.reverse_primary_key_change(table),

            Change::AddedColumn { table, field } => {
                self.reverse_added_column(table, &field.db_name)
            }
            Change::DroppedColumn { table, column } => self.reverse_dropped_column(table, column),
            Change::TypeChanged {
                table,
                column,
                from,
                ..
            } => self.reverse_type_change(table, column, from),
            Change::NullabilityChanged {
                table,
                column,
                now_required,
            } => self.reverse_nullability_change(table, column, *now_required),
            Change::DefaultChanged {
                table,
                column,
                from,
                ..
            } => self.reverse_default_change(table, column, from.as_deref()),
            Change::AutoIncrementChanged { table, column, .. } => {
                self.reverse_auto_increment_change(table, column)
            }
            Change::ComputedExprChanged { table, column, .. } => {
                cannot_reverse(format!("computed expression change: {}.{}", table, column))
            }

            Change::IndexAdded {
                table,
                columns,
                index_name,
                ..
            } => self.reverse_index_added(table, columns, index_name.as_deref()),
            Change::IndexDropped {
                table,
                columns,
                unique,
                index_name,
            } => self.reverse_index_dropped(table, columns, *unique, index_name),

            Change::CheckChanged { table, column, .. } => {
                let target = match column {
                    Some(col) => format!("{}.{}", table, col),
                    None => table.to_string(),
                };
                cannot_reverse(format!("CHECK constraint change on {}", target))
            }
            Change::ForeignKeyAdded {
                table,
                constraint_name,
                ..
            } => self.reverse_foreign_key_added(table, constraint_name),
            Change::ForeignKeyDropped {
                table,
                constraint_name,
            } => cannot_reverse(format!(
                "DROP FOREIGN KEY {} on {}; restore manually",
                constraint_name, table
            )),

            Change::CreateCompositeType { name } | Change::CreateEnum { name, .. } => {
                self.reverse_user_type(|| vec![self.drop_type_sql(name)])
            }
            Change::DropCompositeType { name } | Change::AlterCompositeType { name, .. } => self
                .reverse_user_type(|| {
                    cannot_reverse(format!(
                        "composite type change for '{}'; restore manually",
                        name
                    ))
                }),
            Change::DropEnum { name } | Change::AlterEnum { name, .. } => {
                self.reverse_user_type(|| {
                    cannot_reverse(format!("enum type change for '{}'; restore manually", name))
                })
            }
            Change::CreateExtension { name, .. } => self.reverse_user_type(|| {
                vec![format!(
                    "DROP EXTENSION IF EXISTS \"{}\"",
                    name.replace('"', "\"\"")
                )]
            }),
            Change::DropExtension { name } => self.reverse_user_type(|| {
                cannot_reverse(format!("extension drop for '{}'; reinstall manually", name))
            }),
        }
    }

    fn quote(&self, name: &str) -> String {
        self.provider.quote_identifier(name)
    }

    /// Run `build` only on providers with user-defined types; elsewhere the
    /// forward change was itself a no-op, so its reversal must be empty.
    fn reverse_user_type(&self, build: impl FnOnce() -> Vec<String>) -> Vec<String> {
        if self.strategy.supports_user_defined_types() {
            build()
        } else {
            Vec::new()
        }
    }

    fn drop_type_sql(&self, name: &str) -> String {
        format!("DROP TYPE IF EXISTS {}", self.quote(name))
    }

    fn reverse_new_table(&self, table: &str) -> Vec<String> {
        vec![self
            .strategy
            .drop_table_sql(table, self.provider == DatabaseProvider::Postgres)]
    }

    fn reverse_dropped_table(&self, table: &str) -> Vec<String> {
        match self.live.tables.get(table) {
            Some(live_table) => create_table_sql_from_live(live_table, self.provider),
            None => missing_snapshot(format!("table {} was dropped", table)),
        }
    }

    fn reverse_primary_key_change(&self, table: &str) -> Vec<String> {
        let Some(live_table) = self.live.tables.get(table) else {
            return cannot_reverse(format!("PRIMARY KEY change on {}: no live snapshot", table));
        };
        if live_table.primary_key.is_empty() {
            return cannot_reverse(format!("PRIMARY KEY change on {}: no live PK info", table));
        }

        let pk_cols = self.quote_all(&live_table.primary_key);
        match self.provider {
            DatabaseProvider::Postgres => vec![
                format!(
                    "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}",
                    self.quote(table),
                    self.quote(&format!("{}_pkey", table))
                ),
                format!(
                    "ALTER TABLE {} ADD PRIMARY KEY ({})",
                    self.quote(table),
                    pk_cols
                ),
            ],
            DatabaseProvider::Mysql => vec![format!(
                "ALTER TABLE {} DROP PRIMARY KEY, ADD PRIMARY KEY ({})",
                self.quote(table),
                pk_cols,
            )],
            DatabaseProvider::Sqlite => cannot_reverse(format!(
                "PRIMARY KEY change on {} (SQLite requires table rebuild)",
                table
            )),
        }
    }

    fn reverse_added_column(&self, table: &str, column: &str) -> Vec<String> {
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                self.quote(table),
                self.quote(column),
            )],
            DatabaseProvider::Sqlite => {
                cannot_reverse(format!("ADD COLUMN on SQLite: {}.{}", table, column))
            }
        }
    }

    fn reverse_dropped_column(&self, table: &str, column: &str) -> Vec<String> {
        let missing = || missing_snapshot(format!("column {}.{} was dropped", table, column));
        let Some(live_column) = self
            .live
            .tables
            .get(table)
            .and_then(|live_table| live_table.columns.iter().find(|c| c.name == column))
        else {
            return missing();
        };

        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Mysql => {
                let not_null = if live_column.nullable {
                    ""
                } else {
                    " NOT NULL"
                };
                let default_clause = live_column
                    .default_value
                    .as_deref()
                    .map(|default| format!(" DEFAULT {}", default))
                    .unwrap_or_default();

                vec![format!(
                    "ALTER TABLE {} ADD COLUMN {} {}{}{}",
                    self.quote(table),
                    self.quote(column),
                    live_column.col_type.to_uppercase(),
                    not_null,
                    default_clause,
                )]
            }
            DatabaseProvider::Sqlite => {
                cannot_reverse(format!("dropped column on SQLite: {}.{}", table, column))
            }
        }
    }

    /// Restore the column's `AUTO_INCREMENT` state as the live snapshot recorded
    /// it, by restating the definition MySQL had before the change.
    fn reverse_auto_increment_change(&self, table: &str, column: &str) -> Vec<String> {
        if self.provider != DatabaseProvider::Mysql {
            return cannot_reverse(format!("AUTO_INCREMENT change: {}.{}", table, column));
        }

        let Some(live_column) = self
            .live
            .tables
            .get(table)
            .and_then(|live_table| live_table.columns.iter().find(|c| c.name == column))
        else {
            return missing_snapshot(format!("AUTO_INCREMENT change on {}.{}", table, column));
        };

        let not_null = if live_column.nullable {
            ""
        } else {
            " NOT NULL"
        };
        let auto_increment = if live_column.auto_increment {
            " AUTO_INCREMENT"
        } else {
            ""
        };

        vec![format!(
            "ALTER TABLE {} MODIFY COLUMN {} {}{}{}",
            self.quote(table),
            self.quote(column),
            live_column.col_type.to_uppercase(),
            not_null,
            auto_increment,
        )]
    }

    fn reverse_type_change(&self, table: &str, column: &str, from: &str) -> Vec<String> {
        self.strategy
            .reverse_column_type_sql(table, column, from)
            .unwrap_or_else(|| {
                cannot_reverse(format!(
                    "TYPE change on {}.{} (was {})",
                    table, column, from
                ))
            })
    }

    fn reverse_nullability_change(
        &self,
        table: &str,
        column: &str,
        now_required: bool,
    ) -> Vec<String> {
        self.strategy
            .reverse_nullability_change_sql(table, column, now_required)
            .unwrap_or_else(|| cannot_reverse(format!("nullability change: {}.{}", table, column)))
    }

    fn reverse_default_change(&self, table: &str, column: &str, from: Option<&str>) -> Vec<String> {
        self.strategy
            .reverse_default_change_sql(table, column, from)
            .unwrap_or_else(|| cannot_reverse(format!("DEFAULT change: {}.{}", table, column)))
    }

    fn reverse_index_added(
        &self,
        table: &str,
        columns: &[String],
        index_name: Option<&str>,
    ) -> Vec<String> {
        let index_name = index_name
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!("idx_{}_{}", table, columns.join("_")));
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Sqlite => {
                vec![format!("DROP INDEX IF EXISTS {}", self.quote(&index_name))]
            }
            DatabaseProvider::Mysql => vec![format!(
                "DROP INDEX {} ON {}",
                self.quote(&index_name),
                self.quote(table),
            )],
        }
    }

    fn reverse_index_dropped(
        &self,
        table: &str,
        columns: &[String],
        unique: bool,
        index_name: &str,
    ) -> Vec<String> {
        if let Some(live_index) = self
            .live
            .tables
            .get(table)
            .and_then(|t| t.indexes.iter().find(|i| i.name == *index_name))
        {
            return vec![create_index_sql_from_live(table, live_index, self.provider)];
        }

        let unique_kw = if unique { "UNIQUE " } else { "" };
        let cols_sql = self.quote_all(columns);
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Sqlite => vec![format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} ({})",
                unique_kw,
                self.quote(index_name),
                self.quote(table),
                cols_sql,
            )],
            DatabaseProvider::Mysql => vec![format!(
                "CREATE {}INDEX {} ON {} ({})",
                unique_kw,
                self.quote(index_name),
                self.quote(table),
                cols_sql,
            )],
        }
    }

    fn reverse_foreign_key_added(&self, table: &str, constraint_name: &str) -> Vec<String> {
        match self.provider {
            DatabaseProvider::Sqlite => {
                cannot_reverse(format!("ADD FOREIGN KEY on SQLite: {}", constraint_name))
            }
            DatabaseProvider::Postgres => vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}",
                self.quote(table),
                self.quote(constraint_name),
            )],
            DatabaseProvider::Mysql => vec![format!(
                "ALTER TABLE {} DROP FOREIGN KEY {}",
                self.quote(table),
                self.quote(constraint_name),
            )],
        }
    }

    fn quote_all(&self, columns: &[String]) -> String {
        columns
            .iter()
            .map(|column| self.quote(column))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn cannot_reverse(detail: String) -> Vec<String> {
    vec![format!("-- Cannot auto-reverse {}", detail)]
}

fn missing_snapshot(detail: String) -> Vec<String> {
    vec![format!(
        "-- Cannot auto-reverse: {} (no live snapshot)",
        detail
    )]
}

/// Generate a `CREATE TABLE … ` statement (plus any `CREATE INDEX` statements)
/// from a live table snapshot. Used to build down-SQL for `DroppedTable`.
fn create_table_sql_from_live(table: &LiveTable, provider: DatabaseProvider) -> Vec<String> {
    let q = |name: &str| provider.quote_identifier(name);

    // SQLite: single-column INTEGER PK -> must be inlined as
    // `col INTEGER PRIMARY KEY AUTOINCREMENT` (no separate PRIMARY KEY clause).
    let sqlite_inline_pk = provider == DatabaseProvider::Sqlite
        && table.primary_key.len() == 1
        && table
            .columns
            .iter()
            .any(|c| c.name == table.primary_key[0] && c.col_type.to_lowercase() == "integer");

    let mut col_lines: Vec<String> = Vec::new();
    for col in &table.columns {
        let is_pk = table.primary_key.contains(&col.name);
        if sqlite_inline_pk && is_pk {
            col_lines.push(format!(
                "  {} INTEGER PRIMARY KEY AUTOINCREMENT",
                q(&col.name)
            ));
        } else {
            let type_upper = col.col_type.to_uppercase();
            let mut parts = vec![q(&col.name), type_upper];
            if !col.nullable {
                parts.push("NOT NULL".to_string());
            }
            if let Some(default) = &col.default_value {
                parts.push(format!("DEFAULT {}", default));
            }
            col_lines.push(format!("  {}", parts.join(" ")));
        }
    }

    if !sqlite_inline_pk && !table.primary_key.is_empty() {
        let pk_cols = table
            .primary_key
            .iter()
            .map(|c| q(c))
            .collect::<Vec<_>>()
            .join(", ");
        col_lines.push(format!("  PRIMARY KEY ({})", pk_cols));
    }

    let mut stmts = vec![format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n)",
        q(&table.name),
        col_lines.join(",\n"),
    )];

    for idx in &table.indexes {
        stmts.push(create_index_sql_from_live(&table.name, idx, provider));
    }

    stmts
}

fn create_index_sql_from_live(
    table_name: &str,
    index: &LiveIndex,
    provider: DatabaseProvider,
) -> String {
    let kind = index.kind.to_index_kind();
    let predicate = index
        .predicate
        .as_deref()
        .map(crate::utils::schema_bool_expr_to_sql);
    ProviderStrategy::new(provider).create_index_sql(CreateIndex {
        table: table_name,
        name: &index.name,
        columns: &index.columns,
        unique: index.unique,
        kind: &kind,
        if_not_exists: true,
        predicate: predicate.as_deref(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveColumn, LiveIndex};
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

    fn live_table_with_index(table: &str, column: &str, index: LiveIndex) -> LiveTable {
        LiveTable {
            name: table.to_string(),
            columns: vec![
                LiveColumn {
                    name: "id".to_string(),
                    col_type: "integer".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                    auto_increment: false,
                },
                LiveColumn {
                    name: column.to_string(),
                    col_type: "text".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                    auto_increment: false,
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: vec![index],
            check_constraints: vec![],
            foreign_keys: vec![],
        }
    }

    #[test]
    fn dropped_table_down_sql_preserves_postgres_index_name_and_method() {
        let stmts = create_table_sql_from_live(
            &live_table_with_index(
                "User",
                "email",
                LiveIndex {
                    name: "email_hash_idx".to_string(),
                    columns: vec!["email".to_string()],
                    unique: false,
                    kind: crate::live::LiveIndexKind::Basic(
                        nautilus_schema::ir::BasicIndexType::Hash,
                    ),
                    predicate: None,
                },
            ),
            DatabaseProvider::Postgres,
        );

        assert!(
            stmts.iter().any(|sql| {
                sql.contains("CREATE INDEX IF NOT EXISTS \"email_hash_idx\"")
                    && sql.contains("ON \"User\" USING HASH (\"email\")")
            }),
            "down SQL must preserve the live physical name and USING HASH method: {:?}",
            stmts
        );
        assert!(
            !stmts.iter().any(|sql| sql.contains("idx_User_email")),
            "down SQL must not fall back to auto-generated index names: {:?}",
            stmts
        );
    }

    #[test]
    fn dropped_table_down_sql_preserves_mysql_fulltext_index_name() {
        let stmts = create_table_sql_from_live(
            &live_table_with_index(
                "Post",
                "body",
                LiveIndex {
                    name: "body_search".to_string(),
                    columns: vec!["body".to_string()],
                    unique: false,
                    kind: crate::live::LiveIndexKind::Basic(
                        nautilus_schema::ir::BasicIndexType::FullText,
                    ),
                    predicate: None,
                },
            ),
            DatabaseProvider::Mysql,
        );

        assert!(
            stmts.iter().any(|sql| {
                sql.contains("CREATE FULLTEXT INDEX `body_search`")
                    && sql.contains("ON `Post` (`body`)")
            }),
            "down SQL must preserve MySQL FULLTEXT index metadata: {:?}",
            stmts
        );
        assert!(
            !stmts.iter().any(|sql| sql.contains("idx_Post_body")),
            "down SQL must not fall back to auto-generated index names: {:?}",
            stmts
        );
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
