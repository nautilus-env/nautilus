use std::time::Instant;

use anyhow::{bail, Context};
use nautilus_migrate::{change_risk, DdlGenerator, DiffApplier, SchemaDiff, SchemaInspector};

use super::connection::{apply_changes, DbContext};
use crate::tui;

/// Execute `nautilus db reset` — drop all tables then re-push the schema.
///
/// With `--only-data` the tables are kept and only their rows are deleted
/// (TRUNCATE / DELETE). Requires `--force` (or interactive confirmation)
/// because all data will be lost.
pub async fn run(
    schema_arg: Option<String>,
    db_url_arg: Option<String>,
    force: bool,
    only_data: bool,
) -> anyhow::Result<()> {
    tui::print_header("db reset");

    let ctx = DbContext::build(schema_arg, db_url_arg).await?;

    if only_data {
        return run_truncate(ctx, force).await;
    }

    if !force {
        tui::print_warning_box(
            "This will DROP ALL TABLES and recreate them from scratch. ALL DATA WILL BE LOST.",
        );
        if !tui::confirm_destructive() {
            println!();
            tui::print_summary_err("Aborted", "No changes were applied");
            bail!("Aborted by user");
        }
    } else {
        tui::print_warning("--force passed — skipping confirmation");
    }

    let generator = DdlGenerator::new(ctx.provider);

    tui::print_section("Dropping tables");
    let start = Instant::now();

    // Inspect the live DB so we drop every table that actually exists,
    // including those that have already been removed from the schema file.
    let live_before = SchemaInspector::new(ctx.provider, &ctx.database_url)
        .inspect()
        .await
        .context("Failed to inspect live schema before drop")?;

    let drop_stmts = generator.generate_drop_live_tables(&live_before);

    let table_count = live_before.tables.len();
    let sp = tui::spinner("Dropping all tables…");
    ctx.conn
        .execute_in_transaction(&drop_stmts)
        .await
        .context("Failed to drop tables")?;
    tui::spinner_ok(
        sp,
        &format!(
            "{} table{} dropped",
            table_count,
            if table_count == 1 { "" } else { "s" },
        ),
    );

    tui::print_section("Applying schema");

    // After the drop the database is empty — re-inspect to get a clean baseline
    // for the diff engine (avoids any stale state from live_before).
    let live = SchemaInspector::new(ctx.provider, &ctx.database_url)
        .inspect()
        .await
        .context("Failed to inspect live schema after drop")?;

    let raw_changes = SchemaDiff::compute(&live, &ctx.schema_ir, ctx.provider);

    let classified: Vec<_> = raw_changes
        .into_iter()
        .map(|c| {
            let r = change_risk(&c);
            (c, r)
        })
        .collect();

    let applier = DiffApplier::new(ctx.provider, &generator, &ctx.schema_ir, &live);

    let (ok, failed) = apply_changes(&classified, &applier, &ctx.conn).await?;

    let elapsed = start.elapsed();

    if failed == 0 {
        tui::print_summary_ok(
            "Reset complete",
            &format!("{ok} applied  {:.0}ms", elapsed.as_secs_f64() * 1000.0),
        );
    } else {
        tui::print_summary_err(
            "Reset completed with errors",
            &format!(
                "{ok} ok, {failed} failed  {:.0}ms",
                elapsed.as_secs_f64() * 1000.0,
            ),
        );
        bail!("{failed} statement(s) failed during re-push");
    }

    Ok(())
}

/// Delete all rows from every table without touching the schema.
async fn run_truncate(ctx: DbContext, force: bool) -> anyhow::Result<()> {
    if !force {
        tui::print_warning_box(
            "This will DELETE ALL ROWS from every table. The schema will be kept intact. ALL DATA WILL BE LOST.",
        );
        if !tui::confirm_destructive() {
            println!();
            tui::print_summary_err("Aborted", "No changes were applied");
            bail!("Aborted by user");
        }
    } else {
        tui::print_warning("--force passed — skipping confirmation");
    }

    let generator = DdlGenerator::new(ctx.provider);

    tui::print_section("Truncating tables");
    let start = Instant::now();

    let live = SchemaInspector::new(ctx.provider, &ctx.database_url)
        .inspect()
        .await
        .context("Failed to inspect live schema before truncate")?;

    let stmts = generator.generate_truncate_live_tables(&live);

    let table_count = live.tables.len();

    if table_count == 0 {
        tui::print_summary_ok("Nothing to truncate", "No tables found in the database");
        return Ok(());
    }

    let sp = tui::spinner("Deleting all rows…");
    ctx.conn
        .execute_in_transaction(&stmts)
        .await
        .context("Failed to truncate tables")?;
    tui::spinner_ok(
        sp,
        &format!(
            "{} table{} truncated",
            table_count,
            if table_count == 1 { "" } else { "s" },
        ),
    );

    let elapsed = start.elapsed();
    tui::print_summary_ok(
        "Reset complete",
        &format!("data cleared  {:.0}ms", elapsed.as_secs_f64() * 1000.0),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::test_support::sqlite_url;
    use sqlx::query_scalar;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    use tempfile::TempDir;

    #[tokio::test]
    async fn reset_only_data_truncates_live_tables_not_just_schema_models() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("reset.db");
        let schema_path = temp_dir.path().join("schema.nautilus");
        let database_url = sqlite_url(&db_path);

        let schema = format!(
            r#"datasource db {{
  provider = "sqlite"
  url      = "{database_url}"
}}

model User {{
  id Int @id
}}
"#
        );
        std::fs::write(&schema_path, schema).expect("failed to write schema");

        let pool = sqlx::SqlitePool::connect_with(
            SqliteConnectOptions::from_str(&database_url)
                .expect("valid sqlite url")
                .create_if_missing(true),
        )
        .await
        .expect("failed to connect to sqlite db");

        sqlx::raw_sql(
            r#"CREATE TABLE "user" (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT
);
CREATE TABLE "legacy" (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  note TEXT
);
INSERT INTO "user"(name) VALUES ('alice');
INSERT INTO "legacy"(note) VALUES ('leftover');
"#,
        )
        .execute(&pool)
        .await
        .expect("failed to seed live tables");

        run(
            Some(schema_path.to_string_lossy().to_string()),
            Some(database_url.clone()),
            true,
            true,
        )
        .await
        .expect("reset --only-data should succeed");

        let user_count: i64 = query_scalar(r#"SELECT COUNT(*) FROM "user""#)
            .fetch_one(&pool)
            .await
            .expect("failed to count user rows");
        let legacy_count: i64 = query_scalar(r#"SELECT COUNT(*) FROM "legacy""#)
            .fetch_one(&pool)
            .await
            .expect("failed to count legacy rows");

        assert_eq!(user_count, 0);
        assert_eq!(legacy_count, 0);
    }
}
