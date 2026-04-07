//! Schema-aware row normalization helpers.
//!
//! The raw connector intentionally decodes only what backend row metadata can
//! prove without schema context. These helpers let higher layers apply
//! per-column hints afterward when they *do* know the expected types.

use std::str::FromStr;

use crate::error::{ConnectorError as Error, Result};
use crate::{FromRow, Row};
use nautilus_core::Value;
use uuid::Uuid;

/// Schema-aware coercion hint for a single projected column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueHint {
    /// Parse textual / numeric values into [`Value::Decimal`].
    Decimal,
    /// Parse textual values into [`Value::DateTime`].
    DateTime,
    /// Parse JSON text (or wrap scalar backend values) into [`Value::Json`].
    Json,
    /// Parse textual values into [`Value::Uuid`].
    Uuid,
}

/// Normalize a vector of rows using per-column schema hints.
pub fn normalize_rows_with_hints(rows: Vec<Row>, hints: &[Option<ValueHint>]) -> Result<Vec<Row>> {
    rows.into_iter()
        .map(|row| normalize_row_with_hints(row, hints))
        .collect()
}

/// Normalize a single row using per-column schema hints.
pub fn normalize_row_with_hints(row: Row, hints: &[Option<ValueHint>]) -> Result<Row> {
    if row.len() != hints.len() {
        return Err(Error::row_decode_msg(format!(
            "Schema-aware normalization expected {} projected columns, got {}",
            hints.len(),
            row.len()
        )));
    }

    let columns = row
        .columns()
        .iter()
        .enumerate()
        .map(|(idx, (name, value))| {
            let normalized = match hints[idx] {
                Some(hint) => normalize_value_with_hint(name, idx, value.clone(), hint)?,
                None => value.clone(),
            };
            Ok::<(String, Value), crate::ConnectorError>((name.clone(), normalized))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Row::new(columns))
}

/// Normalize a row with hints and decode it via [`FromRow`].
pub fn decode_row_with_hints<T: FromRow>(row: Row, hints: &[Option<ValueHint>]) -> Result<T> {
    let row = normalize_row_with_hints(row, hints)?;
    T::from_row(&row).map_err(Error::from)
}

fn parse_datetime_string(raw: &str) -> Option<chrono::NaiveDateTime> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(dt.naive_utc());
    }

    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(dt);
        }
    }

    None
}

fn normalize_value_with_hint(
    column: &str,
    index: usize,
    value: Value,
    hint: ValueHint,
) -> Result<Value> {
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }

    match hint {
        ValueHint::Decimal => normalize_decimal_value(column, index, value),
        ValueHint::DateTime => normalize_datetime_value(column, index, value),
        ValueHint::Json => normalize_json_value(column, index, value),
        ValueHint::Uuid => normalize_uuid_value(column, index, value),
    }
}

fn normalize_decimal_value(column: &str, index: usize, value: Value) -> Result<Value> {
    match value {
        Value::Decimal(decimal) => Ok(Value::Decimal(decimal)),
        Value::I32(n) => parse_decimal(column, index, &n.to_string()),
        Value::I64(n) => parse_decimal(column, index, &n.to_string()),
        Value::F64(n) if n.is_finite() => parse_decimal(column, index, &n.to_string()),
        Value::String(raw) => parse_decimal(column, index, &raw),
        other => Err(invalid_hint_value(column, index, ValueHint::Decimal, other)),
    }
}

fn normalize_datetime_value(column: &str, index: usize, value: Value) -> Result<Value> {
    match value {
        Value::DateTime(dt) => Ok(Value::DateTime(dt)),
        Value::String(raw) => parse_datetime_string(&raw)
            .map(Value::DateTime)
            .ok_or_else(|| invalid_hint_parse(column, index, ValueHint::DateTime, raw)),
        other => Err(invalid_hint_value(
            column,
            index,
            ValueHint::DateTime,
            other,
        )),
    }
}

fn normalize_json_value(column: &str, index: usize, value: Value) -> Result<Value> {
    match value {
        Value::Json(json) => Ok(Value::Json(json)),
        Value::String(raw) => serde_json::from_str::<serde_json::Value>(&raw)
            .map(Value::Json)
            .map_err(|_| invalid_hint_parse(column, index, ValueHint::Json, raw)),
        other => Ok(Value::Json(other.to_json_plain())),
    }
}

fn normalize_uuid_value(column: &str, index: usize, value: Value) -> Result<Value> {
    match value {
        Value::Uuid(uuid) => Ok(Value::Uuid(uuid)),
        Value::String(raw) => Uuid::parse_str(&raw)
            .map(Value::Uuid)
            .map_err(|_| invalid_hint_parse(column, index, ValueHint::Uuid, raw)),
        other => Err(invalid_hint_value(column, index, ValueHint::Uuid, other)),
    }
}

fn parse_decimal(column: &str, index: usize, raw: &str) -> Result<Value> {
    rust_decimal::Decimal::from_str(raw)
        .map(Value::Decimal)
        .map_err(|_| invalid_hint_parse(column, index, ValueHint::Decimal, raw.to_string()))
}

fn invalid_hint_parse(
    column: &str,
    index: usize,
    hint: ValueHint,
    raw: impl Into<String>,
) -> crate::ConnectorError {
    Error::row_decode_msg(format!(
        "Failed to normalize column '{}' at position {} as {} from value {:?}",
        column,
        index,
        hint_name(hint),
        raw.into()
    ))
}

fn invalid_hint_value(
    column: &str,
    index: usize,
    hint: ValueHint,
    value: Value,
) -> crate::ConnectorError {
    Error::row_decode_msg(format!(
        "Failed to normalize column '{}' at position {} as {} from incompatible value {:?}",
        column,
        index,
        hint_name(hint),
        value
    ))
}

fn hint_name(hint: ValueHint) -> &'static str {
    match hint {
        ValueHint::Decimal => "Decimal",
        ValueHint::DateTime => "DateTime",
        ValueHint::Json => "Json",
        ValueHint::Uuid => "Uuid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_row_with_hints_parses_schema_aware_values() {
        let row = Row::new(vec![
            ("price".to_string(), Value::F64(12.34)),
            (
                "profile".to_string(),
                Value::String(r#"{"name":"Alice","active":true}"#.to_string()),
            ),
            (
                "external_id".to_string(),
                Value::String("550e8400-e29b-41d4-a716-446655440000".to_string()),
            ),
            (
                "created_at".to_string(),
                Value::String("2026-03-30T12:34:56Z".to_string()),
            ),
        ]);

        let normalized = normalize_row_with_hints(
            row,
            &[
                Some(ValueHint::Decimal),
                Some(ValueHint::Json),
                Some(ValueHint::Uuid),
                Some(ValueHint::DateTime),
            ],
        )
        .unwrap();

        assert_eq!(
            normalized.get("price"),
            Some(&Value::Decimal(rust_decimal::Decimal::new(1234, 2)))
        );
        assert_eq!(
            normalized.get("profile"),
            Some(&Value::Json(
                serde_json::json!({"name":"Alice","active":true})
            ))
        );
        assert_eq!(
            normalized.get("external_id"),
            Some(&Value::Uuid(
                uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
            ))
        );
        assert_eq!(
            normalized.get("created_at"),
            Some(&Value::DateTime(
                chrono::NaiveDate::from_ymd_opt(2026, 3, 30)
                    .unwrap()
                    .and_hms_opt(12, 34, 56)
                    .unwrap()
            ))
        );
    }

    #[test]
    fn normalize_row_with_hints_rejects_invalid_json() {
        let row = Row::new(vec![(
            "profile".to_string(),
            Value::String("not-json".to_string()),
        )]);

        let err = normalize_row_with_hints(row, &[Some(ValueHint::Json)]).unwrap_err();
        assert!(err.to_string().contains("profile"));
    }

    #[test]
    fn decode_row_with_hints_flows_into_from_row() {
        let row = Row::new(vec![
            ("id".to_string(), Value::I64(1)),
            (
                "tags".to_string(),
                Value::String(r#"["orm","sqlite"]"#.to_string()),
            ),
        ]);

        let decoded: (i64, Vec<String>) =
            decode_row_with_hints(row, &[None, Some(ValueHint::Json)]).unwrap();

        assert_eq!(decoded.0, 1);
        assert_eq!(decoded.1, vec!["orm".to_string(), "sqlite".to_string()]);
    }
}
