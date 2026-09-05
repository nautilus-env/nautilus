//! How a batch of DDL is split into phases, and what an apply leaves behind.
//!
//! Not every statement can share a transaction: PostgreSQL refuses
//! `ALTER TYPE ... ADD VALUE` inside one, and MySQL commits implicitly around
//! most DDL, so a batch is a *sequence* of phases rather than one atomic unit.
//! Splitting it never reorders the statements, because a later phase routinely
//! depends on an object an earlier one created.

use crate::ddl::DatabaseProvider;
use crate::utils::requires_own_transaction;

/// One run of statements executed together, in plan order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyPhase {
    /// A statement the provider refuses to run inside a transaction block. It
    /// commits on its own and cannot be undone by a later failure.
    Standalone(String),
    /// A run of statements opened in a single transaction.
    Transaction(Vec<String>),
}

impl ApplyPhase {
    /// How many statements this phase carries.
    pub fn len(&self) -> usize {
        match self {
            Self::Standalone(_) => 1,
            Self::Transaction(stmts) => stmts.len(),
        }
    }

    /// Whether the phase carries no statement at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The statements of this phase, in order.
    pub fn statements(&self) -> &[String] {
        match self {
            Self::Standalone(sql) => std::slice::from_ref(sql),
            Self::Transaction(stmts) => stmts,
        }
    }
}

/// Split `statements` into phases without reordering them.
pub fn plan_apply_phases(statements: &[String]) -> Vec<ApplyPhase> {
    let mut phases: Vec<ApplyPhase> = Vec::new();
    for sql in statements {
        if requires_own_transaction(sql) {
            phases.push(ApplyPhase::Standalone(sql.clone()));
        } else if let Some(ApplyPhase::Transaction(run)) = phases.last_mut() {
            run.push(sql.clone());
        } else {
            phases.push(ApplyPhase::Transaction(vec![sql.clone()]));
        }
    }
    phases
}

/// The statement an apply stopped on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyFailure {
    /// The statement that returned the error.
    pub statement: String,
    /// The error the database reported.
    pub message: String,
}

/// What an apply left in the database.
///
/// The three counts partition the batch: a statement is either durable, undone
/// by the rollback of its phase, or never applied — the failing statement
/// itself counts as never applied, since its effect did not take.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Leading statements whose effect the database keeps.
    pub committed: usize,
    /// Statements that ran and were then undone.
    pub rolled_back: usize,
    /// The failing statement and everything after it.
    pub not_applied: usize,
    /// The statement the apply stopped on, when it stopped.
    pub failure: Option<ApplyFailure>,
}

impl ApplyOutcome {
    /// The outcome of a batch that ran to the end.
    pub fn committed_all(total: usize) -> Self {
        Self {
            committed: total,
            rolled_back: 0,
            not_applied: 0,
            failure: None,
        }
    }

    /// The outcome of a batch that stopped inside a phase.
    ///
    /// `committed` counts the statements of earlier phases, `attempted_in_phase`
    /// the ones this phase had already run, and `provider` decides whether the
    /// rollback can actually undo them.
    pub fn stopped(
        total: usize,
        committed: usize,
        attempted_in_phase: usize,
        provider: DatabaseProvider,
        failure: ApplyFailure,
    ) -> Self {
        let (kept, undone) = if provider.ddl_rolls_back() {
            (committed, attempted_in_phase)
        } else {
            (committed + attempted_in_phase, 0)
        };
        Self {
            committed: kept,
            rolled_back: undone,
            not_applied: total.saturating_sub(kept + undone),
            failure: Some(failure),
        }
    }

    /// Whether every statement committed.
    pub fn succeeded(&self) -> bool {
        self.failure.is_none()
    }

    /// Whether the database still carries part of a batch that failed, so the
    /// schema is neither the old one nor the new one.
    pub fn left_partial_state(&self) -> bool {
        self.failure.is_some() && self.committed > 0
    }

    /// Where each group of statements ended up, given the group sizes in apply
    /// order. Groups let a caller report per change what it sent per statement.
    pub fn classify_groups(&self, group_sizes: &[usize]) -> Vec<GroupStatus> {
        let mut start = 0;
        group_sizes
            .iter()
            .map(|size| {
                let end = start + size;
                let status = self.classify_range(start, end);
                start = end;
                status
            })
            .collect()
    }

    fn classify_range(&self, start: usize, end: usize) -> GroupStatus {
        if end <= self.committed {
            return GroupStatus::Applied;
        }
        if self.succeeded() {
            return GroupStatus::Applied;
        }
        let undone_end = self.committed + self.rolled_back;
        if start >= undone_end {
            return if start == undone_end {
                GroupStatus::Failed { committed: 0 }
            } else {
                GroupStatus::NotAttempted
            };
        }
        if end <= undone_end {
            return GroupStatus::RolledBack;
        }
        GroupStatus::Failed {
            committed: self.committed.saturating_sub(start),
        }
    }
}

/// Where one group of statements ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStatus {
    /// Every statement of the group is committed.
    Applied,
    /// The group ran inside the transaction that rolled back.
    RolledBack,
    /// The group holds the statement the apply stopped on; `committed` counts
    /// how many of its own statements the database kept.
    Failed {
        /// Statements of this group that are durable despite the failure.
        committed: usize,
    },
    /// The group was never reached.
    NotAttempted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const ADD_VALUE: &str = "ALTER TYPE \"shade\" ADD VALUE IF NOT EXISTS 'blue'";

    fn boom() -> ApplyFailure {
        ApplyFailure {
            statement: "boom".to_string(),
            message: "syntax error".to_string(),
        }
    }

    #[test]
    fn a_standalone_statement_splits_the_run_it_sits_in() {
        let phases = plan_apply_phases(&sql(&[
            "CREATE SCHEMA \"analytics\"",
            ADD_VALUE,
            "ALTER TABLE \"Box\" ALTER COLUMN \"shade\" SET DEFAULT 'blue'",
        ]));

        assert_eq!(
            phases,
            vec![
                ApplyPhase::Transaction(sql(&["CREATE SCHEMA \"analytics\""])),
                ApplyPhase::Standalone(ADD_VALUE.to_string()),
                ApplyPhase::Transaction(sql(&[
                    "ALTER TABLE \"Box\" ALTER COLUMN \"shade\" SET DEFAULT 'blue'"
                ])),
            ],
            "hoisting the standalone statement would run it before the schema exists"
        );
    }

    #[test]
    fn statements_that_share_a_transaction_stay_in_one_phase() {
        let phases = plan_apply_phases(&sql(&["CREATE TABLE a ()", "CREATE TABLE b ()"]));
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].len(), 2);
    }

    #[test]
    fn a_rollback_undoes_only_the_phase_that_failed() {
        let outcome = ApplyOutcome::stopped(5, 2, 1, DatabaseProvider::Postgres, boom());

        assert_eq!(outcome.committed, 2);
        assert_eq!(outcome.rolled_back, 1);
        assert_eq!(outcome.not_applied, 2);
        assert!(outcome.left_partial_state());
    }

    #[test]
    fn mysql_keeps_what_the_failed_transaction_already_ran() {
        let outcome = ApplyOutcome::stopped(5, 2, 1, DatabaseProvider::Mysql, boom());

        assert_eq!(outcome.committed, 3, "DDL commits implicitly on MySQL");
        assert_eq!(outcome.rolled_back, 0);
        assert_eq!(outcome.not_applied, 2);
    }

    #[test]
    fn groups_report_where_each_change_ended_up() {
        let outcome = ApplyOutcome::stopped(6, 2, 2, DatabaseProvider::Postgres, boom());

        assert_eq!(
            outcome.classify_groups(&[2, 2, 1, 1]),
            vec![
                GroupStatus::Applied,
                GroupStatus::RolledBack,
                GroupStatus::Failed { committed: 0 },
                GroupStatus::NotAttempted,
            ]
        );
    }

    #[test]
    fn a_change_straddling_the_failure_reports_what_it_left_behind() {
        let outcome = ApplyOutcome::stopped(3, 1, 0, DatabaseProvider::Postgres, boom());

        assert_eq!(
            outcome.classify_groups(&[3]),
            vec![GroupStatus::Failed { committed: 1 }],
            "one statement of the change is committed, so it is not a clean rollback"
        );
    }

    #[test]
    fn a_batch_that_ran_to_the_end_reports_every_group_applied() {
        let outcome = ApplyOutcome::committed_all(3);
        assert!(outcome.succeeded());
        assert!(!outcome.left_partial_state());
        assert_eq!(
            outcome.classify_groups(&[1, 2]),
            vec![GroupStatus::Applied, GroupStatus::Applied]
        );
    }
}
