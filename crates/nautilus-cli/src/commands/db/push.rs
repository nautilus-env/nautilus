use std::time::Instant;

use anyhow::{bail, Context};
use nautilus_migrate::{
    change_risk, Change, ChangeRisk, DdlGenerator, DiffApplier, SchemaDiff, SchemaInspector,
};

use super::connection::{apply_changes, DbContext};
use crate::{commands::generate::run_generate, tui};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PostGenerateContext {
    AlreadyInSync,
    AppliedChanges,
}

/// Execute `nautilus db push` — compute the diff between the local schema and
/// the live database, then apply every change (with confirmation for
/// destructive operations).
pub async fn run(
    schema_arg: Option<String>,
    db_url_arg: Option<String>,
    accept_data_loss: bool,
    no_generate: bool,
) -> anyhow::Result<()> {
    tui::print_header("db push");

    let schema_arg_for_generate = schema_arg.clone();
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
            "No changes detected",
            "Database is already in sync with the schema",
        );
        return maybe_run_post_generate(
            PostGenerateContext::AlreadyInSync,
            schema_arg_for_generate,
            no_generate,
        )
        .await;
    }

    let classified: Vec<(Change, ChangeRisk)> = raw_changes
        .into_iter()
        .map(|c| {
            let r = change_risk(&c);
            (c, r)
        })
        .collect();

    tui::print_diff_summary(&classified);

    let has_destructive = classified
        .iter()
        .any(|(_, r)| *r == ChangeRisk::Destructive);

    if has_destructive {
        if accept_data_loss {
            tui::print_warning("--accept-data-loss passed — skipping confirmation");
        } else {
            tui::print_warning_box(
                "Destructive changes detected. Some operations will cause data loss.",
            );
            if !tui::confirm_destructive() {
                println!();
                tui::print_summary_err("Aborted", "No changes were applied");
                bail!("Aborted by user");
            }
        }
    }

    tui::print_section("Applying changes");

    let start = Instant::now();
    let generator = DdlGenerator::new(ctx.provider);
    let applier = DiffApplier::new(ctx.provider, &generator, &ctx.schema_ir, &live);

    let (ok, failed) = apply_changes(&classified, &applier, &ctx.conn).await?;

    let elapsed = start.elapsed();

    if failed == 0 {
        tui::print_summary_ok(
            "Done",
            &format!("{ok} applied  {:.0}ms", elapsed.as_secs_f64() * 1000.0),
        );
        maybe_run_post_generate(
            PostGenerateContext::AppliedChanges,
            schema_arg_for_generate,
            no_generate,
        )
        .await
    } else {
        tui::print_summary_err(
            "Completed with errors",
            &format!(
                "{ok} ok, {failed} failed  {:.0}ms",
                elapsed.as_secs_f64() * 1000.0,
            ),
        );
        bail!("{failed} statement(s) failed");
    }
}

async fn maybe_run_post_generate(
    context: PostGenerateContext,
    schema_arg: Option<String>,
    no_generate: bool,
) -> anyhow::Result<()> {
    if no_generate {
        tui::print_section("Generating client code");
        tui::print_warning("--no-generate passed — skipping client generation");
        return Ok(());
    }

    finalize_post_generate(context, run_post_generate(schema_arg).await)
}

async fn run_post_generate(schema_arg: Option<String>) -> anyhow::Result<()> {
    tui::print_section("Generating client code");
    tokio::task::spawn_blocking(move || run_generate(schema_arg, false, false, false))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("Generate task error: {}", e)))
}

fn finalize_post_generate(
    context: PostGenerateContext,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    result.with_context(|| match context {
        PostGenerateContext::AlreadyInSync => {
            "Database is already in sync, but client generation failed"
        }
        PostGenerateContext::AppliedChanges => {
            "Database changes were applied successfully, but client generation failed"
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{finalize_post_generate, maybe_run_post_generate, PostGenerateContext};
    use crate::test_support::{lock_working_dir_async, sqlite_url, CurrentDirGuard};
    use sqlx::query_scalar;
    use tempfile::TempDir;

    #[test]
    fn finalize_post_generate_keeps_success() {
        assert!(finalize_post_generate(PostGenerateContext::AppliedChanges, Ok(())).is_ok());
    }

    #[test]
    fn finalize_post_generate_wraps_error_after_noop_sync() {
        let err = finalize_post_generate(
            PostGenerateContext::AlreadyInSync,
            Err(anyhow::anyhow!("install step failed")),
        )
        .expect_err("expected wrapped error");

        assert_eq!(
            err.to_string(),
            "Database is already in sync, but client generation failed"
        );
        assert_eq!(err.root_cause().to_string(), "install step failed",);
    }

    #[test]
    fn finalize_post_generate_wraps_error_after_applied_sync() {
        let err = finalize_post_generate(
            PostGenerateContext::AppliedChanges,
            Err(anyhow::anyhow!("python package install failed")),
        )
        .expect_err("expected wrapped error");

        assert_eq!(
            err.to_string(),
            "Database changes were applied successfully, but client generation failed"
        );
        assert_eq!(
            err.root_cause().to_string(),
            "python package install failed",
        );
    }

    #[tokio::test]
    async fn maybe_run_post_generate_skips_generation_when_disabled() {
        let _cwd_lock = lock_working_dir_async().await;
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());
        let db_path = temp_dir.path().join("push.db");
        let schema_path = temp_dir.path().join("schema.nautilus");
        let output_path = temp_dir.path().join("generated");
        let database_url = sqlite_url(&db_path);

        std::fs::write(&output_path, "occupied").expect("failed to create occupied output path");
        std::fs::write(
            &schema_path,
            format!(
                r#"datasource db {{
  provider = "sqlite"
  url      = "{database_url}"
}}

model User {{
  id Int @id @default(autoincrement())
}}

generator client {{
  provider = "nautilus-client-rs"
  output   = "./generated"
  interface = "async"
}}
"#
            ),
        )
        .expect("failed to write schema");

        super::run(
            Some(schema_path.to_string_lossy().to_string()),
            Some(database_url.clone()),
            false,
            true,
        )
        .await
        .expect("db push should succeed when generation is skipped");

        let pool = sqlx::SqlitePool::connect(&database_url)
            .await
            .expect("failed to connect to sqlite db");
        let table_exists: i64 = query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("failed to inspect sqlite tables");

        assert_eq!(table_exists, 1);
    }

    #[tokio::test]
    async fn maybe_run_post_generate_reports_generation_errors_when_enabled() {
        let _cwd_lock = lock_working_dir_async().await;
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());
        let schema_path = temp_dir.path().join("schema.nautilus");
        let output_path = temp_dir.path().join("generated");

        std::fs::write(&output_path, "occupied").expect("failed to create occupied output path");
        std::fs::write(
            &schema_path,
            r#"datasource db {
  provider = "sqlite"
  url      = "sqlite:ignored.db"
}

model User {
  id Int @id @default(autoincrement())
}

generator client {
  provider  = "nautilus-client-rs"
  output    = "./generated"
  interface = "async"
}
"#,
        )
        .expect("failed to write schema");

        let err = maybe_run_post_generate(
            PostGenerateContext::AppliedChanges,
            Some(schema_path.to_string_lossy().to_string()),
            false,
        )
        .await
        .expect_err("generation should fail when output path is occupied by a file");

        assert_eq!(
            err.to_string(),
            "Database changes were applied successfully, but client generation failed"
        );
        let error_chain = format!("{err:#}");
        assert!(
            error_chain.contains("Failed to clean output directory"),
            "unexpected error chain: {error_chain}"
        );
    }
}
