//! Execution metadata for generated changes and the legacy SQL-file boundary.

use std::ops::Range;

use crate::apply::{ApplyFailure, ApplyOutcome, GroupStatus};
use crate::ddl::DatabaseProvider;
use crate::diff::Change;
use crate::migration::Migration;

mod legacy;
mod reversal;

pub use legacy::{plan_apply_phases, requires_own_transaction};
pub use reversal::ReversalPlan;

/// Whether statements can share a transaction with their neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionRequirement {
    /// Keep the ordered run in one transaction, including SQLite rebuilds.
    Shared,
    /// Commit each statement before starting the next one.
    Standalone,
}

/// Whether a failed phase can undo the statements it already executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackBehavior {
    /// The transaction can roll back its DDL.
    Transactional,
    /// Earlier statements remain durable, including implicitly committed DDL.
    KeepsStatements,
}

impl RollbackBehavior {
    pub(crate) fn for_provider(provider: DatabaseProvider) -> Self {
        if provider.ddl_rolls_back() {
            Self::Transactional
        } else {
            Self::KeepsStatements
        }
    }
}

/// SQL and execution requirements produced from one schema change.
#[derive(Debug, Clone)]
pub struct ChangeSqlPlan {
    pub(crate) change: Change,
    pub(crate) statements: Vec<String>,
    pub(crate) transaction: TransactionRequirement,
    pub(crate) reversal: ReversalPlan,
}

/// The source change and its statement range in an [`ApplyPlan`].
#[derive(Debug, Clone)]
pub struct PlannedChange {
    change: Change,
    statements: Range<usize>,
    reversal: ReversalPlan,
}

impl PlannedChange {
    /// The schema operation that produced this group, including no-op changes.
    pub fn change(&self) -> &Change {
        &self.change
    }

    /// Indices into [`ApplyPlan::statements`], in execution order.
    pub fn statement_range(&self) -> Range<usize> {
        self.statements.clone()
    }

    /// Best-effort schema reversal; this does not restore dropped row data.
    pub fn reversal(&self) -> &ReversalPlan {
        &self.reversal
    }
}

/// A contiguous execution phase; ranges never reorder or split a shared group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPhase {
    statements: Range<usize>,
    transaction: TransactionRequirement,
    rollback: RollbackBehavior,
}

impl PlannedPhase {
    /// Indices into [`ApplyPlan::statements`].
    pub fn statement_range(&self) -> Range<usize> {
        self.statements.clone()
    }

    /// How this phase must be executed.
    pub fn transaction(&self) -> TransactionRequirement {
        self.transaction
    }

    /// What a rollback can undo after a statement fails.
    pub fn rollback(&self) -> RollbackBehavior {
        self.rollback
    }
}

/// Ordered SQL with explicit transaction boundaries and change provenance.
///
/// Generated input comes from [`crate::DiffApplier::plan_for`]. SQL loaded from
/// migration files uses [`Self::from_sql`]; it has statement indices but no
/// recoverable schema-change or reversibility metadata.
#[derive(Debug, Clone)]
pub struct ApplyPlan {
    statements: Vec<String>,
    phases: Vec<PlannedPhase>,
    changes: Vec<PlannedChange>,
}

impl ApplyPlan {
    /// Consume changes generated for `provider` and already ordered by
    /// [`crate::order_changes_for_apply`].
    /// Transaction requirements come from generation, never from SQL text.
    pub fn from_changes(provider: DatabaseProvider, changes: Vec<ChangeSqlPlan>) -> Self {
        let mut plan = Self::empty();
        for change in changes {
            let start = plan.statements.len();
            plan.extend(change.statements, change.transaction, provider);
            plan.changes.push(PlannedChange {
                change: change.change,
                statements: start..plan.statements.len(),
                reversal: change.reversal,
            });
        }
        plan
    }

    /// Order `changes` for apply, generate the SQL of each one, and collect
    /// them into a plan.
    ///
    /// This is the whole road from a diff to something runnable: every caller
    /// that has a [`crate::DiffApplier`] and a list of changes wants exactly
    /// these three steps, in this order.
    ///
    /// # Errors
    ///
    /// Returns the generation error of the first change whose SQL cannot be
    /// produced, naming that change.
    pub fn for_changes(
        provider: DatabaseProvider,
        applier: &crate::DiffApplier<'_>,
        live: &crate::LiveSchema,
        changes: &[Change],
    ) -> crate::error::Result<Self> {
        let ordered = crate::order_changes_for_apply(changes, live);
        let plans = ordered
            .iter()
            .map(|change| applier.plan_for(change))
            .collect::<crate::error::Result<Vec<_>>>()?;
        Ok(Self::from_changes(provider, plans))
    }

    /// SQL in dependency order, including comment placeholders from SQL files.
    pub fn statements(&self) -> &[String] {
        &self.statements
    }

    /// Execution boundaries, with provider rollback behavior already resolved.
    pub fn phases(&self) -> &[PlannedPhase] {
        &self.phases
    }

    /// Change provenance and reversal metadata; empty for raw SQL input.
    pub fn changes(&self) -> &[PlannedChange] {
        &self.changes
    }

    /// Classify each generated change using the ranges carried by the plan.
    pub fn classify_changes(&self, outcome: &ApplyOutcome) -> Vec<GroupStatus> {
        self.changes
            .iter()
            .map(|change| outcome.classify_range(change.statements.start, change.statements.end))
            .collect()
    }

    /// Report a failed phase using its rollback contract and statement offset.
    /// `phase` must belong to this plan; `attempted` counts successful statements
    /// within that phase.
    pub fn stopped(
        &self,
        phase: &PlannedPhase,
        attempted: usize,
        failure: ApplyFailure,
    ) -> ApplyOutcome {
        ApplyOutcome::stopped_with_rollback(
            self.statements.len(),
            phase.statements.start,
            attempted,
            phase.rollback,
            failure,
        )
    }

    /// Export generated changes to the existing SQL-only migration format.
    /// Down groups reverse dependency order and preserve manual placeholders.
    pub fn into_migration(self, name: String) -> Migration {
        let down = self
            .changes
            .into_iter()
            .rev()
            .flat_map(|change| change.reversal.into_statements())
            .collect();
        Migration::new(name, self.statements, down)
    }

    fn empty() -> Self {
        Self {
            statements: Vec::new(),
            phases: Vec::new(),
            changes: Vec::new(),
        }
    }

    fn extend(
        &mut self,
        statements: Vec<String>,
        transaction: TransactionRequirement,
        provider: DatabaseProvider,
    ) {
        let rollback = match transaction {
            TransactionRequirement::Shared => RollbackBehavior::for_provider(provider),
            TransactionRequirement::Standalone => RollbackBehavior::KeepsStatements,
        };
        for sql in statements {
            let index = self.statements.len();
            self.statements.push(sql);
            if transaction == TransactionRequirement::Shared {
                if let Some(last) = self.phases.last_mut() {
                    if last.transaction == transaction && last.rollback == rollback {
                        last.statements.end += 1;
                        continue;
                    }
                }
            }
            self.phases.push(PlannedPhase {
                statements: index..index + 1,
                transaction,
                rollback,
            });
        }
    }
}
