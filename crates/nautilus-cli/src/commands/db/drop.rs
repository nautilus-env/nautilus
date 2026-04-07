use anyhow::{bail, Context};
use nautilus_migrate::{DdlGenerator, SchemaInspector};

use super::connection::DbContext;
use crate::tui;

/// Execute `nautilus db drop` — drop all tables without recreating them.
pub async fn run(
    schema_arg: Option<String>,
    db_url_arg: Option<String>,
    force: bool,
) -> anyhow::Result<()> {
    tui::print_header("db drop");

    let ctx = DbContext::build(schema_arg, db_url_arg).await?;

    if !force {
        tui::print_warning_box("This will DROP ALL TABLES permanently. ALL DATA WILL BE LOST.");
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

    let live = SchemaInspector::new(ctx.provider, &ctx.database_url)
        .inspect()
        .await
        .context("Failed to inspect live schema")?;

    let drop_stmts = generator.generate_drop_live_tables(&live);
    let table_count = live.tables.len();

    if table_count == 0 {
        tui::print_summary_ok("Nothing to drop", "No tables found in the database");
        return Ok(());
    }

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

    tui::print_summary_ok("Drop complete", &format!("{table_count} tables removed"));

    Ok(())
}
