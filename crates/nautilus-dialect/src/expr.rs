//! Expression-level pieces shared by the dialects: the operator spellings, and
//! the default parameter cast for a dialect that needs none.

/// The `$param_cast` hook for dialects that never cast a bound parameter.
///
/// PostgreSQL is the only dialect that needs one: it binds several values as
/// text (pgvector, PostGIS, JSON) and the server refuses to assign text to a
/// column of the real type without an explicit cast.
pub(crate) fn no_param_cast(_value: &nautilus_core::Value) -> Option<String> {
    None
}

/// Return the SQL operator keyword for a standard scalar binary operation.
///
/// Call only for the nine scalar operators (Eq through Like).  Composite cases
/// (IN/NOT IN, array operators) must be handled separately by each dialect before
/// delegating to this helper.
#[inline]
pub(crate) fn binary_op_sql(op: &nautilus_core::BinaryOp) -> &'static str {
    match op {
        nautilus_core::BinaryOp::Eq => "=",
        nautilus_core::BinaryOp::Ne => "!=",
        nautilus_core::BinaryOp::Lt => "<",
        nautilus_core::BinaryOp::Le => "<=",
        nautilus_core::BinaryOp::Gt => ">",
        nautilus_core::BinaryOp::Ge => ">=",
        nautilus_core::BinaryOp::And => "AND",
        nautilus_core::BinaryOp::Or => "OR",
        nautilus_core::BinaryOp::Like | nautilus_core::BinaryOp::LikeEscape => "LIKE",
        nautilus_core::BinaryOp::Add => "+",
        nautilus_core::BinaryOp::Sub => "-",
        nautilus_core::BinaryOp::Mul => "*",
        nautilus_core::BinaryOp::Div => "/",
        nautilus_core::BinaryOp::ArrayContains
        | nautilus_core::BinaryOp::ArrayContainedBy
        | nautilus_core::BinaryOp::ArrayOverlaps
        | nautilus_core::BinaryOp::In
        | nautilus_core::BinaryOp::NotIn => {
            unreachable!(
                "binary_op_sql: operator {:?} must be handled by dialect-specific code",
                op
            )
        }
    }
}
