//! Internal database values, with distinct tagged serde and plain JSON representations.

mod conversions;
mod plain;
mod scalar_text;
mod tagged;
mod wrappers;

#[cfg(test)]
mod test_values;

use std::collections::BTreeMap;

pub(crate) use plain::json_to_value_ref;
pub use plain::PlainValueRef;
pub use scalar_text::parse_datetime;
pub use wrappers::{Geography, Geometry};

/// Database value with a tagged serde representation that preserves its variant.
///
/// [`Value::to_json_plain`] and [`PlainValueRef`] expose the untagged wire shape.
/// Both formats encode decimals as strings to avoid precision loss, datetimes
/// as RFC3339 strings, UUIDs as lowercase hyphenated strings, and bytes as base64.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// NULL value.
    Null,
    /// Boolean.
    Bool(bool),
    /// 32-bit integer.
    I32(i32),
    /// 64-bit integer.
    I64(i64),
    /// 64-bit float.
    F64(f64),
    /// Fixed-precision decimal number.
    Decimal(rust_decimal::Decimal),
    /// Date and time (without timezone).
    DateTime(chrono::NaiveDateTime),
    /// UUID.
    Uuid(uuid::Uuid),
    /// JSON value.
    Json(serde_json::Value),
    /// PostgreSQL hstore key/value map.
    Hstore(BTreeMap<String, Option<String>>),
    /// PostgreSQL PostGIS geometry value.
    Geometry(String),
    /// PostgreSQL PostGIS geography value.
    Geography(String),
    /// PostgreSQL pgvector dense embedding vector.
    Vector(Vec<f32>),
    /// String.
    String(String),
    /// Byte array.
    Bytes(Vec<u8>),
    /// Array of values (PostgreSQL native arrays).
    Array(Vec<Value>),
    /// 2D array of values (PostgreSQL multi-dimensional arrays).
    Array2D(Vec<Vec<Value>>),
    /// A text-backed PostgreSQL extension scalar with its type name.
    ///
    /// Carries the textual value together with the lowercase type name
    /// (e.g. `"citext"`, `"ltree"`) for the same reason as [`Value::Enum`]:
    /// without an explicit `$1::citext`, PostgreSQL resolves a comparison
    /// against a `citext` column as `text = text` and compares case
    /// sensitively, silently defeating the point of the type.
    /// All other backends treat this identically to `Value::String`.
    Extension {
        /// The textual value sent to / received from the DB.
        value: String,
        /// Lowercase PostgreSQL type name (e.g. `"citext"`).
        type_name: String,
    },
    /// A database enum value with its PostgreSQL type name.
    ///
    /// Carries the variant string (e.g. `"ADMIN"`) together with the
    /// lowercase PG type name (e.g. `"role"`) so that the PostgreSQL
    /// dialect can emit the required explicit cast (`$1::role`).
    /// All other backends treat this identically to `Value::String`.
    Enum {
        /// The enum variant string sent to / received from the DB.
        value: String,
        /// Lowercase PostgreSQL type name (e.g. `"role"`, `"poststatus"`).
        type_name: String,
    },
    /// A PostgreSQL native composite type value.
    ///
    /// Carries the lowercase PG type name (e.g. `"championstats"`) together
    /// with the field values in their declared order. The PostgreSQL dialect
    /// emits the required explicit cast (`$1::championstats`) and the connector
    /// encodes the fields as a record literal (`("0","0",…)`). Backends that
    /// store composites as JSON never receive this variant.
    Composite {
        /// Lowercase PostgreSQL type name (e.g. `"championstats"`).
        type_name: String,
        /// Field values in the composite type's declared order.
        fields: Vec<Value>,
    },
}
