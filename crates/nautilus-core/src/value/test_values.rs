use core::f64;
use std::collections::BTreeMap;

use super::Value;

/// One sample per `Value` variant, including the edge cases that take a
/// non-obvious serialization path (non-finite floats -> null, fractional
/// datetimes, nested arrays, hstore NULLs, composite fields).
pub(super) fn plain_equivalence_samples() -> Vec<Value> {
    use chrono::NaiveDate;
    use serde_json::json;
    use uuid::Uuid;

    vec![
        Value::Null,
        Value::Bool(true),
        Value::I32(-42),
        Value::I64(9007199254740991),
        Value::I64(i64::MAX),
        Value::I64(i64::MIN),
        Value::F64(f64::consts::PI),
        Value::F64(f64::NAN),
        Value::F64(f64::INFINITY),
        Value::Decimal(rust_decimal::Decimal::new(-12345, 2)),
        Value::Decimal("12345678901234567890.123456789".parse().unwrap()),
        Value::DateTime(
            NaiveDate::from_ymd_opt(2026, 2, 18)
                .unwrap()
                .and_hms_micro_opt(10, 30, 45, 123456)
                .unwrap(),
        ),
        Value::Uuid(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()),
        Value::Json(json!({"nested": {"ok": true}, "list": [1, "two", null]})),
        Value::Hstore(BTreeMap::from([
            ("display_name".to_string(), Some("Bob".to_string())),
            ("nickname".to_string(), None),
        ])),
        Value::Geometry("POINT(1 2)".to_string()),
        Value::Geography("SRID=4326;POINT(9 45)".to_string()),
        Value::Vector(vec![1.0, -2.5, f32::NAN]),
        Value::String("hello \"quoted\" world".to_string()),
        Value::Bytes(vec![72, 101, 108, 108, 111]),
        Value::Array(vec![
            Value::I32(1),
            Value::String("two".to_string()),
            Value::Null,
            Value::Array(vec![Value::Bool(false)]),
        ]),
        Value::Array2D(vec![
            vec![Value::I32(1), Value::I32(2)],
            vec![Value::I32(3), Value::I32(4)],
        ]),
        Value::Enum {
            value: "ADMIN".to_string(),
            type_name: "role".to_string(),
        },
        Value::Extension {
            value: "MiXeD".to_string(),
            type_name: "citext".to_string(),
        },
        Value::Composite {
            type_name: "championstats".to_string(),
            fields: vec![Value::I32(0), Value::String("x".to_string()), Value::Null],
        },
    ]
}
