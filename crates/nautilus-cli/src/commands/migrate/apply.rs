use anyhow::Context;

use super::shared::MigrateContext;
use crate::tui;

/// Execute `nautilus migrate apply`.
pub async fn run(
    schema: Option<String>,
    database_url: Option<String>,
    migrations_dir: Option<String>,
) -> anyhow::Result<()> {
    tui::print_header("migrate apply");

    let sp = tui::spinner("Loading schema and connecting…");
    let ctx = MigrateContext::build(schema, database_url, migrations_dir).await?;
    tui::spinner_ok(sp, "Connected");

    let names = ctx
        .store
        .list_migration_names()
        .context("Failed to list migration files")?;

    if names.is_empty() {
        tui::print_summary_ok(
            "No migration files found",
            &format!(
                "Run `nautilus migrate generate` to create one in {}",
                ctx.store.dir().display()
            ),
        );
        return Ok(());
    }

    let mut applied = 0usize;
    let mut skipped = 0usize;

    for name in &names {
        let migration = ctx
            .store
            .load_migration(name)
            .with_context(|| format!("Failed to load migration {name}"))?;

        match ctx.executor.apply_migration(&migration).await {
            Ok(()) => {
                println!("  ✔  Applied   {name}");
                applied += 1;
            }
            Err(nautilus_migrate::MigrationError::AlreadyApplied(_)) => {
                println!("  –  Skipped   {name}  (already applied)");
                skipped += 1;
            }
            Err(e) => {
                tui::print_table_err(name, &e.to_string());
                return Err(e).with_context(|| format!("Migration {name} failed"));
            }
        }
    }

    println!();
    tui::print_summary_ok(
        &format!("{applied} applied, {skipped} skipped"),
        "Migration complete",
    );

    Ok(())
}
