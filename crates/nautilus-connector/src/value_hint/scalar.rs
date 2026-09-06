//! Column-independent coercions shared by every layer that applies schema hints.
//!
//! These primitives know nothing about rows, column positions or the error type
//! of their caller: they answer with the coerced [`Value`] or with the reason it
//! could not be produced, so the connector and the engine can keep their own
//! error mapping while sharing one implementation per hint.

use std::str::FromStr;

use nautilus_core::{parse_datetime, Value};
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
    /// Wrap textual values as [`Value::Geometry`].
    Geometry,
    /// Wrap textual values as [`Value::Geography`].
    Geography,
}

/// Why a value could not be coerced to its hinted type.
#[derive(Debug, Clone, PartialEq)]
pub enum HintMismatch {
    /// The value had the right shape but its text could not be parsed.
    Parse(String),
    /// The hint does not apply to a value of this kind.
    Incompatible(Value),
}

/// Coerce one value to its hinted type, leaving `NULL` untouched.
pub fn normalize_scalar(value: Value, hint: ValueHint) -> Result<Value, HintMismatch> {
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }

    match hint {
        ValueHint::Decimal => normalize_decimal(value),
        ValueHint::DateTime => normalize_datetime(value),
        ValueHint::Json => normalize_json(value),
        ValueHint::Uuid => normalize_uuid(value),
        ValueHint::Geometry => normalize_geometry(value),
        ValueHint::Geography => normalize_geography(value),
    }
}

/// Name used for a hint in diagnostics.
pub fn hint_name(hint: ValueHint) -> &'static str {
    match hint {
        ValueHint::Decimal => "Decimal",
        ValueHint::DateTime => "DateTime",
        ValueHint::Json => "Json",
        ValueHint::Uuid => "Uuid",
        ValueHint::Geometry => "Geometry",
        ValueHint::Geography => "Geography",
    }
}

fn normalize_decimal(value: Value) -> Result<Value, HintMismatch> {
    match value {
        Value::Decimal(decimal) => Ok(Value::Decimal(decimal)),
        Value::I32(n) => parse_decimal(&n.to_string()),
        Value::I64(n) => parse_decimal(&n.to_string()),
        Value::F64(n) if n.is_finite() => parse_decimal(&n.to_string()),
        Value::String(raw) => parse_decimal(&raw),
        other => Err(HintMismatch::Incompatible(other)),
    }
}

fn normalize_datetime(value: Value) -> Result<Value, HintMismatch> {
    match value {
        Value::DateTime(dt) => Ok(Value::DateTime(dt)),
        Value::String(raw) => parse_datetime(&raw)
            .map(Value::DateTime)
            .ok_or(HintMismatch::Parse(raw)),
        other => Err(HintMismatch::Incompatible(other)),
    }
}

fn normalize_json(value: Value) -> Result<Value, HintMismatch> {
    match value {
        Value::Json(json) => Ok(Value::Json(json)),
        Value::String(raw) => serde_json::from_str::<serde_json::Value>(&raw)
            .map(Value::Json)
            .map_err(|_| HintMismatch::Parse(raw)),
        other => Ok(Value::Json(other.to_json_plain())),
    }
}

fn normalize_uuid(value: Value) -> Result<Value, HintMismatch> {
    match value {
        Value::Uuid(uuid) => Ok(Value::Uuid(uuid)),
        Value::String(raw) => Uuid::parse_str(&raw)
            .map(Value::Uuid)
            .map_err(|_| HintMismatch::Parse(raw)),
        other => Err(HintMismatch::Incompatible(other)),
    }
}

fn normalize_geometry(value: Value) -> Result<Value, HintMismatch> {
    match value {
        Value::Geometry(raw) | Value::String(raw) => Ok(Value::Geometry(raw)),
        other => Err(HintMismatch::Incompatible(other)),
    }
}

fn normalize_geography(value: Value) -> Result<Value, HintMismatch> {
    match value {
        Value::Geography(raw) | Value::String(raw) => Ok(Value::Geography(raw)),
        other => Err(HintMismatch::Incompatible(other)),
    }
}

fn parse_decimal(raw: &str) -> Result<Value, HintMismatch> {
    rust_decimal::Decimal::from_str(raw)
        .map(Value::Decimal)
        .map_err(|_| HintMismatch::Parse(raw.to_string()))
}
