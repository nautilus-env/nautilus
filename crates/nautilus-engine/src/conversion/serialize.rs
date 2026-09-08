//! Direct serialization of decoded rows into the JSON the client receives.

use nautilus_connector::Row;
use nautilus_core::PlainValueRef;
use nautilus_protocol::ProtocolError;

/// Newtype wrapper used by [`rows_to_raw_json`] to serialize a [`Row`] as a
/// JSON object without cloning any column name strings.
struct RowRef<'a>(&'a Row);

impl serde::Serialize for RowRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in self.0.iter() {
            map.serialize_entry(name, &PlainValueRef(value))?;
        }
        map.end()
    }
}

/// Newtype wrapper used by [`rows_to_raw_json`] to serialize a slice of [`Row`]s
/// as a JSON array using the SIMD-accelerated `sonic-rs` serializer.
struct RowsRef<'a>(&'a [Row]);

impl serde::Serialize for RowsRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for row in self.0 {
            seq.serialize_element(&RowRef(row))?;
        }
        seq.end()
    }
}

/// Serialize database rows directly to a `Box<RawValue>` JSON array, bypassing
/// all intermediate `Map` / `Vec<JsonValue>` allocations.
///
/// Column names are written as `&str` references — no `.to_string()` cloning.
/// Uses SIMD-accelerated `sonic-rs` for the serialization pass.
pub fn rows_to_raw_json(rows: &[Row]) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let mut buf = Vec::with_capacity(rows.len().saturating_add(1) * 64);
    sonic_rs::to_writer(&mut buf, &RowsRef(rows))
        .map_err(|e| ProtocolError::Internal(format!("Serialize error: {}", e)))?;
    let s = String::from_utf8(buf)
        .map_err(|e| ProtocolError::Internal(format!("UTF-8 error: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("RawValue error: {}", e)))
}
