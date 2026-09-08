/// Best-effort down SQL and whether it was produced automatically.
/// This describes schema reconstruction, never recovery of dropped row data.
#[derive(Debug, Clone)]
pub struct ReversalPlan {
    statements: Vec<String>,
    automatic: bool,
}

impl ReversalPlan {
    /// SQL and any manual-reversal placeholders, in execution order.
    pub fn statements(&self) -> &[String] {
        &self.statements
    }

    /// False when manual work is needed or an object is deliberately retained.
    /// Automatic reversals retain the existing best-effort reconstruction rules.
    pub fn is_automatic(&self) -> bool {
        self.automatic
    }

    pub(crate) fn manual(statements: Vec<String>) -> Self {
        Self {
            statements,
            automatic: false,
        }
    }

    pub(crate) fn into_statements(self) -> Vec<String> {
        self.statements
    }
}

impl From<Vec<String>> for ReversalPlan {
    fn from(statements: Vec<String>) -> Self {
        Self {
            statements,
            automatic: true,
        }
    }
}
