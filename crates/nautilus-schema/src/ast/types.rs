//! The type written after a field's name, and how that field is stored.

use std::fmt;

/// A field type in a model.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    /// String type.
    String,
    /// Boolean type.
    Boolean,
    /// Int type (32-bit).
    Int,
    /// BigInt type (64-bit).
    BigInt,
    /// Float type.
    Float,
    /// Decimal type with precision and scale.
    Decimal {
        /// Precision (total digits).
        precision: u32,
        /// Scale (digits after decimal point).
        scale: u32,
    },
    /// DateTime type.
    DateTime,
    /// Bytes type.
    Bytes,
    /// JSON type.
    Json,
    /// UUID type.
    Uuid,
    /// Case-insensitive text type (PostgreSQL + citext extension).
    Citext,
    /// Key/value string map type (PostgreSQL + hstore extension).
    Hstore,
    /// Label tree path type (PostgreSQL + ltree extension).
    Ltree,
    /// Planar spatial value (PostgreSQL + PostGIS extension).
    Geometry,
    /// Geodetic spatial value (PostgreSQL + PostGIS extension).
    Geography,
    /// Dense embedding vector type (PostgreSQL + pgvector `vector` extension).
    Vector {
        /// Number of vector dimensions.
        dimension: u32,
    },
    /// JSONB type (PostgreSQL only).
    Jsonb,
    /// XML type (PostgreSQL only).
    Xml,
    /// Fixed-length character type.
    Char {
        /// Column length.
        length: u32,
    },
    /// Variable-length character type.
    VarChar {
        /// Maximum column length.
        length: u32,
    },
    /// User-defined type (model or enum reference).
    UserType(String),
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldType::String => write!(f, "String"),
            FieldType::Boolean => write!(f, "Boolean"),
            FieldType::Int => write!(f, "Int"),
            FieldType::BigInt => write!(f, "BigInt"),
            FieldType::Float => write!(f, "Float"),
            FieldType::Decimal { precision, scale } => {
                write!(f, "Decimal({}, {})", precision, scale)
            }
            FieldType::DateTime => write!(f, "DateTime"),
            FieldType::Bytes => write!(f, "Bytes"),
            FieldType::Json => write!(f, "Json"),
            FieldType::Uuid => write!(f, "Uuid"),
            FieldType::Citext => write!(f, "Citext"),
            FieldType::Hstore => write!(f, "Hstore"),
            FieldType::Ltree => write!(f, "Ltree"),
            FieldType::Geometry => write!(f, "Geometry"),
            FieldType::Geography => write!(f, "Geography"),
            FieldType::Vector { dimension } => write!(f, "Vector({})", dimension),
            FieldType::Jsonb => write!(f, "Jsonb"),
            FieldType::Xml => write!(f, "Xml"),
            FieldType::Char { length } => write!(f, "Char({})", length),
            FieldType::VarChar { length } => write!(f, "VarChar({})", length),
            FieldType::UserType(name) => write!(f, "{}", name),
        }
    }
}

/// Storage strategy for array fields on databases without native array support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageStrategy {
    /// Native database array type (PostgreSQL).
    Native,
    /// JSON-serialized array storage (MySQL, SQLite).
    Json,
}

/// Whether a computed column is physically stored or computed on every read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputedKind {
    /// Column value persisted on disk (PostgreSQL, MySQL, SQLite).
    Stored,
    /// Column value computed on read, not stored (MySQL and SQLite only).
    Virtual,
}
