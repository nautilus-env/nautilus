use super::{ApplyPlan, TransactionRequirement};
use crate::apply::ApplyPhase;
use crate::ddl::DatabaseProvider;

impl ApplyPlan {
    /// Adapt SQL-only migrations, including existing and hand-edited files.
    ///
    /// Preserves the historical textual classification and statement order.
    /// No schema origin or down-SQL completeness can be inferred from this input.
    pub fn from_sql(provider: DatabaseProvider, statements: &[String]) -> Self {
        let mut plan = Self::empty();
        for sql in statements {
            let transaction = if requires_own_transaction(sql) {
                TransactionRequirement::Standalone
            } else {
                TransactionRequirement::Shared
            };
            plan.extend(vec![sql.clone()], transaction, provider);
        }
        plan
    }
}

/// Compatibility adapter for SQL-only callers; generated changes use
/// [`crate::DiffApplier::plan_for`] and [`ApplyPlan::from_changes`].
pub fn plan_apply_phases(statements: &[String]) -> Vec<ApplyPhase> {
    let plan = ApplyPlan::from_sql(DatabaseProvider::Postgres, statements);
    plan.phases()
        .iter()
        .map(|phase| {
            let sql = &plan.statements()[phase.statement_range()];
            match phase.transaction() {
                TransactionRequirement::Standalone => ApplyPhase::Standalone(sql[0].clone()),
                TransactionRequirement::Shared => ApplyPhase::Transaction(sql.to_vec()),
            }
        })
        .collect()
}

/// Legacy SQL-file classification for PostgreSQL enum additions. A newly
/// added value must commit before a later statement can use it as a default.
/// This textual rule is retained for compatibility, not used for generated DDL.
pub fn requires_own_transaction(sql: &str) -> bool {
    let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = normalized.to_uppercase();
    upper.starts_with("ALTER TYPE ") && upper.contains(" ADD VALUE ")
}
