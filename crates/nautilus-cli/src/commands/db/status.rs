use anyhow::Context;
use nautilus_migrate::{change_risk, Change, ChangeRisk, SchemaDiff, SchemaInspector};

use super::connection::DbContext;
use crate::tui;

/// Execute `nautilus db status` — show pending changes without applying them.
pub async fn run(schema_arg: Option<String>, db_url_arg: Option<String>) -> anyhow::Result<()> {
    tui::print_header("db status");

    let ctx = DbContext::build(schema_arg, db_url_arg).await?;

    let sp = tui::spinner("Inspecting live schema…");
    let live = SchemaInspector::new(ctx.provider, &ctx.database_url)
        .inspect()
        .await
        .context("Failed to inspect live schema")?;
    tui::spinner_ok(
        sp,
        &format!(
            "Live schema read  ({} table{})",
            live.tables.len(),
            if live.tables.len() == 1 { "" } else { "s" },
        ),
    );

    let raw_changes = SchemaDiff::compute(&live, &ctx.schema_ir, ctx.provider);

    if raw_changes.is_empty() {
        tui::print_summary_ok(
            "Up to date",
            "Database is already in sync with the schema — no changes pending",
        );
        return Ok(());
    }

    let classified: Vec<(Change, ChangeRisk)> = raw_changes
        .into_iter()
        .map(|c| {
            let r = change_risk(&c);
            (c, r)
        })
        .collect();

    tui::print_diff_summary(&classified);

    let destructive = classified
        .iter()
        .filter(|(_, r)| *r == ChangeRisk::Destructive)
        .count();
    let safe = classified.len() - destructive;

    tui::print_summary_ok(
        "Pending changes",
        &format!(
            "{} change{} ({} safe, {} destructive) — run `db push` to apply",
            classified.len(),
            if classified.len() == 1 { "" } else { "s" },
            safe,
            destructive,
        ),
    );

    Ok(())
}
