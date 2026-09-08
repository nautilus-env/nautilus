//! What an apply did, told to the person who asked for it.
//!
//! The plan says which changes the database kept, which rolled back and which
//! were never attempted; this module runs it and turns that into the lines the
//! terminal shows.

use nautilus_migrate::{
    ApplyPlan, Change, ChangeRisk, DatabaseProvider, DiffApplier, GroupStatus, LiveSchema,
};

use super::database::Connection;
use crate::tui;

/// Short human-readable label for a [`Change`] (used in progress lines).
pub fn change_display_name(change: &Change) -> String {
    match change {
        Change::NewTable(m) => m.db_name.clone(),
        Change::DroppedTable { name } => name.to_string(),
        Change::AddedColumn { table, field } => format!("{}.{}", table, field.db_name),
        Change::DroppedColumn { table, column }
        | Change::TypeChanged { table, column, .. }
        | Change::NullabilityChanged { table, column, .. }
        | Change::DefaultChanged { table, column, .. }
        | Change::AutoIncrementChanged { table, column, .. }
        | Change::ComputedExprChanged { table, column, .. } => format!("{}.{}", table, column),
        Change::CheckChanged {
            table,
            column: Some(col),
            ..
        } => format!("{}.{}", table, col),
        Change::CheckChanged {
            table,
            column: None,
            ..
        } => format!("{} (CHECK)", table),
        Change::PrimaryKeyChanged { table } => format!("{} (PK)", table),
        Change::IndexAdded { table, columns, .. } | Change::IndexDropped { table, columns, .. } => {
            format!("{} ({})", table, columns.join(","))
        }
        Change::CreateCompositeType { name }
        | Change::DropCompositeType { name }
        | Change::AlterCompositeType { name, .. } => format!("type:{}", name),
        Change::CreateEnum { name, .. }
        | Change::DropEnum { name }
        | Change::AlterEnum { name, .. } => format!("enum:{}", name),
        Change::CreateExtension { name, .. } | Change::DropExtension { name } => {
            format!("ext:{}", name)
        }
        Change::CreateSchema { name } => format!("schema:{}", name),
        Change::ForeignKeyAdded { table, columns, .. } => {
            format!("{} (fk:{})", table, columns.join(","))
        }
        Change::ForeignKeyDropped {
            table,
            constraint_name,
        } => format!("{} (fk:{})", table, constraint_name),
    }
}

/// What applying a set of changes left in the database.
pub struct AppliedChanges {
    /// Changes whose statements are all committed.
    pub applied: usize,
    /// Changes that did not survive: rolled back, stopped on, or never reached.
    pub failed: usize,
    /// Whether the database kept part of a batch that then failed, so it
    /// matches neither the previous schema nor the requested one.
    pub partial: bool,
}

/// Apply a list of classified changes through the given [`DiffApplier`].
///
/// The generated SQL runs in dependency order, in as few transactions as the
/// provider allows. It is **not** one atomic unit: `ALTER TYPE ... ADD VALUE`
/// has to commit on its own, and MySQL commits implicitly around most DDL. A
/// failure therefore rolls back at most the phase it happened in, and the
/// report says which changes the database kept.
pub async fn apply_changes(
    classified: &[(Change, ChangeRisk)],
    applier: &DiffApplier<'_>,
    live: &LiveSchema,
    conn: &Connection,
    provider: DatabaseProvider,
) -> anyhow::Result<AppliedChanges> {
    let changes: Vec<Change> = classified
        .iter()
        .map(|(change, _risk)| change.clone())
        .collect();
    let plan = ApplyPlan::for_changes(provider, applier, live, &changes)
        .map_err(|e| anyhow::anyhow!("SQL generation failed: {}", e))?;

    let sp = tui::spinner("Applying…");
    let outcome = conn.apply_plan(&plan).await;
    let statuses = plan.classify_changes(&outcome);

    let Some(failure) = &outcome.failure else {
        tui::spinner_ok(sp, "All changes committed");
        for change in plan.changes() {
            tui::print_ok(&change_display_name(change.change()));
        }
        return Ok(AppliedChanges {
            applied: plan.changes().len(),
            failed: 0,
            partial: false,
        });
    };

    tui::spinner_err(sp, phase_summary(&outcome));

    let mut applied = 0;
    for (change, status) in plan.changes().iter().zip(&statuses) {
        let label = change_display_name(change.change());
        let stmts = &plan.statements()[change.statement_range()];
        match status {
            GroupStatus::Applied => {
                applied += 1;
                tui::print_ok(&label);
            }
            GroupStatus::RolledBack => tui::print_err_line(&format!("{label} (rolled back)")),
            GroupStatus::NotAttempted => tui::print_err_line(&format!("{label} (not attempted)")),
            GroupStatus::Failed { committed } => {
                tui::print_err_line(&format!("{label} ({})", failed_change_note(*committed)));
                for sql in stmts {
                    if *sql == failure.statement {
                        eprintln!("  [sql] {}   <- stopped here", sql);
                    } else {
                        eprintln!("  [sql] {}", sql);
                    }
                }
            }
        }
    }

    tui::print_table_err("Statement", &failure.message);

    Ok(AppliedChanges {
        applied,
        failed: plan.changes().len() - applied,
        partial: outcome.left_partial_state(),
    })
}

/// One line describing how much of the batch the database kept.
fn phase_summary(outcome: &nautilus_migrate::ApplyOutcome) -> &'static str {
    if outcome.left_partial_state() {
        "Stopped part-way — earlier statements are committed"
    } else {
        "Stopped — the failed transaction rolled back"
    }
}

/// How to describe the change the apply stopped on.
fn failed_change_note(committed: usize) -> String {
    if committed == 0 {
        "failed".to_string()
    } else {
        format!("failed after {committed} committed statement(s)")
    }
}
