use anyhow::Context;
use chrono::Utc;
use nautilus_migrate::{SchemaDiff, SchemaInspector};

use super::shared::MigrateContext;
use crate::tui;

/// Execute `nautilus migrate generate`.
pub async fn run(
    label: Option<String>,
    schema: Option<String>,
    database_url: Option<String>,
    migrations_dir: Option<String>,
) -> anyhow::Result<()> {
    tui::print_header("migrate generate");

    let sp = tui::spinner("Loading schema and connecting…");
    let ctx = MigrateContext::build(schema, database_url, migrations_dir).await?;
    tui::spinner_ok(sp, "Connected");

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

    let changes = SchemaDiff::compute(&live, &ctx.schema_ir, ctx.provider);

    if changes.is_empty() {
        tui::print_summary_ok(
            "No changes detected",
            "Database is already in sync with the schema — nothing to generate",
        );
        return Ok(());
    }

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let label_part = sanitize_migration_label(label.as_deref());
    let migration_name = format!("{timestamp}_{label_part}");

    let sp = tui::spinner("Generating migration…");
    let migration = ctx
        .executor
        .generate_migration_from_diff(migration_name.clone(), &changes, &ctx.schema_ir, &live)
        .context("Failed to generate migration")?;
    tui::spinner_ok(
        sp,
        &format!(
            "Migration prepared  ({} up, {} down)",
            migration.up_sql.len(),
            migration.down_sql.len()
        ),
    );

    ctx.store
        .write_migration(&migration_name, &migration.up_sql, &migration.down_sql)
        .context("Failed to write migration files")?;

    let dir = ctx.store.dir().display().to_string();
    println!();
    println!("  Wrote:");
    println!("    {dir}/{migration_name}.up.sql");
    println!("    {dir}/{migration_name}.down.sql");
    println!();
    tui::print_summary_ok(
        "Migration generated",
        "Run `nautilus migrate apply` to apply it",
    );

    Ok(())
}

fn sanitize_migration_label(label: Option<&str>) -> String {
    let raw = label.unwrap_or("auto");
    let mut sanitized = String::new();
    let mut last_was_separator = false;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('_');
            last_was_separator = true;
        }
    }

    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "auto".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_migration_label;

    #[test]
    fn sanitize_migration_label_normalizes_for_portable_directory_names() {
        assert_eq!(sanitize_migration_label(Some("Add Users")), "add_users");
        assert_eq!(
            sanitize_migration_label(Some(r"..\weird/name:*? migration")),
            "weird_name_migration"
        );
        assert_eq!(sanitize_migration_label(Some("___")), "auto");
        assert_eq!(sanitize_migration_label(None), "auto");
    }
}
