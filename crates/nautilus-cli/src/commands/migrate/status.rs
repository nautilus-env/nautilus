use anyhow::Context;
use console::style;

use super::shared::MigrateContext;
use crate::tui;

/// Execute `nautilus migrate status`.
pub async fn run(
    schema: Option<String>,
    database_url: Option<String>,
    migrations_dir: Option<String>,
) -> anyhow::Result<()> {
    tui::print_header("migrate status");

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

    let migrations: Vec<_> = names
        .iter()
        .map(|name| ctx.store.load_migration(name))
        .collect::<Result<_, _>>()
        .context("Failed to load migration files")?;

    let status = ctx
        .executor
        .migration_status(&migrations)
        .await
        .context("Failed to query migration status")?;

    println!();
    println!(
        "  {:<50}  {}",
        style("Migration").bold(),
        style("Status").bold()
    );
    println!("  {}", "─".repeat(60));

    let mut pending_count = 0usize;
    let mut applied_count = 0usize;

    for (name, is_applied) in &status {
        if *is_applied {
            println!("  {:<50}  {}", name, style("Applied").green(),);
            applied_count += 1;
        } else {
            println!("  {:<50}  {}", name, style("Pending").yellow(),);
            pending_count += 1;
        }
    }

    println!();
    tui::print_summary_ok(
        &format!("{applied_count} applied, {pending_count} pending"),
        if pending_count == 0 {
            "Database is up to date"
        } else {
            "Run `nautilus migrate apply` to apply pending migrations"
        },
    );

    Ok(())
}
