//! What the language offers, described once for every reader.
//!
//! Completion and hover answer about the same scalar types, so the label, the
//! SQL a type maps to, the extension it needs and the sentence that describes
//! it live here rather than in two lists that drift apart. Provider support is
//! not restated: it is read from [`ScalarType::supported_by`], the same answer
//! validation gives.
//!
//! This is a description of syntax and documentation. It carries no relation
//! rules and no SQL generation.

use crate::ast::FieldType;
use crate::ir::{DatabaseProvider, ScalarType};

/// One scalar type as the language surfaces it.
pub(super) struct ScalarTypeDoc {
    /// The name written in a schema, and the completion label.
    pub label: &'static str,
    /// LSP snippet inserted instead of the label, for a type with arguments.
    pub snippet: Option<&'static str>,
    /// The IR type this entry describes, with placeholder arguments where the
    /// written form takes them.
    pub scalar: ScalarType,
    /// The SQL type it maps to.
    pub sql: &'static str,
    /// The PostgreSQL extension it requires, when it requires one.
    pub extension: Option<&'static str>,
    /// What the type holds, as a noun phrase.
    pub summary: &'static str,
    /// The mapping sentence for hover, when the SQL type differs per provider
    /// and the single `sql` name cannot say so.
    pub mapping: Option<&'static str>,
}

impl ScalarTypeDoc {
    /// Whether a datasource with this provider can store the type.
    ///
    /// An unknown or absent provider offers everything: the schema being
    /// edited may not have named its datasource yet.
    pub(super) fn supported_by(&self, provider: Option<&str>) -> bool {
        let Some(provider) = provider else {
            return true;
        };
        match provider.parse::<DatabaseProvider>() {
            Ok(provider) => self.scalar.supported_by(provider),
            Err(_) => true,
        }
    }

    /// The one-line detail shown next to a completion item.
    pub(super) fn detail(&self) -> String {
        format!("{} -> {}{}", self.summary, self.sql, self.availability())
    }

    /// The documentation shown on hover.
    pub(super) fn documentation(&self) -> String {
        let mapping = self
            .mapping
            .map(str::to_string)
            .unwrap_or_else(|| format!("Maps to {}.", quoted_sql(self.sql)));
        let mut text = format!("{}. {}", self.summary, mapping);
        if let Some(extension) = self.extension {
            text.push_str(&format!(
                " PostgreSQL only, and requires the `{}` extension.",
                extension
            ));
        } else if !self.scalar.supported_by(DatabaseProvider::Sqlite) {
            text.push_str(&format!(" {}.", self.scalar.supported_providers()));
        }
        text
    }

    /// The provider note appended to the completion detail.
    fn availability(&self) -> String {
        match self.extension {
            Some(extension) => format!(" (PostgreSQL + {} extension)", extension),
            None if !self.scalar.supported_by(DatabaseProvider::Sqlite) => {
                format!(" ({})", self.scalar.supported_providers())
            }
            None => String::new(),
        }
    }
}

/// Render a SQL type name, or a `A / B` pair of them, as inline code.
fn quoted_sql(sql: &str) -> String {
    sql.split(" / ")
        .map(|name| format!("`{}`", name))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Every scalar type the language accepts, in the order completion offers them.
pub(super) const SCALAR_TYPES: &[ScalarTypeDoc] = &[
    ScalarTypeDoc {
        label: "String",
        snippet: None,
        scalar: ScalarType::String,
        sql: "VARCHAR / TEXT",
        extension: None,
        mapping: None,
        summary: "UTF-8 text",
    },
    ScalarTypeDoc {
        label: "Boolean",
        snippet: None,
        scalar: ScalarType::Boolean,
        sql: "BOOLEAN",
        extension: None,
        mapping: None,
        summary: "true / false",
    },
    ScalarTypeDoc {
        label: "Int",
        snippet: None,
        scalar: ScalarType::Int,
        sql: "INTEGER",
        extension: None,
        mapping: None,
        summary: "32-bit integer",
    },
    ScalarTypeDoc {
        label: "BigInt",
        snippet: None,
        scalar: ScalarType::BigInt,
        sql: "BIGINT",
        extension: None,
        mapping: None,
        summary: "64-bit integer",
    },
    ScalarTypeDoc {
        label: "Float",
        snippet: None,
        scalar: ScalarType::Float,
        sql: "DOUBLE PRECISION",
        extension: None,
        mapping: None,
        summary: "64-bit float",
    },
    ScalarTypeDoc {
        label: "Decimal",
        snippet: None,
        scalar: ScalarType::Decimal {
            precision: 0,
            scale: 0,
        },
        sql: "NUMERIC",
        extension: None,
        mapping: None,
        summary: "Exact decimal",
    },
    ScalarTypeDoc {
        label: "DateTime",
        snippet: None,
        scalar: ScalarType::DateTime,
        sql: "TIMESTAMPTZ",
        extension: None,
        mapping: None,
        summary: "Timestamp with time zone",
    },
    ScalarTypeDoc {
        label: "Bytes",
        snippet: None,
        scalar: ScalarType::Bytes,
        sql: "BYTEA",
        extension: None,
        mapping: Some("Maps to `BYTEA` on PostgreSQL and `BLOB` on MySQL and SQLite."),
        summary: "Binary data",
    },
    ScalarTypeDoc {
        label: "Json",
        snippet: None,
        scalar: ScalarType::Json,
        sql: "JSONB",
        extension: None,
        mapping: Some("Maps to `JSONB` on PostgreSQL and `JSON` on MySQL and SQLite."),
        summary: "JSON document",
    },
    ScalarTypeDoc {
        label: "Uuid",
        snippet: None,
        scalar: ScalarType::Uuid,
        sql: "UUID",
        extension: None,
        mapping: None,
        summary: "UUID",
    },
    ScalarTypeDoc {
        label: "Citext",
        snippet: None,
        scalar: ScalarType::Citext,
        sql: "CITEXT",
        extension: Some("citext"),
        mapping: None,
        summary: "Case-insensitive text",
    },
    ScalarTypeDoc {
        label: "Hstore",
        snippet: None,
        scalar: ScalarType::Hstore,
        sql: "HSTORE",
        extension: Some("hstore"),
        mapping: None,
        summary: "Key/value text map",
    },
    ScalarTypeDoc {
        label: "Ltree",
        snippet: None,
        scalar: ScalarType::Ltree,
        sql: "LTREE",
        extension: Some("ltree"),
        mapping: None,
        summary: "Label tree path",
    },
    ScalarTypeDoc {
        label: "Geometry",
        snippet: None,
        scalar: ScalarType::Geometry,
        sql: "GEOMETRY",
        extension: Some("postgis"),
        mapping: None,
        summary: "Planar spatial value",
    },
    ScalarTypeDoc {
        label: "Geography",
        snippet: None,
        scalar: ScalarType::Geography,
        sql: "GEOGRAPHY",
        extension: Some("postgis"),
        mapping: None,
        summary: "Geodetic spatial value",
    },
    ScalarTypeDoc {
        label: "Vector(dim)",
        snippet: Some("Vector(${1:1536})"),
        scalar: ScalarType::Vector { dimension: 0 },
        sql: "VECTOR(dim)",
        extension: Some("vector"),
        mapping: None,
        summary: "Dense embedding vector",
    },
    ScalarTypeDoc {
        label: "Jsonb",
        snippet: None,
        scalar: ScalarType::Jsonb,
        sql: "JSONB",
        extension: None,
        mapping: None,
        summary: "JSONB document",
    },
    ScalarTypeDoc {
        label: "Xml",
        snippet: None,
        scalar: ScalarType::Xml,
        sql: "XML",
        extension: None,
        mapping: None,
        summary: "XML document",
    },
    ScalarTypeDoc {
        label: "Char(n)",
        snippet: Some("Char(${1:n})"),
        scalar: ScalarType::Char { length: 0 },
        sql: "CHAR(n)",
        extension: None,
        mapping: None,
        summary: "Fixed-length string",
    },
    ScalarTypeDoc {
        label: "VarChar(n)",
        snippet: Some("VarChar(${1:n})"),
        scalar: ScalarType::VarChar { length: 0 },
        sql: "VARCHAR(n)",
        extension: None,
        mapping: None,
        summary: "Variable-length string",
    },
];

/// The catalog entry describing a parsed field type, when it names a scalar.
pub(super) fn scalar_doc(field_type: &FieldType) -> Option<&'static ScalarTypeDoc> {
    let label = match field_type {
        FieldType::String => "String",
        FieldType::Boolean => "Boolean",
        FieldType::Int => "Int",
        FieldType::BigInt => "BigInt",
        FieldType::Float => "Float",
        FieldType::Decimal { .. } => "Decimal",
        FieldType::DateTime => "DateTime",
        FieldType::Bytes => "Bytes",
        FieldType::Json => "Json",
        FieldType::Uuid => "Uuid",
        FieldType::Citext => "Citext",
        FieldType::Hstore => "Hstore",
        FieldType::Ltree => "Ltree",
        FieldType::Geometry => "Geometry",
        FieldType::Geography => "Geography",
        FieldType::Vector { .. } => "Vector(dim)",
        FieldType::Jsonb => "Jsonb",
        FieldType::Xml => "Xml",
        FieldType::Char { .. } => "Char(n)",
        FieldType::VarChar { .. } => "VarChar(n)",
        FieldType::UserType(_) => return None,
    };
    SCALAR_TYPES.iter().find(|doc| doc.label == label)
}
