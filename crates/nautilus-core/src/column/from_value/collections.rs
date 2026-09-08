//! Native arrays, vectors and their JSON storage equivalents.

use std::collections::BTreeMap;

use super::{ExtensionScalar, FromValue};
use crate::{Result, Value};

impl<T: ExtensionScalar> FromValue for Vec<T> {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Array(items) => items.iter().map(T::from_value).collect(),
            Value::Json(json_value) => decode_json_array(json_value, T::from_value),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for an extension scalar array".to_string(),
            )),
            other => Err(crate::Error::TypeError(format!(
                "expected Array or Json for an extension scalar array, got {:?}",
                other
            ))),
        }
    }
}

impl FromValue for Vec<f32> {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Vector(values) => Ok(values.clone()),
            Value::Array(items) => items.iter().map(f32::from_value).collect(),
            Value::Json(json_value) => decode_json_array(json_value, f32::from_value),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for Vec<f32>".to_string(),
            )),
            _ => Err(crate::Error::TypeError(format!(
                "expected Vector, Array, or Json for Vec<f32>, got {:?}",
                value
            ))),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Vector(values) => Ok(values),
            other => Self::from_value(&other),
        }
    }
}

/// Generates `FromValue` implementations for `Vec<T>` and `Vec<Vec<T>>` for
/// scalar types that already implement `FromValue`.
///
/// Both variants support:
/// - Native PostgreSQL array values (`Value::Array` / `Value::Array2D`)
/// - JSON-encoded arrays from MySQL and SQLite (`Value::Json`)
macro_rules! impl_vec_from_value {
    ($T:ty) => {
        impl FromValue for Vec<$T> {
            fn from_value(value: &Value) -> Result<Self> {
                match value {
                    Value::Array(items) => items.iter().map(<$T>::from_value).collect(),
                    Value::Json(json_value) => decode_json_array(json_value, <$T>::from_value),
                    Value::Null => Err(crate::Error::TypeError(
                        concat!("NULL value for Vec<", stringify!($T), ">").to_string(),
                    )),
                    _ => Err(crate::Error::TypeError(format!(
                        concat!(
                            "expected Array or Json for Vec<",
                            stringify!($T),
                            ">, got {:?}"
                        ),
                        value
                    ))),
                }
            }
        }

        impl FromValue for Vec<Vec<$T>> {
            fn from_value(value: &Value) -> Result<Self> {
                match value {
                    Value::Array2D(rows) => rows
                        .iter()
                        .map(|row| row.iter().map(<$T>::from_value).collect())
                        .collect(),
                    Value::Json(json_value) => decode_json_2d_array(json_value, <$T>::from_value),
                    Value::Null => Err(crate::Error::TypeError(
                        concat!("NULL value for Vec<Vec<", stringify!($T), ">>").to_string(),
                    )),
                    _ => Err(crate::Error::TypeError(format!(
                        concat!(
                            "expected Array2D or Json for Vec<Vec<",
                            stringify!($T),
                            ">>, got {:?}"
                        ),
                        value
                    ))),
                }
            }
        }
    };
}

impl_vec_from_value!(String);
impl_vec_from_value!(i32);
impl_vec_from_value!(i64);
impl_vec_from_value!(f64);
impl_vec_from_value!(bool);
impl_vec_from_value!(rust_decimal::Decimal);
impl_vec_from_value!(chrono::NaiveDateTime);
impl_vec_from_value!(uuid::Uuid);
impl_vec_from_value!(serde_json::Value);
impl_vec_from_value!(BTreeMap<String, Option<String>>);

fn decode_json_array<T, F>(json_value: &serde_json::Value, decoder: F) -> Result<Vec<T>>
where
    F: Fn(&Value) -> Result<T>,
{
    if let serde_json::Value::Array(arr) = json_value {
        let mut result = Vec::with_capacity(arr.len());
        for json_item in arr {
            let value = crate::value::json_to_value_ref(json_item);
            result.push(decoder(&value)?);
        }
        Ok(result)
    } else {
        Err(crate::Error::TypeError(format!(
            "expected JSON array, got {:?}",
            json_value
        )))
    }
}

fn decode_json_2d_array<T, F>(json_value: &serde_json::Value, decoder: F) -> Result<Vec<Vec<T>>>
where
    F: Fn(&Value) -> Result<T>,
{
    if let serde_json::Value::Array(outer_arr) = json_value {
        let mut result = Vec::with_capacity(outer_arr.len());
        for json_row in outer_arr {
            if let serde_json::Value::Array(inner_arr) = json_row {
                let mut row = Vec::with_capacity(inner_arr.len());
                for json_item in inner_arr {
                    let value = crate::value::json_to_value_ref(json_item);
                    row.push(decoder(&value)?);
                }
                result.push(row);
            } else {
                return Err(crate::Error::TypeError(format!(
                    "expected inner JSON array, got {:?}",
                    json_row
                )));
            }
        }
        Ok(result)
    } else {
        Err(crate::Error::TypeError(format!(
            "expected JSON 2D array, got {:?}",
            json_value
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ExtensionScalar, FromValue};
    use crate::{Result, Value};

    #[test]
    fn decimal_uuid_datetime_and_json_arrays_decode_from_json_storage() {
        let decimals = Value::Json(serde_json::json!(["12.34", "56.78"]));
        let uuids = Value::Json(serde_json::json!([
            "550e8400-e29b-41d4-a716-446655440000",
            "123e4567-e89b-12d3-a456-426614174000"
        ]));
        let datetimes = Value::Json(serde_json::json!([
            "2026-02-18T10:30:45Z",
            "2026-02-19T11:31:46Z"
        ]));
        let jsons = Value::Json(serde_json::json!([
            {"name": "Alice"},
            42,
            true
        ]));

        let decimals: Vec<rust_decimal::Decimal> = Vec::from_value(&decimals).unwrap();
        let uuids: Vec<uuid::Uuid> = Vec::from_value(&uuids).unwrap();
        let datetimes: Vec<chrono::NaiveDateTime> = Vec::from_value(&datetimes).unwrap();
        let jsons: Vec<serde_json::Value> = Vec::from_value(&jsons).unwrap();

        assert_eq!(decimals.len(), 2);
        assert_eq!(uuids.len(), 2);
        assert_eq!(datetimes.len(), 2);
        assert_eq!(
            jsons,
            vec![
                serde_json::json!({"name": "Alice"}),
                serde_json::json!(42),
                serde_json::json!(true),
            ]
        );
    }

    #[test]
    fn hstore_arrays_decode_from_json_storage() {
        let json = Value::Json(serde_json::json!([
            {"display_name": "Bob", "nickname": null},
            {"display_name": "OpenAI", "nickname": "oai"}
        ]));

        let decoded: Vec<BTreeMap<String, Option<String>>> = Vec::from_value(&json).unwrap();

        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0]["display_name"], Some("Bob".to_string()));
        assert_eq!(decoded[0]["nickname"], None);
        assert_eq!(decoded[1]["nickname"], Some("oai".to_string()));
    }

    /// Stands in for the wrappers a generated client defines for the
    /// PostgreSQL extension scalars.
    #[derive(Debug, Clone, PartialEq)]
    struct Tag(String);

    impl From<Tag> for Value {
        fn from(value: Tag) -> Self {
            Value::String(value.0)
        }
    }

    impl FromValue for Tag {
        fn from_value(value: &Value) -> Result<Self> {
            String::from_value(value).map(Tag)
        }
    }

    impl ExtensionScalar for Tag {}

    #[test]
    fn extension_scalar_arrays_decode_from_native_and_json_values() {
        let native = Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]);
        let json = Value::Json(serde_json::json!(["a", "b"]));
        let expected = vec![Tag("a".to_string()), Tag("b".to_string())];

        assert_eq!(Vec::<Tag>::from_value(&native).unwrap(), expected);
        assert_eq!(Vec::<Tag>::from_value(&json).unwrap(), expected);
        assert!(Vec::<Tag>::from_value(&Value::Null).is_err());
    }

    #[test]
    fn extension_scalar_arrays_encode_as_array_values() {
        let values = vec![Tag("a".to_string()), Tag("b".to_string())];

        assert_eq!(
            Value::from(values),
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ])
        );
    }

    #[test]
    fn optional_extension_scalars_encode_through_the_generic_conversion() {
        assert_eq!(
            Value::from(Some(Tag("a".to_string()))),
            Value::String("a".to_string())
        );
        assert_eq!(Value::from(None::<Tag>), Value::Null);
    }

    #[test]
    fn vector_decodes_from_native_and_json_values() {
        let native = Value::Vector(vec![0.1, 0.2, 0.3]);
        let json = Value::Json(serde_json::json!([0.1, 0.2, 0.3]));

        assert_eq!(
            Vec::<f32>::from_value(&native).unwrap(),
            vec![0.1, 0.2, 0.3]
        );
        assert_eq!(Vec::<f32>::from_value(&json).unwrap(), vec![0.1, 0.2, 0.3]);
    }
}
