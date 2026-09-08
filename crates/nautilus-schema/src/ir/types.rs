//! The type of a field once it is resolved: a scalar, an enum, a composite
//! type or a relation.

use super::{DatabaseProvider, RelationIr};

/// Resolved field type after validation.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedFieldType {
    /// A scalar type (String, Int, etc.).
    Scalar(ScalarType),
    /// An enum type with the enum's logical name.
    Enum {
        /// The logical name of the enum.
        enum_name: String,
        /// The enum's variants, in declaration order.
        ///
        /// Carried on the type because MySQL renders an enum column as a
        /// native `ENUM(...)`, and the DDL for a single column has to be
        /// derivable from the field alone.
        variants: Vec<String>,
    },
    /// A relation to another model.
    Relation(RelationIr),
    /// A composite type (embedded struct).
    CompositeType {
        /// The logical name of the composite type (used for generated code).
        type_name: String,
        /// The physical SQL type name (`@@map` value or lowercased logical name).
        db_name: String,
    },
}

/// Scalar type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    /// UTF-8 string type.
    String,
    /// Boolean type (true/false).
    Boolean,
    /// 32-bit integer.
    Int,
    /// 64-bit integer.
    BigInt,
    /// 64-bit floating point.
    Float,
    /// Fixed-precision decimal number.
    Decimal {
        /// Number of total digits.
        precision: u32,
        /// Number of digits after decimal point.
        scale: u32,
    },
    /// Date and time.
    DateTime,
    /// Binary data.
    Bytes,
    /// JSON value.
    Json,
    /// UUID value.
    Uuid,
    /// Case-insensitive text value (PostgreSQL + citext extension).
    Citext,
    /// Key/value string map (PostgreSQL + hstore extension).
    Hstore,
    /// Label tree path (PostgreSQL + ltree extension).
    Ltree,
    /// Planar spatial value (PostgreSQL + PostGIS extension).
    Geometry,
    /// Geodetic spatial value (PostgreSQL + PostGIS extension).
    Geography,
    /// Dense embedding vector (PostgreSQL + pgvector `vector` extension).
    Vector {
        /// Number of vector dimensions.
        dimension: u32,
    },
    /// JSONB value (PostgreSQL only).
    Jsonb,
    /// XML value (PostgreSQL only).
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
}

impl ScalarType {
    /// Returns `true` when this scalar type is supported by the given database provider.
    pub fn supported_by(self, provider: DatabaseProvider) -> bool {
        match self {
            ScalarType::Citext
            | ScalarType::Hstore
            | ScalarType::Ltree
            | ScalarType::Geometry
            | ScalarType::Geography
            | ScalarType::Vector { .. }
            | ScalarType::Jsonb
            | ScalarType::Xml => provider == DatabaseProvider::Postgres,
            ScalarType::Char { .. } | ScalarType::VarChar { .. } => {
                matches!(
                    provider,
                    DatabaseProvider::Postgres | DatabaseProvider::Mysql
                )
            }
            _ => true,
        }
    }

    /// Human-readable list of supported providers (for diagnostics).
    pub fn supported_providers(self) -> &'static str {
        match self {
            ScalarType::Citext
            | ScalarType::Hstore
            | ScalarType::Ltree
            | ScalarType::Geometry
            | ScalarType::Geography
            | ScalarType::Vector { .. }
            | ScalarType::Jsonb
            | ScalarType::Xml => "PostgreSQL only",
            ScalarType::Char { .. } | ScalarType::VarChar { .. } => "PostgreSQL and MySQL",
            _ => "all databases",
        }
    }

    /// Returns `true` when this scalar is a pgvector `Vector(dim)`.
    pub fn is_vector(self) -> bool {
        matches!(self, ScalarType::Vector { .. })
    }

    /// Returns `true` when this scalar is a PostGIS spatial type.
    pub fn is_postgis(self) -> bool {
        matches!(self, ScalarType::Geometry | ScalarType::Geography)
    }
}
