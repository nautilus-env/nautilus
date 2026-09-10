//! Borrowed and owned decoding of database values into Rust types.

mod collections;
mod extensions;
mod scalars;

use crate::{Result, Value};

/// Decodes one [`Value`] into a Rust type.
///
/// Implemented per scalar, collection and extension type; the selection API
/// calls it once per column while deserializing a row.
pub trait FromValue: Sized {
    /// Convert a Value reference to this type.
    ///
    /// Returns an error if the value is NULL, has the wrong type,
    /// or cannot be converted.
    fn from_value(value: &Value) -> Result<Self>;

    /// Convert an owned Value to this type, avoiding clones when possible.
    ///
    /// The default delegates to `from_value`. Implementations can override
    /// this to consume heap data, as strings and bytes do.
    fn from_value_owned(value: Value) -> Result<Self> {
        Self::from_value(&value)
    }
}

/// Opt-in marker for scalar wrappers a generated client defines itself.
///
/// Generated clients declare their own types for the PostgreSQL extension
/// scalars (pgvector, PostGIS, citext, hstore, ltree). The orphan rule forbids
/// *them* from implementing [`FromValue`] for `Vec<Wrapper>` or converting one
/// into a [`Value`], because both `Vec` and `Value` are foreign to them — so
/// the array conversions live here instead, keyed on this marker.
///
/// Implement it on a wrapper that already round-trips as a single value and
/// `Vec<Wrapper>` starts decoding from `Value::Array` (or a JSON array on
/// MySQL and SQLite) and encoding back into one.
pub trait ExtensionScalar: FromValue + Into<Value> + Clone {}
