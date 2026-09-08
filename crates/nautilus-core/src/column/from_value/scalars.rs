//! Scalar and nullable column decoding, including owned strings and bytes.

use super::FromValue;
use crate::{Result, Value};

impl FromValue for i64 {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::I64(v) => Ok(*v),
            Value::Null => Err(crate::Error::TypeError("NULL value for i64".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected i64, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for i32 {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::I32(v) => Ok(*v),
            Value::I64(v) => (*v).try_into().map_err(|_| {
                crate::Error::TypeError(format!("i64 value {} doesn't fit in i32", v))
            }),
            Value::Null => Err(crate::Error::TypeError("NULL value for i32".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected i32, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(v) => Ok(v.clone()),
            Value::Null => Err(crate::Error::TypeError("NULL value for String".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected String, got {:?}",
                value
            ))),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::String(v) => Ok(v),
            Value::Null => Err(crate::Error::TypeError("NULL value for String".to_string())),
            other => Err(crate::Error::TypeError(format!(
                "expected String, got {:?}",
                other
            ))),
        }
    }
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Bool(v) => Ok(*v),
            Value::I32(v) => match *v {
                0 => Ok(false),
                1 => Ok(true),
                other => Err(crate::Error::TypeError(format!(
                    "expected bool-compatible i32 (0 or 1), got {}",
                    other
                ))),
            },
            Value::I64(v) => match *v {
                0 => Ok(false),
                1 => Ok(true),
                other => Err(crate::Error::TypeError(format!(
                    "expected bool-compatible i64 (0 or 1), got {}",
                    other
                ))),
            },
            Value::Null => Err(crate::Error::TypeError("NULL value for bool".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected bool, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for f64 {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::F64(v) => Ok(*v),
            Value::Null => Err(crate::Error::TypeError("NULL value for f64".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected f64, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for f32 {
    fn from_value(value: &Value) -> Result<Self> {
        let value = match value {
            Value::F64(v) if v.is_finite() => *v,
            Value::I32(v) => *v as f64,
            Value::I64(v) => *v as f64,
            Value::Null => return Err(crate::Error::TypeError("NULL value for f32".to_string())),
            _ => {
                return Err(crate::Error::TypeError(format!(
                    "expected f32-compatible number, got {:?}",
                    value
                )));
            }
        };

        if value < f32::MIN as f64 || value > f32::MAX as f64 {
            return Err(crate::Error::TypeError(format!(
                "number {} does not fit in f32",
                value
            )));
        }

        Ok(value as f32)
    }
}

impl FromValue for rust_decimal::Decimal {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Decimal(v) => Ok(*v),
            Value::String(v) => v.parse::<rust_decimal::Decimal>().map_err(|e| {
                crate::Error::TypeError(format!(
                    "failed to parse Decimal from string {:?}: {}",
                    v, e
                ))
            }),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for Decimal".to_string(),
            )),
            _ => Err(crate::Error::TypeError(format!(
                "expected Decimal, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for chrono::NaiveDateTime {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::DateTime(v) => Ok(*v),
            Value::String(v) => crate::value::parse_datetime(v).ok_or_else(|| {
                crate::Error::TypeError(format!("failed to parse DateTime from string {:?}", v))
            }),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for DateTime".to_string(),
            )),
            _ => Err(crate::Error::TypeError(format!(
                "expected DateTime, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for uuid::Uuid {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Uuid(v) => Ok(*v),
            Value::String(v) => uuid::Uuid::parse_str(v).map_err(|e| {
                crate::Error::TypeError(format!("failed to parse Uuid from string {:?}: {}", v, e))
            }),
            Value::Null => Err(crate::Error::TypeError("NULL value for Uuid".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected Uuid, got {:?}",
                value
            ))),
        }
    }
}

impl FromValue for serde_json::Value {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Null => Err(crate::Error::TypeError("NULL value for Json".to_string())),
            other => Ok(other.to_json_plain()),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Null => Err(crate::Error::TypeError("NULL value for Json".to_string())),
            other => Ok(other.to_json_plain()),
        }
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Bytes(v) => Ok(v.clone()),
            Value::Null => Err(crate::Error::TypeError("NULL value for Bytes".to_string())),
            _ => Err(crate::Error::TypeError(format!(
                "expected Bytes, got {:?}",
                value
            ))),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Bytes(v) => Ok(v),
            Value::Null => Err(crate::Error::TypeError("NULL value for Bytes".to_string())),
            other => Err(crate::Error::TypeError(format!(
                "expected Bytes, got {:?}",
                other
            ))),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Null => Ok(None),
            _ => T::from_value(value).map(Some),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Null => Ok(None),
            _ => T::from_value_owned(value).map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FromValue, Value};

    #[test]
    fn uuid_and_decimal_scalars_accept_string_values() {
        let uuid = uuid::Uuid::from_value(&Value::String(
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
        ))
        .unwrap();
        let decimal =
            rust_decimal::Decimal::from_value(&Value::String("12.34".to_string())).unwrap();

        assert_eq!(uuid.to_string(), "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(decimal.to_string(), "12.34");
    }
}
