//! JSON request values converted into the internal [`Value`] type.
//!
//! Conversion is schema-aware where the declared field type changes how the
//! value must be bound (enums, decimals, datetimes, extension types); the
//! untyped entry point stays available for paths without a field type.

use std::str::FromStr;

use uuid::Uuid;

use nautilus_core::{parse_datetime, Value};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{ResolvedFieldType, ScalarType};

use super::extension::{
    json_to_geography_value, json_to_geometry_value, json_to_hstore_value, json_to_vector_value,
};

/// Speculatively detect UUID-format strings so they bind correctly as
/// [`uuid::Uuid`] against UUID columns (required for PostgreSQL prepared
/// statements).
///
/// A cheap shape gate (length, hyphen position) skips the parser call for the
/// vast majority of non-UUID strings. The gated lengths are exactly the four
/// formats `Uuid::parse_str` accepts: simple (32), hyphenated (36), braced
/// (38) and URN (45); every other length fails the full parse anyway.
fn detect_uuid(s: &str) -> Option<Uuid> {
    match s.len() {
        36 if s.as_bytes()[8] != b'-' => None,
        32 | 36 | 38 | 45 => Uuid::parse_str(s).ok(),
        _ => None,
    }
}

/// Convert a JSON value to a [`Value`] for use in queries.
///
/// Handles UUID auto-detection, int32/int64 discrimination, arrays, and objects.
pub fn json_to_value(json: &serde_json::Value) -> Result<Value, ProtocolError> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Ok(Value::I32(i as i32))
                } else {
                    Ok(Value::I64(i))
                }
            } else if let Some(f) = n.as_f64() {
                Ok(Value::F64(f))
            } else {
                Err(ProtocolError::InvalidParams("Invalid number".to_string()))
            }
        }
        serde_json::Value::String(s) => {
            if let Some(u) = detect_uuid(s) {
                Ok(Value::Uuid(u))
            } else {
                Ok(Value::String(s.clone()))
            }
        }
        serde_json::Value::Array(arr) => {
            let values: Result<Vec<Value>, _> = arr.iter().map(json_to_value).collect();
            Ok(Value::Array(values?))
        }
        serde_json::Value::Object(_) => Ok(Value::Json(json.clone())),
    }
}

/// Convert a JSON value to a [`Value`], using schema field-type context to
/// produce [`Value::Enum`] for enum-typed fields and [`Value::DateTime`] for
/// datetime-typed fields.
///
/// For PostgreSQL, enum columns require an explicit `::type_name` cast in
/// parameterised queries.  Wrapping the value in `Value::Enum` lets the
/// dialect layer inject that cast automatically; all other backends treat it
/// identically to `Value::String`.
pub fn json_to_value_field(
    json: &serde_json::Value,
    field_type: &ResolvedFieldType,
) -> Result<Value, ProtocolError> {
    if let ResolvedFieldType::Enum {
        enum_name,
        variants,
    } = field_type
    {
        match json {
            serde_json::Value::Null => return Ok(Value::Null),
            serde_json::Value::String(s) => {
                // SQLite stores an enum as plain text and would accept anything;
                // PostgreSQL and MySQL reject it at the server. Checking here
                // makes the three backends agree and names the offending value.
                if !variants.iter().any(|variant| variant == s) {
                    return Err(ProtocolError::InvalidParams(format!(
                        "'{}' is not a variant of enum {}; expected one of: {}",
                        s,
                        enum_name,
                        variants.join(", ")
                    )));
                }
                return Ok(Value::Enum {
                    value: s.clone(),
                    type_name: enum_name.to_lowercase(),
                });
            }
            _ => {} // fall through to the generic converter below
        }
    }
    // A plain `String` field must never be sniffed into `Value::Uuid`: on
    // PostgreSQL that binds the parameter as `uuid` and a comparison against a
    // `text` column fails with "operator does not exist: text = uuid".
    if let ResolvedFieldType::Scalar(ScalarType::String) = field_type {
        if let serde_json::Value::String(s) = json {
            return Ok(Value::String(s.clone()));
        }
    }
    // The engine returns a `Decimal` as a JSON string, so a row read back and
    // written again arrives as one. PostgreSQL rejects text against `numeric`,
    // hence the explicit parse into `Value::Decimal`.
    if let ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) = field_type {
        if let serde_json::Value::String(s) = json {
            return rust_decimal::Decimal::from_str(s)
                .map(Value::Decimal)
                .map_err(|_| {
                    ProtocolError::InvalidParams(format!("'{}' is not a valid Decimal", s))
                });
        }
    }
    // For DateTime fields, parse ISO-8601 / RFC-3339 strings into
    // Value::DateTime so the connector can bind them with the correct
    // PostgreSQL OID instead of sending an untyped text value.
    if let ResolvedFieldType::Scalar(ScalarType::DateTime) = field_type {
        if let serde_json::Value::String(s) = json {
            if let Some(dt) = parse_datetime(s) {
                return Ok(Value::DateTime(dt));
            }
        }
    }
    if let ResolvedFieldType::Scalar(ScalarType::Hstore) = field_type {
        return json_to_hstore_value(json);
    }
    if let ResolvedFieldType::Scalar(ScalarType::Vector { dimension }) = field_type {
        return json_to_vector_value(json, *dimension);
    }
    if let ResolvedFieldType::Scalar(ScalarType::Geometry) = field_type {
        return json_to_geometry_value(json);
    }
    if let ResolvedFieldType::Scalar(ScalarType::Geography) = field_type {
        return json_to_geography_value(json);
    }
    // citext and ltree arrive as plain JSON strings. Tagging them with their
    // type name lets the PostgreSQL dialect emit `$1::citext`, without which
    // the server resolves the comparison as `text = text` and a citext column
    // stops being case insensitive.
    if let ResolvedFieldType::Scalar(scalar @ (ScalarType::Citext | ScalarType::Ltree)) = field_type
    {
        if let serde_json::Value::String(s) = json {
            let type_name = match scalar {
                ScalarType::Citext => "citext",
                _ => "ltree",
            };
            return Ok(Value::Extension {
                value: s.clone(),
                type_name: type_name.to_string(),
            });
        }
    }
    json_to_value(json)
}

/// Whether a field's declared type makes a JSON object or array a legitimate
/// value rather than a wrapper the engine should interpret.
pub fn holds_structured_json(field_type: &ResolvedFieldType, is_array: bool) -> bool {
    if is_array {
        return true;
    }
    match field_type {
        ResolvedFieldType::Scalar(scalar) => matches!(
            scalar,
            ScalarType::Json
                | ScalarType::Bytes
                | ScalarType::Hstore
                | ScalarType::Vector { .. }
                | ScalarType::Geometry
                | ScalarType::Geography
        ),
        ResolvedFieldType::CompositeType { .. } | ResolvedFieldType::Relation(_) => true,
        ResolvedFieldType::Enum { .. } => false,
    }
}

/// Reject a JSON object or array written to a field that holds a single scalar.
///
/// Without this, the structured value is bound verbatim: SQLite's dynamic
/// typing then stores the JSON text in, say, an `INTEGER` column and every
/// later read of the table fails to decode. The update operators reach this
/// point only where they cannot be applied — an operator object survives the
/// mutation handlers only when the field or the operation refuses it — so they
/// keep a message that names the operator instead of the shape.
pub fn ensure_scalar_input(
    json: &serde_json::Value,
    field_type: &ResolvedFieldType,
    is_array: bool,
    field_name: &str,
) -> Result<(), ProtocolError> {
    if !matches!(
        json,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    ) || holds_structured_json(field_type, is_array)
    {
        return Ok(());
    }

    if let serde_json::Value::Object(object) = json {
        if let Some(op) = object.keys().find(|key| {
            matches!(
                key.as_str(),
                "set" | "increment" | "decrement" | "multiply" | "divide"
            )
        }) {
            return Err(ProtocolError::InvalidParams(format!(
                "Field '{}' does not support the update operator '{}'",
                field_name, op
            )));
        }
    }

    let shape = if json.is_array() {
        "an array"
    } else {
        "an object"
    };
    Err(ProtocolError::InvalidParams(format!(
        "Field '{}' holds a single scalar value but received {}",
        field_name, shape
    )))
}
