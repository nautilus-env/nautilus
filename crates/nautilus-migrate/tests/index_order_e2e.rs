//! Compound indexes against a live database: an index added to an existing
//! table keeps its declared column order, `db status` tells `(a, b)` from
//! `(b, a)`, and a reordered declaration is recreated under the same name and
//! restored in its previous order by the rollback. A compound primary key
//! whose live order differs from its `@@id` is rebuilt in the declared order,
//! keeping its rows, and the rollback restores the live order.

mod common;

use nautilus_core::TableName;
use nautilus_migrate::live::LiveSchema;
use nautilus_migrate::{
    ApplyPlan, Change, DatabaseProvider, DdlGenerator, DiffApplier, Migration, MigrationFileStore,
    SchemaDiff, SchemaInspector,
};
use nautilus_schema::ir::SchemaIr;
use sqlx::{AnyPool, Row};

fn postgres_url() -> String {
    std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://nautilus:nautilus@localhost/nautilus_test".to_string())
}

fn mysql_url() -> String {
    std::env::var("NAUTILUS_TEST_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://nautilus:nautilus@localhost:3306/nautilus_test".to_string())
}

fn skip_missing_prerequisite(reason: &str) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("NAUTILUS_REQUIRE_E2E").is_some() {
        return Err(format!("required E2E prerequisite missing: {reason}").into());
    }
    eprintln!("skipping index order E2E test: {reason}");
    Ok(())
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

/// One table to run the scenario on, isolated from every other test.
struct Sandbox {
    provider: DatabaseProvider,
    /// URL the inspector and the pool use; on PostgreSQL its `search_path`
    /// points at a schema of its own.
    url: String,
    pool: AnyPool,
    table: String,
    cleanup: Vec<String>,
    /// Holds the SQLite database file until the sandbox closes.
    dir: Option<tempfile::TempDir>,
}

impl Sandbox {
    async fn open(provider: DatabaseProvider, table_prefix: &str) -> Option<Self> {
        sqlx::any::install_default_drivers();
        let suffix = unique_suffix();
        let table = format!("{table_prefix}_{suffix}");
        match provider {
            DatabaseProvider::Sqlite => {
                let dir = tempfile::tempdir().ok()?;
                let url = format!(
                    "sqlite:{}?mode=rwc",
                    dir.path()
                        .join("index_order.db")
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/")
                );
                let pool = AnyPool::connect(&url).await.ok()?;
                Some(Self {
                    provider,
                    url,
                    pool,
                    table,
                    cleanup: Vec::new(),
                    dir: Some(dir),
                })
            }
            DatabaseProvider::Postgres => {
                let base = postgres_url();
                let schema = format!("nautilus_index_order_{suffix}");
                let admin = AnyPool::connect(&base).await.ok()?;
                sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
                    .persistent(false)
                    .execute(&admin)
                    .await
                    .ok()?;
                admin.close().await;
                let sep = if base.contains('?') { '&' } else { '?' };
                let url = format!("{base}{sep}options=-c%20search_path%3D{schema}");
                let pool = AnyPool::connect(&url).await.ok()?;
                Some(Self {
                    provider,
                    url,
                    pool,
                    table,
                    cleanup: vec![format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE")],
                    dir: None,
                })
            }
            _ => {
                let url = mysql_url();
                let pool = AnyPool::connect(&url).await.ok()?;
                let cleanup = vec![format!("DROP TABLE IF EXISTS `{table}`")];
                Some(Self {
                    provider,
                    url,
                    pool,
                    table,
                    cleanup,
                    dir: None,
                })
            }
        }
    }

    /// Run `statements` on one connection, as the executor runs a migration
    /// phase in one transaction. Across pooled connections, a SQLite table
    /// rebuild fails: the connection running the final `RENAME` may not have
    /// seen the `DROP TABLE` and still holds the old table under that name.
    async fn execute(&self, statements: &[String]) -> Result<(), sqlx::Error> {
        let mut conn = self.pool.acquire().await?;
        for statement in statements {
            sqlx::query(statement)
                .persistent(false)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    async fn count_rows(&self) -> Result<i64, sqlx::Error> {
        sqlx::query(&format!("SELECT COUNT(*) FROM {}", self.table))
            .persistent(false)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)
    }

    /// The live schema reduced to the sandbox table, so tables of other tests
    /// sharing the database do not show up as drops.
    async fn inspect(&self) -> Result<LiveSchema, Box<dyn std::error::Error>> {
        let live = SchemaInspector::new(self.provider, &self.url)
            .inspect()
            .await?;
        let key = TableName::new(self.table.as_str());
        let mut only = LiveSchema::default();
        if let Some(t) = live.tables.get(&key) {
            only.tables.insert(key, t.clone());
        }
        Ok(only)
    }

    /// Generate the migration for `changes` as `migrate generate` does, write
    /// and reload its files, and apply its `up.sql`.
    async fn migrate(
        &self,
        name: &str,
        target: &SchemaIr,
        live: &LiveSchema,
        changes: &[Change],
    ) -> Result<Migration, Box<dyn std::error::Error>> {
        let ddl = DdlGenerator::new(self.provider);
        let applier = DiffApplier::new(self.provider, &ddl, target, live);
        let migration = ApplyPlan::for_changes(self.provider, &applier, live, changes)?
            .into_migration(name.to_string());
        let dir = tempfile::tempdir()?;
        let store = MigrationFileStore::new(dir.path());
        store.write_migration(&migration.name, &migration.up_sql, &migration.down_sql)?;
        let loaded = store.load_migration(&migration.name)?;
        self.execute(&loaded.up_sql).await?;
        Ok(loaded)
    }

    async fn close(self) {
        for statement in &self.cleanup {
            let _ = sqlx::query(statement)
                .persistent(false)
                .execute(&self.pool)
                .await;
        }
        self.pool.close().await;
        drop(self.dir);
    }
}

fn message_schema(table: &str, index: &str) -> Result<SchemaIr, Box<dyn std::error::Error>> {
    Ok(common::parse(&format!(
        r#"model Message {{
  id        Int @id
  threadId Int @map("thread_id")
  {index}
  @@map("{table}")
}}"#
    ))?)
}

/// The key columns of the index named `name`, in the order the database
/// reports them.
fn index_columns(live: &LiveSchema, table: &str, name: &str) -> Vec<String> {
    live.tables
        .get(&TableName::new(table))
        .and_then(|t| t.indexes.iter().find(|i| i.name == name))
        .map(|i| i.columns.clone())
        .unwrap_or_default()
}

async fn compound_index_keeps_its_declared_order(
    provider: DatabaseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(sandbox) = Sandbox::open(provider, "messages").await else {
        if provider == DatabaseProvider::Sqlite {
            return Err("cannot open a SQLite file database".into());
        }
        return skip_missing_prerequisite(&format!("cannot connect to {provider:?}"));
    };

    let result = async {
        let table = sandbox.table.as_str();
        let name = format!("idx_{table}_id_thread_id");
        let base = message_schema(table, "")?;
        let thread_first = message_schema(table, "@@index([threadId, id])")?;
        let id_first = message_schema(table, "@@index([id, threadId])")?;

        let ddl = DdlGenerator::new(provider);
        sandbox.execute(&ddl.generate_create_tables(&base)?).await?;
        let live = sandbox.inspect().await?;
        let changes = SchemaDiff::compute(&live, &thread_first, provider);
        assert!(
            matches!(
                changes.as_slice(),
                [Change::IndexAdded { columns, index_name: Some(n), .. }]
                    if columns == &["thread_id", "id"] && *n == name
            ),
            "expected one index in declared order named after its sorted columns, got {changes:?}"
        );
        sandbox
            .migrate("add_index", &thread_first, &live, &changes)
            .await?;

        let added = sandbox.inspect().await?;
        assert_eq!(index_columns(&added, table, &name), ["thread_id", "id"]);
        let drift = SchemaDiff::compute(&added, &thread_first, provider);
        assert!(
            drift.is_empty(),
            "expected no change after apply, got {drift:?}"
        );

        let changes = SchemaDiff::compute(&added, &id_first, provider);
        assert_eq!(
            changes.len(),
            2,
            "a reordered index must be dropped and added again, got {changes:?}"
        );
        let reorder = sandbox
            .migrate("reorder_index", &id_first, &added, &changes)
            .await?;

        let reordered = sandbox.inspect().await?;
        assert_eq!(index_columns(&reordered, table, &name), ["id", "thread_id"]);
        let drift = SchemaDiff::compute(&reordered, &id_first, provider);
        assert!(
            drift.is_empty(),
            "expected no change after the reorder, got {drift:?}"
        );

        sandbox.execute(&reorder.down_sql).await?;
        let restored = sandbox.inspect().await?;
        assert_eq!(index_columns(&restored, table, &name), ["thread_id", "id"]);
        let drift = SchemaDiff::compute(&restored, &thread_first, provider);
        assert!(drift.is_empty(), "schema drift after rollback: {drift:?}");
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;

    sandbox.close().await;
    result
}

#[tokio::test]
async fn sqlite_compound_index_keeps_its_declared_order() -> Result<(), Box<dyn std::error::Error>>
{
    compound_index_keeps_its_declared_order(DatabaseProvider::Sqlite).await
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker-compose up -d` first)"]
async fn postgres_compound_index_keeps_its_declared_order() -> Result<(), Box<dyn std::error::Error>>
{
    compound_index_keeps_its_declared_order(DatabaseProvider::Postgres).await
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker-compose up -d` first)"]
async fn mysql_compound_index_keeps_its_declared_order() -> Result<(), Box<dyn std::error::Error>> {
    compound_index_keeps_its_declared_order(DatabaseProvider::Mysql).await
}

fn order_line_schema(table: &str, id: &str) -> Result<SchemaIr, Box<dyn std::error::Error>> {
    Ok(common::parse(&format!(
        r#"model OrderLine {{
  orderId Int @map("order_id")
  lineNo  Int @map("line_no")
  {id}
  @@map("{table}")
}}"#
    ))?)
}

/// The primary-key columns of `table`, in the order the database reports them.
fn primary_key(live: &LiveSchema, table: &str) -> Vec<String> {
    live.tables
        .get(&TableName::new(table))
        .map(|t| t.primary_key.clone())
        .unwrap_or_default()
}

async fn compound_primary_key_keeps_its_declared_order(
    provider: DatabaseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(sandbox) = Sandbox::open(provider, "order_lines").await else {
        if provider == DatabaseProvider::Sqlite {
            return Err("cannot open a SQLite file database".into());
        }
        return skip_missing_prerequisite(&format!("cannot connect to {provider:?}"));
    };

    let result = async {
        let table = sandbox.table.as_str();
        let line_first = order_line_schema(table, "@@id([lineNo, orderId])")?;
        let order_first = order_line_schema(table, "@@id([orderId, lineNo])")?;

        let ddl = DdlGenerator::new(provider);
        sandbox
            .execute(&ddl.generate_create_tables(&line_first)?)
            .await?;
        sandbox
            .execute(&[format!(
                "INSERT INTO {table} (order_id, line_no) VALUES (1, 1), (1, 2), (2, 1)"
            )])
            .await?;
        let live = sandbox.inspect().await?;
        assert_eq!(primary_key(&live, table), ["line_no", "order_id"]);

        let changes = SchemaDiff::compute(&live, &order_first, provider);
        assert!(
            matches!(changes.as_slice(), [Change::PrimaryKeyChanged { .. }]),
            "a key on the same columns in another order must change, got {changes:?}"
        );
        let reorder = sandbox
            .migrate("reorder_primary_key", &order_first, &live, &changes)
            .await?;

        let reordered = sandbox.inspect().await?;
        assert_eq!(primary_key(&reordered, table), ["order_id", "line_no"]);
        assert_eq!(sandbox.count_rows().await?, 3);
        let drift = SchemaDiff::compute(&reordered, &order_first, provider);
        assert!(
            drift.is_empty(),
            "expected no change after the reorder, got {drift:?}"
        );

        if provider == DatabaseProvider::Sqlite {
            // SQLite changes a key only by rebuilding the table, which the
            // rollback leaves to a hand-written migration.
            assert!(
                reorder
                    .down_sql
                    .iter()
                    .any(|s| s.starts_with("-- Cannot auto-reverse PRIMARY KEY change")),
                "expected a manual rollback, got {:?}",
                reorder.down_sql
            );
            return Ok(());
        }

        sandbox.execute(&reorder.down_sql).await?;
        let restored = sandbox.inspect().await?;
        assert_eq!(primary_key(&restored, table), ["line_no", "order_id"]);
        assert_eq!(sandbox.count_rows().await?, 3);
        let drift = SchemaDiff::compute(&restored, &line_first, provider);
        assert!(drift.is_empty(), "schema drift after rollback: {drift:?}");
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;

    sandbox.close().await;
    result
}

#[tokio::test]
async fn sqlite_compound_primary_key_keeps_its_declared_order(
) -> Result<(), Box<dyn std::error::Error>> {
    compound_primary_key_keeps_its_declared_order(DatabaseProvider::Sqlite).await
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker-compose up -d` first)"]
async fn postgres_compound_primary_key_keeps_its_declared_order(
) -> Result<(), Box<dyn std::error::Error>> {
    compound_primary_key_keeps_its_declared_order(DatabaseProvider::Postgres).await
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker-compose up -d` first)"]
async fn mysql_compound_primary_key_keeps_its_declared_order(
) -> Result<(), Box<dyn std::error::Error>> {
    compound_primary_key_keeps_its_declared_order(DatabaseProvider::Mysql).await
}
