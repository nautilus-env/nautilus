//! Untagged JSON used by transport and raw-query paths, without schema inference.

use serde::{Serialize, Serializer};

use super::scalar_text::{format_datetime, Base64String, DateTimeString};
use super::Value;

impl Value {
    /// Convert this value into the plain JSON shape used on transport/wire paths.
    ///
    /// Unlike the serde representation of [`Value`] itself, this helper
    /// intentionally mirrors the historic untagged encoding used by the engine
    /// and generated raw-query helpers.
    pub fn to_json_plain(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(v) => serde_json::Value::Bool(*v),
            Value::I32(v) => serde_json::Value::Number((*v).into()),
            Value::I64(v) => serde_json::Value::Number((*v).into()),
            Value::F64(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Decimal(v) => serde_json::Value::String(v.to_string()),
            Value::DateTime(v) => serde_json::Value::String(format_datetime(*v)),
            Value::Uuid(v) => serde_json::Value::String(v.to_string()),
            Value::Json(v) => v.clone(),
            Value::Hstore(v) => serde_json::Value::Object(
                v.iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            value
                                .as_ref()
                                .map(|item| serde_json::Value::String(item.clone()))
                                .unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .collect(),
            ),
            Value::Geometry(v) | Value::Geography(v) => serde_json::Value::String(v.clone()),
            Value::Vector(v) => serde_json::Value::Array(
                v.iter()
                    .map(|item| {
                        serde_json::Number::from_f64(*item as f64)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect(),
            ),
            Value::String(v) => serde_json::Value::String(v.clone()),
            Value::Bytes(v) => {
                use base64::Engine;
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(v))
            }
            Value::Array(v) => {
                serde_json::Value::Array(v.iter().map(Value::to_json_plain).collect())
            }
            Value::Array2D(v) => serde_json::Value::Array(
                v.iter()
                    .map(|row| {
                        serde_json::Value::Array(row.iter().map(Value::to_json_plain).collect())
                    })
                    .collect(),
            ),
            Value::Extension { value, .. } | Value::Enum { value, .. } => {
                serde_json::Value::String(value.clone())
            }
            Value::Composite { fields, .. } => {
                serde_json::Value::Array(fields.iter().map(Value::to_json_plain).collect())
            }
        }
    }
}

/// Serializes a borrowed [`Value`] in the same plain JSON shape produced by
/// [`Value::to_json_plain`], writing directly into the serializer.
///
/// Unlike `to_json_plain`, no intermediate `serde_json::Value` tree is built:
/// strings, arrays, hstore maps and composites are serialized by reference.
/// Used on the hot row-serialization path; `to_json_plain` remains for callers
/// that need an owned `serde_json::Value`. The two are kept in sync by
/// equivalence tests over every variant.
pub struct PlainValueRef<'a>(pub &'a Value);

/// Serializes an `f64` as a JSON number, or `null` when not finite —
/// mirroring `serde_json::Number::from_f64` returning `None` for NaN/±∞.
struct PlainF64(f64);

impl Serialize for PlainF64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.0.is_finite() {
            serializer.serialize_f64(self.0)
        } else {
            serializer.serialize_unit()
        }
    }
}

struct PlainSliceRef<'a>(&'a [Value]);

impl Serialize for PlainSliceRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().map(PlainValueRef))
    }
}

impl Serialize for PlainValueRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(v) => serializer.serialize_bool(*v),
            Value::I32(v) => serializer.serialize_i32(*v),
            Value::I64(v) => serializer.serialize_i64(*v),
            Value::F64(v) => PlainF64(*v).serialize(serializer),
            Value::Decimal(v) => serializer.collect_str(v),
            Value::DateTime(v) => DateTimeString(*v).serialize(serializer),
            Value::Uuid(v) => serializer.collect_str(v),
            Value::Json(v) => v.serialize(serializer),
            Value::Hstore(v) => serializer.collect_map(v.iter()),
            Value::Geometry(v) | Value::Geography(v) => serializer.serialize_str(v),
            Value::Vector(v) => serializer.collect_seq(v.iter().map(|item| PlainF64(*item as f64))),
            Value::String(v) => serializer.serialize_str(v),
            Value::Bytes(v) => Base64String(v).serialize(serializer),
            Value::Array(v) => serializer.collect_seq(v.iter().map(PlainValueRef)),
            Value::Array2D(v) => serializer.collect_seq(v.iter().map(|row| PlainSliceRef(row))),
            Value::Extension { value, .. } | Value::Enum { value, .. } => {
                serializer.serialize_str(value)
            }
            Value::Composite { fields, .. } => {
                serializer.collect_seq(fields.iter().map(PlainValueRef))
            }
        }
    }
}

/// Infer an internal value from JSON without schema metadata.
///
/// Numbers are coerced to `I32` before `I64` when they fit, then `F64`.
/// Arrays of arrays are **not** auto-promoted to `Array2D` here; that
/// promotion happens in the connector stream decoders where full schema
/// knowledge is available.
pub(crate) fn json_to_value_ref(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Value::I32(i as i32)
                } else {
                    Value::I64(i)
                }
            } else if let Some(f) = n.as_f64() {
                Value::F64(f)
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => Value::Array(arr.iter().map(json_to_value_ref).collect()),
        serde_json::Value::Object(_) => Value::Json(json.clone()),
    }
}

#[cfg(test)]
mod tests {
    use core::f64;
    use std::collections::BTreeMap;

    use super::{json_to_value_ref, PlainValueRef, Value};
    use crate::value::test_values::plain_equivalence_samples;

    #[test]
    fn test_value_to_json_plain_primitives() {
        assert_eq!(Value::Null.to_json_plain(), serde_json::Value::Null);
        assert_eq!(
            Value::Bool(true).to_json_plain(),
            serde_json::Value::Bool(true)
        );
        assert_eq!(Value::I32(42).to_json_plain().as_i64(), Some(42));
        assert_eq!(
            Value::I64(9007199254740991).to_json_plain().as_i64(),
            Some(9007199254740991)
        );
        assert_eq!(
            Value::F64(f64::consts::PI).to_json_plain().as_f64(),
            Some(f64::consts::PI)
        );
        assert_eq!(
            Value::String("hello world".to_string())
                .to_json_plain()
                .as_str(),
            Some("hello world")
        );
    }

    #[test]
    fn test_value_to_json_plain_special_scalars() {
        use rust_decimal::Decimal;
        let dec = Decimal::new(12345, 2);
        use chrono::NaiveDate;
        let dt = NaiveDate::from_ymd_opt(2026, 2, 18)
            .unwrap()
            .and_hms_opt(10, 30, 45)
            .unwrap();
        use uuid::Uuid;
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(Value::Decimal(dec).to_json_plain().as_str(), Some("123.45"));
        assert!(Value::DateTime(dt)
            .to_json_plain()
            .as_str()
            .unwrap()
            .starts_with("2026-02-18T10:30:45"));
        assert_eq!(
            Value::Uuid(id).to_json_plain().as_str(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(
            Value::Bytes(vec![72, 101, 108, 108, 111])
                .to_json_plain()
                .as_str(),
            Some("SGVsbG8=")
        );
    }

    #[test]
    fn test_value_to_json_plain_json_and_arrays() {
        use serde_json::json;
        let object = json!({"name": "Alice", "age": 30});
        assert_eq!(Value::Json(object.clone()).to_json_plain(), object);

        let value = Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ]);

        let json = value.to_json_plain();
        assert_eq!(json[0].as_str(), Some("a"));
        assert_eq!(json[1].as_str(), Some("b"));
        assert_eq!(json[2].as_str(), Some("c"));
    }

    #[test]
    fn test_value_to_json_plain_hstore() {
        let value = Value::Hstore(BTreeMap::from([
            ("display_name".to_string(), Some("Bob".to_string())),
            ("nickname".to_string(), None),
        ]));

        assert_eq!(
            value.to_json_plain(),
            serde_json::json!({
                "display_name": "Bob",
                "nickname": null
            })
        );
    }

    #[test]
    fn test_value_to_json_plain_vector() {
        let json = Value::Vector(vec![1.0, 2.5, 3.25]).to_json_plain();
        assert_eq!(json, serde_json::json!([1.0, 2.5, 3.25]));
    }

    #[test]
    fn test_value_plain_json_array2d_roundtrip_stays_untyped_without_schema() {
        let value = Value::Array2D(vec![
            vec![Value::I32(1), Value::I32(2)],
            vec![Value::I32(3), Value::I32(4)],
        ]);

        let json = value.to_json_plain();
        assert_eq!(json[0][0].as_i64(), Some(1));
        assert_eq!(json[0][1].as_i64(), Some(2));
        assert_eq!(json[1][0].as_i64(), Some(3));
        assert_eq!(json[1][1].as_i64(), Some(4));

        let expected = Value::Array(vec![
            Value::Array(vec![Value::I32(1), Value::I32(2)]),
            Value::Array(vec![Value::I32(3), Value::I32(4)]),
        ]);
        assert_eq!(json_to_value_ref(&json), expected);
    }

    #[test]
    fn test_plain_value_ref_matches_to_json_plain_tree() {
        for value in plain_equivalence_samples() {
            let via_ref = serde_json::to_value(PlainValueRef(&value)).unwrap();
            assert_eq!(via_ref, value.to_json_plain(), "variant: {:?}", value);
        }
    }

    #[test]
    fn test_plain_value_ref_matches_to_json_plain_string() {
        for value in plain_equivalence_samples() {
            let via_ref = serde_json::to_string(&PlainValueRef(&value)).unwrap();
            let via_tree = serde_json::to_string(&value.to_json_plain()).unwrap();
            assert_eq!(via_ref, via_tree, "variant: {:?}", value);
        }
    }
}
