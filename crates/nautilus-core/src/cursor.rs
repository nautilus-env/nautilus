//! Cursor predicate builder for stable pagination.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::{Expr, Value};

/// Build an inclusive cursor predicate for stable forward or backward pagination.
///
/// `pk_fields` is an ordered slice of `(cursor_map_key, "table__db_col")` pairs
/// matching the primary-key field order of the model:
///
/// - `cursor_map_key` is the key the caller uses in the cursor `HashMap`
///   (typically the snake_case logical field name, e.g. `"id"`, `"user_id"`).
/// - `"table__db_col"` is the `table__column` string rendered by the dialect
///   into `"table"."column"` in the generated SQL.
///
/// # Semantics
///
/// | `backward` | predicate style |
/// |---|---|
/// | `false` (forward) | `pk >= cursor_val` (single) / row-value expansion with `>=` on last field |
/// | `true` (backward) | `pk <= cursor_val` (single) / row-value expansion with `<=` on last field |
///
/// The cursor record is always included in the result set.
///
/// ## Composite PK expansion
///
/// For a 2-field PK `(a, b)` in forward direction, the generated predicate is:
/// ```text
/// (a > v1) OR (a = v1 AND b >= v2)
/// ```
/// This avoids tuple syntax `(a, b) >= (v1, v2)`, which is not universally
/// supported.
///
/// # Errors
///
/// Returns [`Error::InvalidQuery`] if any required key is absent from `cursor`.
pub fn build_cursor_predicate(
    pk_fields: &[(&str, &str)],
    cursor: &HashMap<String, Value>,
    backward: bool,
) -> Result<Expr> {
    if pk_fields.is_empty() {
        return Err(Error::InvalidQuery(
            "build_cursor_predicate: pk_fields must not be empty".to_string(),
        ));
    }
    build_cursor_recursive(pk_fields, cursor, backward, 0)
}

fn build_cursor_recursive(
    pk_fields: &[(&str, &str)],
    cursor: &HashMap<String, Value>,
    backward: bool,
    index: usize,
) -> Result<Expr> {
    let (map_key, col_ref) = pk_fields[index];

    let val = cursor
        .get(map_key)
        .ok_or_else(|| {
            Error::InvalidQuery(format!(
                "cursor missing required primary-key field '{}'",
                map_key
            ))
        })?
        .clone();

    let col = Expr::column(col_ref.to_string());
    let param = Expr::param(val);

    if index == pk_fields.len() - 1 {
        Ok(if backward {
            col.le(param)
        } else {
            col.ge(param)
        })
    } else {
        let strict = if backward {
            col.clone().lt(param.clone())
        } else {
            col.clone().gt(param.clone())
        };
        let eq_and_rest = col.eq(param).and(build_cursor_recursive(
            pk_fields,
            cursor,
            backward,
            index + 1,
        )?);
        Ok(strict.or(eq_and_rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn map(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn single_field_forward() {
        let cursor = map(&[("id", Value::I32(5))]);
        let pred = build_cursor_predicate(&[("id", "users__id")], &cursor, false).unwrap();
        assert!(matches!(
            pred,
            crate::Expr::Binary {
                op: crate::BinaryOp::Ge,
                ..
            }
        ));
    }

    #[test]
    fn single_field_backward() {
        let cursor = map(&[("id", Value::I32(5))]);
        let pred = build_cursor_predicate(&[("id", "users__id")], &cursor, true).unwrap();
        assert!(matches!(
            pred,
            crate::Expr::Binary {
                op: crate::BinaryOp::Le,
                ..
            }
        ));
    }

    #[test]
    fn composite_forward() {
        let cursor = map(&[("user_id", Value::I32(2)), ("post_id", Value::I32(10))]);
        let pred = build_cursor_predicate(
            &[("user_id", "posts__user_id"), ("post_id", "posts__post_id")],
            &cursor,
            false,
        )
        .unwrap();
        assert!(matches!(
            pred,
            crate::Expr::Binary {
                op: crate::BinaryOp::Or,
                ..
            }
        ));
    }

    #[test]
    fn missing_key_returns_error() {
        let cursor = map(&[("id", Value::I32(5))]);
        let result = build_cursor_predicate(&[("missing_field", "t__c")], &cursor, false);
        assert!(result.is_err());
    }

    #[test]
    fn empty_pk_fields_returns_error() {
        let cursor = map(&[("id", Value::I32(5))]);
        let result = build_cursor_predicate(&[], &cursor, false);
        assert!(result.is_err());
    }
}
