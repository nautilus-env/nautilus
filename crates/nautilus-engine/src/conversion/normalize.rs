//! Schema-aware normalization of decoded rows.
//!
//! The connector decodes a row from what the backend reports; when the engine
//! knows the schema type of each projected column it applies a [`ValueHint`] on
//! top, so the same query answers with the same wire types on every provider.

use nautilus_connector::{
    hint_name as scalar_hint_name, normalize_scalar, HintMismatch, Row, ValueHint as ScalarHint,
};
use nautilus_core::Value;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::CompositeTypeIr;

use super::composite::normalize_composite_value;

/// Schema-aware coercion hint for a single projected column.
///
/// These hints are applied above the raw connector decoders when the engine
/// knows the expected schema type for each selected column but the backend row
/// metadata is too weak to recover it reliably (notably SQLite and some MySQL
/// text-affinity types).
#[derive(Debug, Clone, PartialEq)]
pub enum ValueHint {
    /// Coerce a backend integer into [`Value::Bool`].
    ///
    /// SQLite has no boolean type and reports a `Boolean` column as `0`/`1`,
    /// which would make the wire shape of the field depend on the provider.
    Bool,
    /// Coerce a backend value into [`Value::I64`].
    Int,
    /// Coerce a backend value into [`Value::F64`].
    ///
    /// Aggregates make this necessary: `AVG` answers with a numeric string on
    /// PostgreSQL and MySQL and with a float on SQLite, so without a hint the
    /// wire type of the same query depends on the provider.
    Float,
    /// Parse textual / numeric values into [`Value::Decimal`].
    Decimal,
    /// Parse textual values into [`Value::DateTime`].
    DateTime,
    /// Parse JSON text (or wrap scalar backend values) into [`Value::Json`].
    Json,
    /// Parse textual values into [`Value::Uuid`].
    Uuid,
    /// Wrap textual values as [`Value::Geometry`].
    Geometry,
    /// Wrap textual values as [`Value::Geography`].
    Geography,
    /// Parse a PostgreSQL composite record literal (e.g. `(0,0,0)`) into a
    /// [`Value::Json`] object keyed by the composite type's field names.
    ///
    /// The composite definition is carried by `Arc` so the hint stays cheap to
    /// clone while remaining self-contained for the row normalizers.
    Composite(std::sync::Arc<CompositeTypeIr>),
}

/// Normalize a vector of rows using per-column schema hints.
///
/// This is intentionally applied only by modeled query paths. Raw SQL methods
/// keep returning the connector-decoded values without additional guessing.
pub fn normalize_rows_with_hints(
    rows: Vec<Row>,
    hints: &[Option<ValueHint>],
) -> Result<Vec<Row>, ProtocolError> {
    rows.into_iter()
        .map(|row| normalize_row_with_hints(row, hints))
        .collect()
}

/// Normalize a single row using per-column schema hints.
pub fn normalize_row_with_hints(
    row: Row,
    hints: &[Option<ValueHint>],
) -> Result<Row, ProtocolError> {
    if row.len() != hints.len() {
        return Err(ProtocolError::Internal(format!(
            "Schema-aware normalization expected {} projected columns, got {}",
            hints.len(),
            row.len()
        )));
    }

    if hints.iter().all(Option::is_none) {
        return Ok(row);
    }

    let mut normalized_row = Row::with_capacity(row.len());
    for (idx, ((name, value), hint)) in row
        .into_columns_iter()
        .zip(hints.iter().cloned())
        .enumerate()
    {
        let normalized = match hint {
            Some(hint) => normalize_value_with_hint(&name, idx, value, hint)?,
            None => value,
        };
        normalized_row.push_column(name, normalized);
    }

    Ok(normalized_row)
}
fn normalize_value_with_hint(
    column: &str,
    index: usize,
    value: Value,
    hint: ValueHint,
) -> Result<Value, ProtocolError> {
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }

    match hint {
        ValueHint::Bool => normalize_bool_value(column, index, value),
        ValueHint::Int => normalize_int_value(column, index, value),
        ValueHint::Float => normalize_float_value(column, index, value),
        ValueHint::Decimal => normalize_shared_value(column, index, value, ScalarHint::Decimal),
        ValueHint::DateTime => normalize_shared_value(column, index, value, ScalarHint::DateTime),
        ValueHint::Json => normalize_shared_value(column, index, value, ScalarHint::Json),
        ValueHint::Uuid => normalize_shared_value(column, index, value, ScalarHint::Uuid),
        ValueHint::Geometry => normalize_shared_value(column, index, value, ScalarHint::Geometry),
        ValueHint::Geography => normalize_shared_value(column, index, value, ScalarHint::Geography),
        ValueHint::Composite(composite) => {
            normalize_composite_value(column, index, value, &composite)
        }
    }
}

/// Apply a coercion the connector already performs on raw rows, reporting a
/// mismatch as an engine error.
fn normalize_shared_value(
    column: &str,
    index: usize,
    value: Value,
    hint: ScalarHint,
) -> Result<Value, ProtocolError> {
    normalize_scalar(value, hint).map_err(|mismatch| match mismatch {
        HintMismatch::Parse(raw) => invalid_hint_parse(column, index, scalar_hint_name(hint), raw),
        HintMismatch::Incompatible(value) => {
            invalid_hint_value(column, index, scalar_hint_name(hint), value)
        }
    })
}

fn normalize_int_value(column: &str, index: usize, value: Value) -> Result<Value, ProtocolError> {
    match value {
        Value::I64(_) => Ok(value),
        Value::I32(n) => Ok(Value::I64(i64::from(n))),
        Value::F64(n) => Ok(Value::I64(n as i64)),
        Value::Decimal(d) => i64::try_from(d)
            .map(Value::I64)
            .map_err(|_| invalid_hint_parse(column, index, "Int", d.to_string())),
        Value::String(ref raw) => raw
            .parse::<i64>()
            .map(Value::I64)
            .map_err(|_| invalid_hint_parse(column, index, "Int", raw.clone())),
        other => Err(invalid_hint_value(column, index, "Int", other)),
    }
}

fn normalize_float_value(column: &str, index: usize, value: Value) -> Result<Value, ProtocolError> {
    match value {
        Value::F64(_) => Ok(value),
        Value::I32(n) => Ok(Value::F64(f64::from(n))),
        Value::I64(n) => Ok(Value::F64(n as f64)),
        Value::Decimal(d) => d
            .to_string()
            .parse::<f64>()
            .map(Value::F64)
            .map_err(|_| invalid_hint_parse(column, index, "Float", d.to_string())),
        Value::String(ref raw) => raw
            .parse::<f64>()
            .map(Value::F64)
            .map_err(|_| invalid_hint_parse(column, index, "Float", raw.clone())),
        other => Err(invalid_hint_value(column, index, "Float", other)),
    }
}

fn normalize_bool_value(column: &str, index: usize, value: Value) -> Result<Value, ProtocolError> {
    match value {
        Value::Bool(_) => Ok(value),
        Value::I32(n) => Ok(Value::Bool(n != 0)),
        Value::I64(n) => Ok(Value::Bool(n != 0)),
        other => Err(invalid_hint_value(column, index, "Boolean", other)),
    }
}
fn invalid_hint_parse(
    column: &str,
    index: usize,
    hint: &str,
    raw: impl Into<String>,
) -> ProtocolError {
    ProtocolError::DatabaseExecution(format!(
        "Failed to normalize column '{}' at position {} as {} from value {:?}",
        column,
        index,
        hint,
        raw.into()
    ))
}

fn invalid_hint_value(column: &str, index: usize, hint: &str, value: Value) -> ProtocolError {
    ProtocolError::DatabaseExecution(format!(
        "Failed to normalize column '{}' at position {} as {} from incompatible value {:?}",
        column, index, hint, value
    ))
}
