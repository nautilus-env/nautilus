//! Real-database checks that a failure after a committed statement is reported
//! as a part-way stop, not as a full rollback.
//!
//! The two providers reach that state differently: PostgreSQL needs a phase
//! boundary, because `ALTER TYPE ... ADD VALUE` cannot share a transaction,
//! while MySQL commits implicitly around DDL so the very first statement of a
//! transaction is already durable.

use nautilus_migrate::{DatabaseProvider, Migration, MigrationExecutor};
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
    eprintln!("skipping apply-phase E2E test: {reason}");
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

async fn connect(url: &str) -> Option<AnyPool> {
    sqlx::any::install_default_drivers();
    AnyPool::connect(url).await.ok()
}

async fn drop_quietly(pool: &AnyPool, statements: &[String]) {
    for sql in statements {
        let _ = sqlx::query(sql).persistent(false).execute(pool).await;
    }
}

async fn table_exists(pool: &AnyPool, provider: DatabaseProvider, name: &str) -> bool {
    let sql = match provider {
        DatabaseProvider::Postgres => "SELECT to_regclass($1) IS NOT NULL AS present",
        _ => {
            "SELECT COUNT(*) > 0 AS present FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_name = ?"
        }
    };
    let row = sqlx::query(sql)
        .bind(name)
        .persistent(false)
        .fetch_one(pool)
        .await
        .expect("existence probe");
    row.try_get::<bool, _>("present")
        .or_else(|_| row.try_get::<i32, _>("present").map(|v| v != 0))
        .expect("existence probe returns a boolean")
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL instance (run `docker compose up -d` first)"]
async fn postgres_reports_a_committed_phase_before_the_failure(
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = connect(&postgres_url()).await else {
        return skip_missing_prerequisite("cannot connect to PostgreSQL");
    };

    let suffix = unique_suffix();
    let enum_name = format!("shade_{suffix}");
    let table = format!("box_{suffix}");
    drop_quietly(
        &pool,
        &[
            format!("DROP TABLE IF EXISTS \"{table}\""),
            format!("DROP TYPE IF EXISTS \"{enum_name}\""),
        ],
    )
    .await;

    sqlx::query(&format!("CREATE TYPE \"{enum_name}\" AS ENUM ('red')"))
        .persistent(false)
        .execute(&pool)
        .await?;

    let executor = MigrationExecutor::new(pool.clone(), DatabaseProvider::Postgres);
    executor.init().await?;

    // The ADD VALUE commits on its own; the table then fails on an unknown type.
    let migration = Migration::new(
        format!("partial_{suffix}"),
        vec![
            format!("ALTER TYPE \"{enum_name}\" ADD VALUE IF NOT EXISTS 'blue'"),
            format!("CREATE TABLE \"{table}\" (id INT PRIMARY KEY)"),
            format!("CREATE TABLE \"{table}_2\" (id no_such_type)"),
        ],
        vec![],
    );

    let outcome = executor.apply_migration_reporting(&migration).await?;

    assert!(outcome.failure.is_some(), "the third statement must fail");
    assert_eq!(outcome.committed, 1, "the ADD VALUE phase is durable");
    assert_eq!(outcome.rolled_back, 1, "the CREATE TABLE phase is undone");
    assert_eq!(outcome.not_applied, 1);
    assert!(
        outcome.left_partial_state(),
        "reporting this as a full rollback would be wrong"
    );

    let variants: i64 = sqlx::query(
        "SELECT COUNT(*) AS n FROM pg_enum e \
         JOIN pg_type t ON t.oid = e.enumtypid WHERE t.typname = $1",
    )
    .bind(&enum_name)
    .persistent(false)
    .fetch_one(&pool)
    .await?
    .try_get("n")?;
    assert_eq!(variants, 2, "the committed ADD VALUE survived the failure");

    assert!(
        !table_exists(&pool, DatabaseProvider::Postgres, &table).await,
        "the transactional phase really did roll back"
    );

    drop_quietly(
        &pool,
        &[
            format!("DROP TABLE IF EXISTS \"{table}\""),
            format!("DROP TYPE IF EXISTS \"{enum_name}\""),
        ],
    )
    .await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a running MySQL instance (run `docker compose up -d` first)"]
async fn mysql_reports_ddl_that_implicitly_committed_before_the_failure(
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = connect(&mysql_url()).await else {
        return skip_missing_prerequisite("cannot connect to MySQL");
    };

    let suffix = unique_suffix();
    let kept = format!("kept_{suffix}");
    let broken = format!("broken_{suffix}");
    drop_quietly(
        &pool,
        &[
            format!("DROP TABLE IF EXISTS `{kept}`"),
            format!("DROP TABLE IF EXISTS `{broken}`"),
        ],
    )
    .await;

    let executor = MigrationExecutor::new(pool.clone(), DatabaseProvider::Mysql);
    executor.init().await?;

    let migration = Migration::new(
        format!("partial_{suffix}"),
        vec![
            format!("CREATE TABLE `{kept}` (id INT PRIMARY KEY)"),
            format!("CREATE TABLE `{broken}` (id no_such_type)"),
        ],
        vec![],
    );

    let outcome = executor.apply_migration_reporting(&migration).await?;

    assert!(outcome.failure.is_some(), "the second statement must fail");
    assert_eq!(
        outcome.committed, 1,
        "MySQL committed the first CREATE TABLE implicitly"
    );
    assert_eq!(
        outcome.rolled_back, 0,
        "there is nothing a MySQL rollback can undo here"
    );
    assert!(outcome.left_partial_state());

    assert!(
        table_exists(&pool, DatabaseProvider::Mysql, &kept).await,
        "the first table is still there, so this was not a full rollback"
    );

    drop_quietly(
        &pool,
        &[
            format!("DROP TABLE IF EXISTS `{kept}`"),
            format!("DROP TABLE IF EXISTS `{broken}`"),
        ],
    )
    .await;
    Ok(())
}
