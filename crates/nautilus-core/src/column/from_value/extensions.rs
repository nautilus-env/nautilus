//! Native and JSON/text decoding for hstore and PostGIS scalars.

use std::collections::BTreeMap;

use super::FromValue;
use crate::{Geography, Geometry, Result, Value};

impl FromValue for BTreeMap<String, Option<String>> {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Hstore(map) => Ok(map.clone()),
            Value::Json(serde_json::Value::Object(map)) => decode_hstore_json_object(map),
            Value::Null => Err(crate::Error::TypeError("NULL value for Hstore".to_string())),
            other => Err(crate::Error::TypeError(format!(
                "expected Hstore or Json object, got {:?}",
                other
            ))),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Hstore(map) => Ok(map),
            Value::Json(serde_json::Value::Object(map)) => decode_hstore_json_object(&map),
            Value::Null => Err(crate::Error::TypeError("NULL value for Hstore".to_string())),
            other => Err(crate::Error::TypeError(format!(
                "expected Hstore or Json object, got {:?}",
                other
            ))),
        }
    }
}

impl FromValue for Geometry {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Geometry(v) | Value::String(v) => Ok(Geometry::new(v.clone())),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for Geometry".to_string(),
            )),
            other => Err(crate::Error::TypeError(format!(
                "expected Geometry or String, got {:?}",
                other
            ))),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Geometry(v) | Value::String(v) => Ok(Geometry::new(v)),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for Geometry".to_string(),
            )),
            other => Err(crate::Error::TypeError(format!(
                "expected Geometry or String, got {:?}",
                other
            ))),
        }
    }
}

impl FromValue for Geography {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Geography(v) | Value::String(v) => Ok(Geography::new(v.clone())),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for Geography".to_string(),
            )),
            other => Err(crate::Error::TypeError(format!(
                "expected Geography or String, got {:?}",
                other
            ))),
        }
    }

    fn from_value_owned(value: Value) -> Result<Self> {
        match value {
            Value::Geography(v) | Value::String(v) => Ok(Geography::new(v)),
            Value::Null => Err(crate::Error::TypeError(
                "NULL value for Geography".to_string(),
            )),
            other => Err(crate::Error::TypeError(format!(
                "expected Geography or String, got {:?}",
                other
            ))),
        }
    }
}

fn decode_hstore_json_object(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<BTreeMap<String, Option<String>>> {
    let mut decoded = BTreeMap::new();
    for (key, value) in object {
        let mapped = match value {
            serde_json::Value::String(item) => Some(item.clone()),
            serde_json::Value::Null => None,
            other => {
                return Err(crate::Error::TypeError(format!(
                    "expected Hstore JSON value to be string or null for key {:?}, got {:?}",
                    key, other
                )));
            }
        };
        decoded.insert(key.clone(), mapped);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FromValue, Value};

    #[test]
    fn hstore_scalar_accepts_native_and_json_object_values() {
        let native = Value::Hstore(BTreeMap::from([
            ("display_name".to_string(), Some("Bob".to_string())),
            ("nickname".to_string(), None),
        ]));
        let json = Value::Json(serde_json::json!({
            "display_name": "Bob",
            "nickname": null
        }));

        let native_map = BTreeMap::<String, Option<String>>::from_value(&native).unwrap();
        let json_map = BTreeMap::<String, Option<String>>::from_value(&json).unwrap();

        assert_eq!(native_map, json_map);
        assert_eq!(native_map["display_name"], Some("Bob".to_string()));
        assert_eq!(native_map["nickname"], None);
    }
}
