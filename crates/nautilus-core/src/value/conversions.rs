//! Rust values converted into database values.

use std::collections::BTreeMap;

use super::{Geography, Geometry, Value};

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::I32(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::F64(v as f64)
    }
}

impl From<rust_decimal::Decimal> for Value {
    fn from(v: rust_decimal::Decimal) -> Self {
        Value::Decimal(v)
    }
}

impl From<chrono::NaiveDateTime> for Value {
    fn from(v: chrono::NaiveDateTime) -> Self {
        Value::DateTime(v)
    }
}

impl From<uuid::Uuid> for Value {
    fn from(v: uuid::Uuid) -> Self {
        Value::Uuid(v)
    }
}

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        Value::Json(v)
    }
}

impl From<BTreeMap<String, Option<String>>> for Value {
    fn from(v: BTreeMap<String, Option<String>>) -> Self {
        Value::Hstore(v)
    }
}

impl From<Geometry> for Value {
    fn from(v: Geometry) -> Self {
        Value::Geometry(v.into_inner())
    }
}

impl From<Geography> for Value {
    fn from(v: Geography) -> Self {
        Value::Geography(v.into_inner())
    }
}

impl From<Vec<f32>> for Value {
    fn from(v: Vec<f32>) -> Self {
        Value::Vector(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Bytes(v)
    }
}

/// Array conversions exclude `Vec<u8>`, which represents bytes.
macro_rules! impl_vec_from {
    ($($t:ty),* $(,)?) => {
        $(
            impl From<Vec<$t>> for Value {
                fn from(v: Vec<$t>) -> Self {
                    Value::Array(v.into_iter().map(|x| x.into()).collect())
                }
            }

            impl From<Vec<Vec<$t>>> for Value {
                fn from(v: Vec<Vec<$t>>) -> Self {
                    Value::Array2D(
                        v.into_iter()
                            .map(|row| row.into_iter().map(|x| x.into()).collect())
                            .collect(),
                    )
                }
            }
        )*
    };
}

impl_vec_from!(
    i32,
    i64,
    f64,
    bool,
    String,
    Geometry,
    Geography,
    BTreeMap<String, Option<String>>,
    rust_decimal::Decimal,
    uuid::Uuid,
    chrono::NaiveDateTime,
    serde_json::Value,
);

/// Arrays of the extension scalars a generated client defines itself.
///
/// See [`crate::ExtensionScalar`] for why this conversion cannot live in the
/// generated crate.
impl<T: crate::ExtensionScalar> From<Vec<T>> for Value {
    fn from(values: Vec<T>) -> Self {
        Value::Array(values.into_iter().map(Into::into).collect())
    }
}

/// `None` maps to SQL NULL, `Some(v)` to whatever `v` converts to.
///
/// This is deliberately generic rather than a list of concrete types: generated
/// clients define their own extension wrappers (pgvector, PostGIS, citext, …)
/// and the orphan rule forbids *them* from converting `Option<Wrapper>` into a
/// `Value` defined here, so the conversion has to live on this side.
impl<T> From<Option<T>> for Value
where
    T: Into<Value>,
{
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::Value;

    #[test]
    fn test_value_variants() {
        assert_eq!(Value::Null, Value::Null);
        assert_eq!(Value::Bool(true), Value::from(true));
        assert_eq!(Value::I32(42), Value::from(42i32));
        assert_eq!(Value::I64(42), Value::from(42i64));
        assert_eq!(Value::F64(2.5), Value::from(2.5f64));
        assert_eq!(Value::String("hello".to_string()), Value::from("hello"));
        assert_eq!(Value::Bytes(vec![1, 2, 3]), Value::from(vec![1u8, 2, 3]));

        use rust_decimal::Decimal;
        let dec = Decimal::new(12345, 2);
        assert_eq!(Value::Decimal(dec), Value::from(dec));

        use chrono::NaiveDate;
        let dt = NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        assert_eq!(Value::DateTime(dt), Value::from(dt));

        use uuid::Uuid;
        let id = Uuid::nil();
        assert_eq!(Value::Uuid(id), Value::from(id));

        use serde_json::json;
        let j = json!({"key": "value"});
        assert_eq!(Value::Json(j.clone()), Value::from(j));

        let hstore = BTreeMap::from([
            ("display_name".to_string(), Some("Bob".to_string())),
            ("nickname".to_string(), None),
        ]);
        assert_eq!(Value::Hstore(hstore.clone()), Value::from(hstore));

        assert_eq!(
            Value::Vector(vec![0.1, 0.2]),
            Value::from(vec![0.1f32, 0.2])
        );
    }
}
