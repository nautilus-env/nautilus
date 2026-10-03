//! CHECK constraints against a live database: several `@@check` on one model,
//! compound predicates and literals beyond `int4` must apply and then diff to
//! nothing; a changed check must replace exactly its own constraint, including
//! one named `chk_<table>` by earlier releases.

mod common;

use nautilus_core::TableName;
use nautilus_migrate::live::LiveSchema;
use nautilus_migrate::{
    ApplyPlan, Change, DatabaseProvider, DdlGenerator, DiffApplier, MigrationFileStore, SchemaDiff,
    SchemaInspector,
};
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
    eprintln!("skipping CHECK constraint E2E test: {reason}");
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

/// A database to run one scenario in, isolated from every other test.
struct Sandbox {
    provider: DatabaseProvider,
    /// URL the inspector and the pool use; on PostgreSQL its `search_path`
    /// points at a schema of its own.
    url: String,
    pool: AnyPool,
    cleanup: Vec<String>,
}

impl Sandbox {
    async fn open(provider: DatabaseProvider) -> Option<Self> {
        sqlx::any::install_default_drivers();
        let suffix = unique_suffix();
        match provider {
            DatabaseProvider::Postgres => {
                let base = postgres_url();
                let schema = format!("nautilus_checks_{suffix}");
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
                    cleanup: vec![format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE")],
                })
            }
            _ => {
                let url = mysql_url();
                let pool = AnyPool::connect(&url).await.ok()?;
                Some(Self {
                    provider,
                    url,
                    pool,
                    cleanup: Vec::new(),
                })
            }
        }
    }

    fn table(&mut self, prefix: &str) -> String {
        let name = format!("{prefix}_{}", unique_suffix());
        if self.provider == DatabaseProvider::Mysql {
            self.cleanup.push(format!("DROP TABLE IF EXISTS `{name}`"));
        }
        name
    }

    fn quote(&self, name: &str) -> String {
        match self.provider {
            DatabaseProvider::Mysql => format!("`{name}`"),
            _ => format!("\"{name}\""),
        }
    }

    async fn execute(&self, statements: &[String]) -> Result<(), sqlx::Error> {
        for statement in statements {
            sqlx::query(statement)
                .persistent(false)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// The live schema reduced to one table, so tables of other tests sharing
    /// the database do not show up as drops.
    async fn inspect(&self, table: &str) -> Result<LiveSchema, Box<dyn std::error::Error>> {
        let live = SchemaInspector::new(self.provider, &self.url)
            .inspect()
            .await?;
        let key = TableName::new(table);
        let mut only = LiveSchema::default();
        if let Some(t) = live.tables.get(&key) {
            only.tables.insert(key.clone(), t.clone());
        }
        if let Some(names) = live.check_constraint_names.get(&key) {
            only.check_constraint_names.insert(key, names.clone());
        }
        Ok(only)
    }

    async fn check_names(&self, table: &str) -> Result<Vec<String>, sqlx::Error> {
        let sql = match self.provider {
            DatabaseProvider::Mysql => {
                "SELECT CAST(constraint_name AS CHAR) AS name \
                 FROM information_schema.table_constraints \
                 WHERE table_schema = DATABASE() AND table_name = ? \
                   AND constraint_type = 'CHECK' ORDER BY constraint_name"
            }
            _ => {
                "SELECT conname::text AS name FROM pg_constraint \
                 WHERE conrelid = to_regclass($1) AND contype = 'c' ORDER BY conname"
            }
        };
        let rows = sqlx::query(sql)
            .bind(table)
            .persistent(false)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| row.try_get("name")).collect()
    }

    async fn close(self) {
        for statement in &self.cleanup {
            let _ = sqlx::query(statement)
                .persistent(false)
                .execute(&self.pool)
                .await;
        }
    }
}

fn provider_name(provider: DatabaseProvider) -> &'static str {
    match provider {
        DatabaseProvider::Mysql => "mysql",
        _ => "postgresql",
    }
}

fn ledger_schema(provider: DatabaseProvider, table: &str, high_limit: u32) -> String {
    format!(
        r#"
datasource db {{
  provider = "{provider}"
  url      = env("DATABASE_URL")
}}

model Ledger {{
  id     Int            @id
  low    Int            @check(low >= 1 AND low <= 10)
  high   Int
  big    BigInt         @check(big <= 4294967295)
  amount Decimal(20, 0) @check(amount >= 0 AND amount <= 18446744073709551615)

  @@check(low <= high)
  @@check(high <= {high_limit})
  @@map("{table}")
}}
"#,
        provider = provider_name(provider),
    )
}

fn is_hashed_table_check(name: &str, table: &str) -> bool {
    name.strip_prefix(&format!("chk_{table}_"))
        .is_some_and(|hash| hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit()))
}

async fn apply_changes(
    sandbox: &Sandbox,
    target: &nautilus_schema::ir::SchemaIr,
    live: &LiveSchema,
    changes: &[Change],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ddl = DdlGenerator::new(sandbox.provider);
    let applier = DiffApplier::new(sandbox.provider, &ddl, target, live);
    let mut statements = Vec::new();
    for change in changes {
        statements.extend(applier.sql_for(change)?);
    }
    sandbox.execute(&statements).await?;
    Ok(statements)
}

async fn several_model_checks_apply_and_diff_to_nothing(
    provider: DatabaseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut sandbox) = Sandbox::open(provider).await else {
        return skip_missing_prerequisite(&format!("cannot connect to {provider:?}"));
    };
    let table = sandbox.table("ledger");

    let result = async {
        let v1 = common::parse(&ledger_schema(provider, &table, 1000))?;
        let ddl = DdlGenerator::new(provider);
        sandbox.execute(&ddl.generate_create_tables(&v1)?).await?;

        let live = sandbox.inspect(&table).await?;
        let changes = SchemaDiff::compute(&live, &v1, provider);
        assert!(changes.is_empty(), "expected no change after apply, got {changes:?}");
        let names = sandbox.check_names(&table).await?;
        let hashed = names.iter().filter(|n| is_hashed_table_check(n, &table)).count();
        assert_eq!(hashed, 2, "two model-level checks expected, got {names:?}");


        let v2 = common::parse(&ledger_schema(provider, &table, 2000))?;
        let changes = SchemaDiff::compute(&live, &v2, provider);
        assert_eq!(changes.len(), 2, "expected one drop and one add, got {changes:?}");
        assert!(changes
            .iter()
            .all(|c| matches!(c, Change::CheckChanged { column: None, .. })));
        apply_changes(&sandbox, &v2, &live, &changes).await?;

        let live = sandbox.inspect(&table).await?;
        let changes = SchemaDiff::compute(&live, &v2, provider);
        assert!(changes.is_empty(), "expected no change after the update, got {changes:?}");
        let names = sandbox.check_names(&table).await?;
        let hashed = names.iter().filter(|n| is_hashed_table_check(n, &table)).count();
        assert_eq!(hashed, 2, "the replaced check must be gone, got {names:?}");


        sandbox
            .execute(&[format!(
                "INSERT INTO {} (id, low, high, big, amount) VALUES (1, 2, 1500, 4294967295, 18446744073709551615)",
                sandbox.quote(&table)
            )])
            .await?;
        let rejected = sandbox
            .execute(&[format!(
                "INSERT INTO {} (id, low, high, big, amount) VALUES (2, 2, 2500, 1, 1)",
                sandbox.quote(&table)
            )])
            .await;
        assert!(rejected.is_err(), "high = 2500 must violate high <= 2000");
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;

    sandbox.close().await;
    result
}

async fn a_check_named_by_an_older_release_is_dropped_by_its_name(
    provider: DatabaseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut sandbox) = Sandbox::open(provider).await else {
        return skip_missing_prerequisite(&format!("cannot connect to {provider:?}"));
    };
    let table = sandbox.table("legacy");

    let result = async {
        let quoted = sandbox.quote(&table);
        let legacy = sandbox.quote(&format!("chk_{table}"));
        sandbox
            .execute(&[format!(
                "CREATE TABLE {quoted} (id INTEGER PRIMARY KEY, n INTEGER NOT NULL, \
                 CONSTRAINT {legacy} CHECK (n > 0))"
            )])
            .await?;

        let target = common::parse(&format!(
            r#"
datasource db {{
  provider = "{}"
  url      = env("DATABASE_URL")
}}

model Legacy {{
  id Int @id
  n  Int

  @@check(n > 5)
  @@map("{table}")
}}
"#,
            provider_name(provider)
        ))?;
        let live = sandbox.inspect(&table).await?;
        let changes = SchemaDiff::compute(&live, &target, provider);
        let statements = apply_changes(&sandbox, &target, &live, &changes).await?;
        assert!(
            statements
                .iter()
                .any(|sql| sql.contains(&legacy) && sql.contains("DROP")),
            "the legacy constraint must be dropped by name: {statements:?}"
        );

        let live = sandbox.inspect(&table).await?;
        let changes = SchemaDiff::compute(&live, &target, provider);
        assert!(
            changes.is_empty(),
            "expected no change after the update, got {changes:?}"
        );
        let names = sandbox.check_names(&table).await?;
        assert!(!names.contains(&format!("chk_{table}")), "{names:?}");
        assert_eq!(
            names
                .iter()
                .filter(|n| is_hashed_table_check(n, &table))
                .count(),
            1,
            "{names:?}"
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;

    sandbox.close().await;
    result
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker-compose up -d` first)"]
async fn postgres_several_model_checks_apply_and_diff_to_nothing(
) -> Result<(), Box<dyn std::error::Error>> {
    several_model_checks_apply_and_diff_to_nothing(DatabaseProvider::Postgres).await
}

async fn dropped_columns_restore_checks_on_rollback(
    provider: DatabaseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut sandbox) = Sandbox::open(provider).await else {
        return skip_missing_prerequisite(&format!("cannot connect to {provider:?}"));
    };
    let table = sandbox.table("rollback_checks");
    let result = async {
        let regex_check = match provider {
            DatabaseProvider::Mysql => "regexp_like(legacy_data,'^[A-Z]*$','c')",
            _ => "legacy_data ~ '^[A-Z]*$'",
        };
        let previous = common::parse(&format!(
            r#"model Item {{
  id Int @id
  data String @map("legacy_data") @check("{regex_check}")
  kind String? @default("Alpha") @check(kind IN ["Alpha", "Beta"])
  @@map("{table}")
}}"#
        ))?;
        let ddl = DdlGenerator::new(provider);
        sandbox
            .execute(&ddl.generate_create_tables(&previous)?)
            .await?;
        let before = sandbox.inspect(&table).await?;
        let initial_drift = SchemaDiff::compute(&before, &previous, provider);
        assert!(
            initial_drift.is_empty(),
            "schema drift before rollback: {initial_drift:?}"
        );
        let before_names = sandbox.check_names(&table).await?;
        assert_eq!(before_names.len(), 2);

        let target = common::parse(&format!(
            r#"model Item {{
  id Int @id
  replacement String? @check("replacement <> 'REJECT'")
  @@map("{table}")
}}"#
        ))?;
        let changes = SchemaDiff::compute(&before, &target, provider);
        let applier = DiffApplier::new(provider, &ddl, &target, &before);
        let migration = ApplyPlan::for_changes(provider, &applier, &before, &changes)?
            .into_migration("replace_checked_columns".into());
        let dir = tempfile::tempdir()?;
        let store = MigrationFileStore::new(dir.path());
        store.write_migration(&migration.name, &migration.up_sql, &migration.down_sql)?;
        let loaded = store.load_migration(&migration.name)?;
        assert_eq!(loaded.down_sql, migration.down_sql);
        assert_eq!(
            loaded
                .down_sql
                .iter()
                .filter(|sql| sql.contains("ADD CONSTRAINT"))
                .count(),
            2,
            "both dropped column CHECKs must be restored: {:?}",
            loaded.down_sql,
        );
        sandbox.execute(&loaded.up_sql).await?;
        let forward = sandbox.inspect(&table).await?;
        assert!(SchemaDiff::compute(&forward, &target, provider).is_empty());

        sandbox.execute(&loaded.down_sql).await?;
        let restored = sandbox.inspect(&table).await?;
        let drift = SchemaDiff::compute(&restored, &previous, provider);
        assert!(drift.is_empty(), "schema drift after rollback: {drift:?}");
        assert_eq!(sandbox.check_names(&table).await?, before_names);
        let quoted = sandbox.quote(&table);
        sandbox
            .execute(&[format!(
                "INSERT INTO {quoted} (id, legacy_data) VALUES (1, 'ABC')"
            )])
            .await?;
        sandbox
            .execute(&[format!(
                "INSERT INTO {quoted} (id, legacy_data, kind) VALUES (2, 'XYZ', NULL)"
            )])
            .await?;
        assert!(
            sandbox
                .execute(&[format!(
                    "INSERT INTO {quoted} (id, legacy_data) VALUES (3, 'lower')"
                )])
                .await
                .is_err(),
            "the original case-sensitive regex must be enforced"
        );
        assert!(
            sandbox
                .execute(&[format!(
                    "INSERT INTO {quoted} (id, legacy_data, kind) VALUES (4, 'ABC', 'Gamma')"
                )])
                .await
                .is_err(),
            "the restored IN check must be enforced"
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    sandbox.close().await;
    result
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker-compose up -d` first)"]
async fn postgres_dropped_columns_restore_checks_on_rollback(
) -> Result<(), Box<dyn std::error::Error>> {
    dropped_columns_restore_checks_on_rollback(DatabaseProvider::Postgres).await
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker-compose up -d` first)"]
async fn mysql_dropped_columns_restore_checks_on_rollback() -> Result<(), Box<dyn std::error::Error>>
{
    dropped_columns_restore_checks_on_rollback(DatabaseProvider::Mysql).await
}

async fn added_columns_apply_from_crlf_and_keep_checks(
    provider: DatabaseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut sandbox) = Sandbox::open(provider).await else {
        return skip_missing_prerequisite(&format!("cannot connect to {provider:?}"));
    };
    let table = sandbox.table("added_columns");
    let result =
        async {
            let base = common::parse(&format!(r#"model Item {{ id Int @id @@map("{table}") }}"#))?;
            let ddl = DdlGenerator::new(provider);
            sandbox.execute(&ddl.generate_create_tables(&base)?).await?;
            let live = sandbox.inspect(&table).await?;
            let target = common::parse(&format!(
                r#"model Item {{
  id Int @id
  quantity Int? @map("qty") @check("qty >= 0")
  startSlot BigInt @map("start_slot")
  @@map("{table}")
}}"#
            ))?;
            let changes = SchemaDiff::compute(&live, &target, provider);
            let applier = DiffApplier::new(provider, &ddl, &target, &live);
            let mut statements = Vec::new();
            for change in &changes {
                statements.extend(applier.sql_for(change)?);
            }
            let dir = tempfile::tempdir()?;
            let store = MigrationFileStore::new(dir.path());
            store.write_migration("added_columns", &statements, &[])?;
            let path = dir.path().join("added_columns/up.sql");
            let sql = std::fs::read_to_string(&path)?;
            std::fs::write(path, sql.replace('\n', "\r\n"))?;
            let loaded = store.load_migration("added_columns")?;
            assert_eq!(loaded.up_sql, statements);
            sandbox.execute(&loaded.up_sql).await?;

            let live = sandbox.inspect(&table).await?;
            let changes = SchemaDiff::compute(&live, &target, provider);
            assert!(
                changes.is_empty(),
                "schema drift after adding columns: {changes:?}"
            );
            assert!(sandbox
                .check_names(&table)
                .await?
                .contains(&format!("chk_{table}_qty")));
            sandbox
                .execute(&[format!(
                    "INSERT INTO {} (id, qty, start_slot) VALUES (1, 0, 42)",
                    sandbox.quote(&table),
                )])
                .await?;
            assert!(
                sandbox
                    .execute(&[format!(
                        "INSERT INTO {} (id, qty, start_slot) VALUES (2, -1, 42)",
                        sandbox.quote(&table),
                    )])
                    .await
                    .is_err(),
                "a negative quantity must violate the added CHECK"
            );

            if provider == DatabaseProvider::Postgres {
                let populated_target = common::parse(&format!(
                    r#"model Item {{
  id Int @id
  quantity Int? @map("qty") @check("qty >= 0")
  startSlot BigInt @map("start_slot")
  required Int
  @@map("{table}")
}}"#
                ))?;
                let applier = DiffApplier::new(provider, &ddl, &populated_target, &live);
                let changes = SchemaDiff::compute(&live, &populated_target, provider);
                let statements = applier.sql_for(&changes[0])?;
                assert!(sandbox.execute(&statements).await.is_err(),
                "PostgreSQL must reject a required column without a default on a populated table");
                let unchanged = sandbox.inspect(&table).await?;
                assert!(SchemaDiff::compute(&unchanged, &target, provider).is_empty());
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;
    sandbox.close().await;
    result
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker-compose up -d` first)"]
async fn postgres_added_columns_apply_from_crlf_and_keep_checks(
) -> Result<(), Box<dyn std::error::Error>> {
    added_columns_apply_from_crlf_and_keep_checks(DatabaseProvider::Postgres).await
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker-compose up -d` first)"]
async fn mysql_added_columns_apply_from_crlf_and_keep_checks(
) -> Result<(), Box<dyn std::error::Error>> {
    added_columns_apply_from_crlf_and_keep_checks(DatabaseProvider::Mysql).await
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker-compose up -d` first)"]
async fn postgres_a_check_named_by_an_older_release_is_dropped_by_its_name(
) -> Result<(), Box<dyn std::error::Error>> {
    a_check_named_by_an_older_release_is_dropped_by_its_name(DatabaseProvider::Postgres).await
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker-compose up -d` first)"]
async fn mysql_several_model_checks_apply_and_diff_to_nothing(
) -> Result<(), Box<dyn std::error::Error>> {
    several_model_checks_apply_and_diff_to_nothing(DatabaseProvider::Mysql).await
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker-compose up -d` first)"]
async fn mysql_a_check_named_by_an_older_release_is_dropped_by_its_name(
) -> Result<(), Box<dyn std::error::Error>> {
    a_check_named_by_an_older_release_is_dropped_by_its_name(DatabaseProvider::Mysql).await
}
