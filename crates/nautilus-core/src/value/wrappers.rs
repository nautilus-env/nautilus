//! Textual PostGIS scalar wrappers.

use serde::{Deserialize, Serialize};

/// A textual scalar whose serde representation stays a plain string.
macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident, $what:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Create a ", $what, " value from its textual representation.")]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Borrow the underlying textual representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the wrapper and return the underlying textual representation.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_newtype!(
    /// PostGIS `geometry` value represented as textual WKT/EWKT or EWKB hex.
    Geometry,
    "geometry"
);

string_newtype!(
    /// PostGIS `geography` value represented as textual WKT/EWKT or EWKB hex.
    Geography,
    "geography"
);
