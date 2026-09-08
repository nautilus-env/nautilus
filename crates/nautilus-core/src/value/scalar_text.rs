//! Scalar text shared by the tagged and plain codecs.

use serde::{Serialize, Serializer};

/// UTC spelling shared by owned and borrowed datetime serialization.
const DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.fZ";

pub(super) fn format_datetime(value: chrono::NaiveDateTime) -> String {
    value.format(DATETIME_FORMAT).to_string()
}

/// Parse the datetime spellings accepted across the stack: RFC-3339, or an
/// ISO-8601 date and time separated by `T` or a space, with an optional
/// fractional part.
///
/// Both the wire codec and the schema-aware row normalizers read the same
/// spellings, so this is the single place that decides which ones are valid.
pub fn parse_datetime(raw: &str) -> Option<chrono::NaiveDateTime> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|value| value.naive_utc())
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f"))
        .ok()
}

/// Serializes a `Display` value as a JSON string without an intermediate `String`.
pub(super) struct DisplayString<T>(pub(super) T);

impl<T: std::fmt::Display> Serialize for DisplayString<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

/// Serializes a datetime in the wire format of [`format_datetime`] without an
/// intermediate `String`.
pub(super) struct DateTimeString(pub(super) chrono::NaiveDateTime);

impl Serialize for DateTimeString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0.format(DATETIME_FORMAT))
    }
}

/// Serializes bytes as a standard-alphabet base64 string without an
/// intermediate `String`.
pub(super) struct Base64String<'a>(pub(super) &'a [u8]);

impl Serialize for Base64String<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&base64::display::Base64Display::new(
            self.0,
            &base64::engine::general_purpose::STANDARD,
        ))
    }
}
