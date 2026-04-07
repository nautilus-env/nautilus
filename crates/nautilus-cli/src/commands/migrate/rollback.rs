use anyhow::Context;

use super::shared::MigrateContext;
use crate::tui;

/// Execute `nautilus migrate rollback`.
pub async fn run(
    steps: usize,
    schema: Option<String>,
    database_url: Option<String>,
    migrations_dir: Option<String>,
) -> anyhow::Result<()> {
    tui::print_header("migrate rollback");

    let sp = tui::spinner("Loading schema and connecting…");
    let ctx = MigrateContext::build(schema, database_url, migrations_dir).await?;
    tui::spinner_ok(sp, "Connected");

    let all_names = ctx
        .store
        .list_migration_names()
        .context("Failed to list migration files")?;

    if all_names.is_empty() {
        tui::print_summary_ok("No migration files", "Nothing to roll back");
        return Ok(());
    }

    let migrations: Vec<_> = all_names
        .iter()
        .map(|name| ctx.store.load_migration(name))
        .collect::<Result<_, _>>()
        .context("Failed to load migration files")?;

    let status = ctx
        .executor
        .migration_status(&migrations)
        .await
        .context("Failed to query migration status")?;

    let applied: Vec<_> = status
        .iter()
        .zip(migrations.iter())
        .filter(|((_, is_applied), _)| *is_applied)
        .map(|(_, migration)| migration)
        .collect();

    if applied.is_empty() {
        tui::print_summary_ok(
            "Nothing to roll back",
            "No migrations are currently applied",
        );
        return Ok(());
    }

    let to_rollback: Vec<_> = applied.into_iter().rev().take(steps).collect();

    let mut rolled_back = 0usize;

    for migration in to_rollback {
        ctx.executor
            .rollback_migration(migration)
            .await
            .with_context(|| format!("Failed to roll back migration {}", migration.name))?;
        println!("  ✔  Rolled back  {}", migration.name);
        rolled_back += 1;
    }

    println!();
    tui::print_summary_ok(&format!("{rolled_back} rolled back"), "Rollback complete");

    Ok(())
}
