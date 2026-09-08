//! Tagged serde preserves variant identity and PostgreSQL type names.

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::scalar_text::{format_datetime, Base64String, DateTimeString, DisplayString};
use super::{parse_datetime, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum SerdeValue {
    Null,
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Decimal(String),
    DateTime(String),
    Uuid(String),
    Json(serde_json::Value),
    Hstore(BTreeMap<String, Option<String>>),
    Geometry(String),
    Geography(String),
    Vector(Vec<f32>),
    String(String),
    Bytes(String),
    Array(Vec<Value>),
    Array2D(Vec<Vec<Value>>),
    Extension {
        value: String,
        type_name: String,
    },
    Enum {
        value: String,
        type_name: String,
    },
    Composite {
        type_name: String,
        fields: Vec<Value>,
    },
}

fn parse_datetime_string(raw: &str) -> std::result::Result<chrono::NaiveDateTime, String> {
    parse_datetime(raw).ok_or_else(|| format!("invalid datetime '{}'", raw))
}

impl From<&Value> for SerdeValue {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => SerdeValue::Null,
            Value::Bool(v) => SerdeValue::Bool(*v),
            Value::I32(v) => SerdeValue::I32(*v),
            Value::I64(v) => SerdeValue::I64(*v),
            Value::F64(v) => SerdeValue::F64(*v),
            Value::Decimal(v) => SerdeValue::Decimal(v.to_string()),
            Value::DateTime(v) => SerdeValue::DateTime(format_datetime(*v)),
            Value::Uuid(v) => SerdeValue::Uuid(v.to_string()),
            Value::Json(v) => SerdeValue::Json(v.clone()),
            Value::Hstore(v) => SerdeValue::Hstore(v.clone()),
            Value::Geometry(v) => SerdeValue::Geometry(v.clone()),
            Value::Geography(v) => SerdeValue::Geography(v.clone()),
            Value::Vector(v) => SerdeValue::Vector(v.clone()),
            Value::String(v) => SerdeValue::String(v.clone()),
            Value::Bytes(v) => {
                use base64::Engine;
                SerdeValue::Bytes(base64::engine::general_purpose::STANDARD.encode(v))
            }
            Value::Array(v) => SerdeValue::Array(v.clone()),
            Value::Array2D(v) => SerdeValue::Array2D(v.clone()),
            Value::Extension { value, type_name } => SerdeValue::Extension {
                value: value.clone(),
                type_name: type_name.clone(),
            },
            Value::Enum { value, type_name } => SerdeValue::Enum {
                value: value.clone(),
                type_name: type_name.clone(),
            },
            Value::Composite { type_name, fields } => SerdeValue::Composite {
                type_name: type_name.clone(),
                fields: fields.clone(),
            },
        }
    }
}

impl TryFrom<SerdeValue> for Value {
    type Error = String;

    fn try_from(value: SerdeValue) -> std::result::Result<Self, Self::Error> {
        match value {
            SerdeValue::Null => Ok(Value::Null),
            SerdeValue::Bool(v) => Ok(Value::Bool(v)),
            SerdeValue::I32(v) => Ok(Value::I32(v)),
            SerdeValue::I64(v) => Ok(Value::I64(v)),
            SerdeValue::F64(v) => Ok(Value::F64(v)),
            SerdeValue::Decimal(raw) => rust_decimal::Decimal::from_str(&raw)
                .map(Value::Decimal)
                .map_err(|e| format!("invalid decimal '{}': {}", raw, e)),
            SerdeValue::DateTime(raw) => parse_datetime_string(&raw).map(Value::DateTime),
            SerdeValue::Uuid(raw) => uuid::Uuid::parse_str(&raw)
                .map(Value::Uuid)
                .map_err(|e| format!("invalid uuid '{}': {}", raw, e)),
            SerdeValue::Json(v) => Ok(Value::Json(v)),
            SerdeValue::Hstore(v) => Ok(Value::Hstore(v)),
            SerdeValue::Geometry(v) => Ok(Value::Geometry(v)),
            SerdeValue::Geography(v) => Ok(Value::Geography(v)),
            SerdeValue::Vector(v) => Ok(Value::Vector(v)),
            SerdeValue::String(v) => Ok(Value::String(v)),
            SerdeValue::Bytes(raw) => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(raw.as_bytes())
                    .map(Value::Bytes)
                    .map_err(|e| format!("invalid base64 bytes '{}': {}", raw, e))
            }
            SerdeValue::Array(v) => Ok(Value::Array(v)),
            SerdeValue::Array2D(v) => Ok(Value::Array2D(v)),
            SerdeValue::Extension { value, type_name } => Ok(Value::Extension { value, type_name }),
            SerdeValue::Enum { value, type_name } => Ok(Value::Enum { value, type_name }),
            SerdeValue::Composite { type_name, fields } => {
                Ok(Value::Composite { type_name, fields })
            }
        }
    }
}

/// Borrowed mirror of [`SerdeValue`]: emits the identical tagged shape but
/// serializes by reference instead of deep-cloning `Json`, `Hstore`, `Vector`,
/// `String`, `Array`, `Array2D`, `Enum` and `Composite` payloads first.
/// Deserialization keeps going through the owned [`SerdeValue`]; the two
/// shapes are kept in sync by equivalence tests over every variant.
#[derive(Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum SerdeValueRef<'a> {
    Null,
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Decimal(DisplayString<&'a rust_decimal::Decimal>),
    DateTime(DateTimeString),
    Uuid(DisplayString<&'a uuid::Uuid>),
    Json(&'a serde_json::Value),
    Hstore(&'a BTreeMap<String, Option<String>>),
    Geometry(&'a str),
    Geography(&'a str),
    Vector(&'a [f32]),
    String(&'a str),
    Bytes(Base64String<'a>),
    Array(&'a [Value]),
    Array2D(&'a [Vec<Value>]),
    Extension {
        value: &'a str,
        type_name: &'a str,
    },
    Enum {
        value: &'a str,
        type_name: &'a str,
    },
    Composite {
        type_name: &'a str,
        fields: &'a [Value],
    },
}

impl<'a> From<&'a Value> for SerdeValueRef<'a> {
    fn from(value: &'a Value) -> Self {
        match value {
            Value::Null => SerdeValueRef::Null,
            Value::Bool(v) => SerdeValueRef::Bool(*v),
            Value::I32(v) => SerdeValueRef::I32(*v),
            Value::I64(v) => SerdeValueRef::I64(*v),
            Value::F64(v) => SerdeValueRef::F64(*v),
            Value::Decimal(v) => SerdeValueRef::Decimal(DisplayString(v)),
            Value::DateTime(v) => SerdeValueRef::DateTime(DateTimeString(*v)),
            Value::Uuid(v) => SerdeValueRef::Uuid(DisplayString(v)),
            Value::Json(v) => SerdeValueRef::Json(v),
            Value::Hstore(v) => SerdeValueRef::Hstore(v),
            Value::Geometry(v) => SerdeValueRef::Geometry(v),
            Value::Geography(v) => SerdeValueRef::Geography(v),
            Value::Vector(v) => SerdeValueRef::Vector(v),
            Value::String(v) => SerdeValueRef::String(v),
            Value::Bytes(v) => SerdeValueRef::Bytes(Base64String(v)),
            Value::Array(v) => SerdeValueRef::Array(v),
            Value::Array2D(v) => SerdeValueRef::Array2D(v),
            Value::Extension { value, type_name } => SerdeValueRef::Extension { value, type_name },
            Value::Enum { value, type_name } => SerdeValueRef::Enum { value, type_name },
            Value::Composite { type_name, fields } => {
                SerdeValueRef::Composite { type_name, fields }
            }
        }
    }
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerdeValueRef::from(self).serialize(serializer)
    }
}

/// Deserializes a [`Value`] from the tagged serde representation emitted by
/// [`Serialize`] for [`Value`].
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let tagged = SerdeValue::deserialize(deserializer)?;
        Value::try_from(tagged).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use core::f64;
    use std::collections::BTreeMap;

    use super::{SerdeValue, Value};
    use crate::value::test_values::plain_equivalence_samples;

    #[test]
    fn test_tagged_serde_shape_is_explicit() {
        let value = Value::Decimal("12345678901234567890.123456789".parse().unwrap());
        let json = serde_json::to_value(&value).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "type": "decimal",
                "value": "12345678901234567890.123456789"
            })
        );
    }

    #[test]
    fn test_tagged_serde_round_trip_preserves_typed_variants() {
        use chrono::NaiveDate;
        use serde_json::json;
        use uuid::Uuid;

        let values = vec![
            Value::Null,
            Value::Bool(false),
            Value::I32(-42),
            Value::I64(i64::MAX),
            Value::I64(i64::MIN),
            Value::F64(f64::consts::E),
            Value::Decimal(rust_decimal::Decimal::new(314, 2)),
            Value::Decimal("12345678901234567890.123456789".parse().unwrap()),
            Value::DateTime(
                NaiveDate::from_ymd_opt(2026, 2, 18)
                    .unwrap()
                    .and_hms_opt(10, 30, 45)
                    .unwrap(),
            ),
            Value::Uuid(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()),
            Value::Bytes(vec![1, 2, 3, 4]),
            Value::Json(json!({"ok": true})),
            Value::Hstore(BTreeMap::from([
                ("display_name".to_string(), Some("Bob".to_string())),
                ("nickname".to_string(), None),
            ])),
            Value::Vector(vec![1.0, 2.0, 3.5]),
            Value::Geometry("POINT(1 2)".to_string()),
            Value::Geography("SRID=4326;POINT(9 45)".to_string()),
            Value::Extension {
                value: "MiXeD".to_string(),
                type_name: "citext".to_string(),
            },
            Value::Composite {
                type_name: "championstats".to_string(),
                fields: vec![Value::I32(0), Value::String("x".to_string()), Value::Null],
            },
            Value::String("test".to_string()),
            Value::Array(vec![Value::I32(1), Value::I32(2)]),
            Value::Array2D(vec![vec![Value::I32(1), Value::I32(2)]]),
            Value::Enum {
                value: "ADMIN".to_string(),
                type_name: "role".to_string(),
            },
        ];

        for value in values {
            let json = serde_json::to_value(&value).unwrap();
            let deserialized: Value = serde_json::from_value(json).unwrap();
            assert_eq!(deserialized, value);
        }
    }

    #[test]
    fn test_tagged_serialize_borrowed_matches_owned_serde_value() {
        for value in plain_equivalence_samples() {
            let via_ref = serde_json::to_string(&value).unwrap();
            let via_owned = serde_json::to_string(&SerdeValue::from(&value)).unwrap();
            assert_eq!(via_ref, via_owned, "variant: {:?}", value);
        }
    }
}
