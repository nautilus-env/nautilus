//! Conversion of JSON input into the PostgreSQL extension types the schema can
//! declare: `hstore`, `vector`, `geometry` and `geography`.
//!
//! Each converter accepts the scalar form and the array form of its type, and
//! rejects anything else with the shape it expected.

use std::collections::BTreeMap;

use nautilus_core::Value;
use nautilus_protocol::ProtocolError;

pub(super) fn json_to_hstore_value(json: &serde_json::Value) -> Result<Value, ProtocolError> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Object(object) => Ok(Value::Hstore(json_object_to_hstore(object)?)),
        serde_json::Value::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(match item {
                    serde_json::Value::Null => Value::Null,
                    serde_json::Value::Object(object) => {
                        Value::Hstore(json_object_to_hstore(object)?)
                    }
                    other => {
                        return Err(ProtocolError::InvalidParams(format!(
                            "Hstore arrays must contain only objects or nulls, got {:?}",
                            other
                        )));
                    }
                });
            }
            Ok(Value::Array(values))
        }
        other => Err(ProtocolError::InvalidParams(format!(
            "Hstore values must be JSON objects with string or null values, got {:?}",
            other
        ))),
    }
}

fn json_object_to_hstore(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<BTreeMap<String, Option<String>>, ProtocolError> {
    let mut decoded = BTreeMap::new();
    for (key, value) in object {
        let mapped = match value {
            serde_json::Value::String(item) => Some(item.clone()),
            serde_json::Value::Null => None,
            other => {
                return Err(ProtocolError::InvalidParams(format!(
                    "Hstore values must be strings or nulls; key {:?} received {:?}",
                    key, other
                )));
            }
        };
        decoded.insert(key.clone(), mapped);
    }
    Ok(decoded)
}

pub(super) fn json_to_vector_value(
    json: &serde_json::Value,
    dimension: u32,
) -> Result<Value, ProtocolError> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Array(items) => {
            if items.len() != dimension as usize {
                return Err(ProtocolError::InvalidParams(format!(
                    "Vector value has {} dimensions but schema requires {}",
                    items.len(),
                    dimension
                )));
            }
            let mut values = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let Some(value) = item.as_f64() else {
                    return Err(ProtocolError::InvalidParams(format!(
                        "Vector values must be arrays of finite numbers; element {} was {:?}",
                        idx, item
                    )));
                };
                if !value.is_finite() || value < f32::MIN as f64 || value > f32::MAX as f64 {
                    return Err(ProtocolError::InvalidParams(format!(
                        "Vector element {} is outside the finite f32 range: {}",
                        idx, value
                    )));
                }
                values.push(value as f32);
            }
            Ok(Value::Vector(values))
        }
        other => Err(ProtocolError::InvalidParams(format!(
            "Vector values must be arrays of finite numbers, got {:?}",
            other
        ))),
    }
}

pub(super) fn json_to_geometry_value(json: &serde_json::Value) -> Result<Value, ProtocolError> {
    json_to_spatial_value(json, "Geometry", Value::Geometry)
}

pub(super) fn json_to_geography_value(json: &serde_json::Value) -> Result<Value, ProtocolError> {
    json_to_spatial_value(json, "Geography", Value::Geography)
}

fn json_to_spatial_value(
    json: &serde_json::Value,
    type_name: &str,
    wrap: fn(String) -> Value,
) -> Result<Value, ProtocolError> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::String(raw) => Ok(wrap(raw.clone())),
        serde_json::Value::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let Some(raw) = item.as_str() else {
                    return Err(ProtocolError::InvalidParams(format!(
                        "{} arrays must contain only strings; element {} was {:?}",
                        type_name, idx, item
                    )));
                };
                values.push(wrap(raw.to_string()));
            }
            Ok(Value::Array(values))
        }
        other => Err(ProtocolError::InvalidParams(format!(
            "{} values must be strings containing WKT/EWKT or EWKB hex, got {:?}",
            type_name, other
        ))),
    }
}
