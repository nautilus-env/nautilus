//! Classification of a statement's columns, done once per result set.
//!
//! A PostgreSQL row reports its column types by name, and resolving that name
//! is the same work for every row of a statement. The plan built here resolves
//! it once — including the alias table, the element type of an array and the
//! column name — and the decoder reads it per cell.

use sqlx::postgres::{PgRow, PgTypeInfo, PgTypeKind as SqlxPgTypeKind};
use sqlx::{Column, Row as SqlxRow, TypeInfo};

/// Per-statement decode plan: one [`PgColumnDecode`] plus one shared name per
/// column.
///
/// Hoists the work previously done per cell — the composite check, the
/// `classify_pg_type` scan over the case-insensitive alias table (plus the
/// alias chains for array element types) and the column-name `String`
/// allocation — so it runs once per column for the whole result set. Rows
/// reference the names via `Arc` clones.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PgColumnPlan {
    pub(super) kinds: Vec<PgColumnDecode>,
    pub(super) names: Vec<std::sync::Arc<str>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PgColumnDecode {
    Bool,
    Int2,
    Int4,
    Int8,
    Float4,
    Float8,
    Text,
    Geometry,
    Geography,
    Hstore,
    Vector,
    Bytes,
    Uuid,
    Timestamp,
    TimestampTz,
    Date,
    Time,
    Numeric,
    Json,
    Array(PgArrayElem),
    Array2D(String),
    Composite,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PgArrayElem {
    Text,
    Geometry,
    Geography,
    Hstore,
    Int2,
    Int4,
    Int8,
    Float4,
    Float8,
    Bool,
    Unsupported(String),
}

impl PgColumnPlan {
    pub(super) fn for_row(row: &PgRow) -> Self {
        let columns = row.columns();
        Self {
            kinds: columns
                .iter()
                .map(|column| plan_column(column.type_info()))
                .collect(),
            names: columns
                .iter()
                .map(|column| std::sync::Arc::from(column.name()))
                .collect(),
        }
    }
}

pub(super) fn plan_column(type_info: &PgTypeInfo) -> PgColumnDecode {
    if matches!(type_info.kind(), SqlxPgTypeKind::Composite(_)) {
        return PgColumnDecode::Composite;
    }

    plan_column_by_name(type_info.name())
}

fn plan_column_by_name(type_name: &str) -> PgColumnDecode {
    match classify_pg_type(type_name) {
        PgTypeKind::Bool => PgColumnDecode::Bool,
        PgTypeKind::Int2 => PgColumnDecode::Int2,
        PgTypeKind::Int4 => PgColumnDecode::Int4,
        PgTypeKind::Int8 => PgColumnDecode::Int8,
        PgTypeKind::Float4 => PgColumnDecode::Float4,
        PgTypeKind::Float8 => PgColumnDecode::Float8,
        PgTypeKind::Text => PgColumnDecode::Text,
        PgTypeKind::Geometry => PgColumnDecode::Geometry,
        PgTypeKind::Geography => PgColumnDecode::Geography,
        PgTypeKind::Hstore => PgColumnDecode::Hstore,
        PgTypeKind::Vector => PgColumnDecode::Vector,
        PgTypeKind::Bytes => PgColumnDecode::Bytes,
        PgTypeKind::Uuid => PgColumnDecode::Uuid,
        PgTypeKind::Timestamp => PgColumnDecode::Timestamp,
        PgTypeKind::TimestampTz => PgColumnDecode::TimestampTz,
        PgTypeKind::Date => PgColumnDecode::Date,
        PgTypeKind::Time => PgColumnDecode::Time,
        PgTypeKind::Numeric => PgColumnDecode::Numeric,
        PgTypeKind::Json => PgColumnDecode::Json,
        PgTypeKind::Array(element_type) => PgColumnDecode::Array(plan_array_elem(element_type)),
        PgTypeKind::Array2D(element_type) => PgColumnDecode::Array2D(element_type.to_string()),
        PgTypeKind::Unknown => PgColumnDecode::Unknown(type_name.to_string()),
    }
}

fn plan_array_elem(element_type: &str) -> PgArrayElem {
    if matches_pg_type(
        element_type,
        &[
            "TEXT", "VARCHAR", "CHAR", "BPCHAR", "NAME", "CITEXT", "LTREE",
        ],
    ) {
        PgArrayElem::Text
    } else if pg_type_is(element_type, "GEOMETRY") {
        PgArrayElem::Geometry
    } else if pg_type_is(element_type, "GEOGRAPHY") {
        PgArrayElem::Geography
    } else if pg_type_is(element_type, "HSTORE") {
        PgArrayElem::Hstore
    } else if pg_type_is(element_type, "INT2") {
        PgArrayElem::Int2
    } else if matches_pg_type(element_type, &["INT4", "SERIAL"]) {
        PgArrayElem::Int4
    } else if matches_pg_type(element_type, &["INT8", "BIGINT", "BIGSERIAL"]) {
        PgArrayElem::Int8
    } else if matches_pg_type(element_type, &["FLOAT4", "REAL"]) {
        PgArrayElem::Float4
    } else if matches_pg_type(element_type, &["FLOAT8", "DOUBLE PRECISION"]) {
        PgArrayElem::Float8
    } else if pg_type_is(element_type, "BOOL") {
        PgArrayElem::Bool
    } else {
        PgArrayElem::Unsupported(element_type.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PgTypeKind<'a> {
    Bool,
    Int2,
    Int4,
    Int8,
    Float4,
    Float8,
    Text,
    Geometry,
    Geography,
    Hstore,
    Vector,
    Bytes,
    Uuid,
    Timestamp,
    TimestampTz,
    Date,
    Time,
    Numeric,
    Json,
    Array(&'a str),
    Array2D(&'a str),
    Unknown,
}

const PG_SCALAR_TYPE_ALIASES: &[(&[&str], PgTypeKind<'static>)] = &[
    (&["BOOL"], PgTypeKind::Bool),
    (&["INT2"], PgTypeKind::Int2),
    (&["INT4", "SERIAL"], PgTypeKind::Int4),
    (&["INT8", "BIGINT", "BIGSERIAL"], PgTypeKind::Int8),
    (&["FLOAT4", "REAL"], PgTypeKind::Float4),
    (&["FLOAT8", "DOUBLE PRECISION"], PgTypeKind::Float8),
    (
        &[
            "VARCHAR", "TEXT", "CHAR", "BPCHAR", "NAME", "CITEXT", "LTREE",
        ],
        PgTypeKind::Text,
    ),
    (&["GEOMETRY"], PgTypeKind::Geometry),
    (&["GEOGRAPHY"], PgTypeKind::Geography),
    (&["HSTORE"], PgTypeKind::Hstore),
    (&["VECTOR"], PgTypeKind::Vector),
    (&["BYTEA"], PgTypeKind::Bytes),
    (&["UUID"], PgTypeKind::Uuid),
    (&["TIMESTAMP"], PgTypeKind::Timestamp),
    (&["TIMESTAMPTZ"], PgTypeKind::TimestampTz),
    (&["DATE"], PgTypeKind::Date),
    (&["TIME"], PgTypeKind::Time),
    (&["NUMERIC"], PgTypeKind::Numeric),
    (&["JSON", "JSONB"], PgTypeKind::Json),
];

pub(super) fn classify_pg_type(type_name: &str) -> PgTypeKind<'_> {
    match classify_pg_array_type(type_name) {
        Some(kind) => kind,
        None => classify_pg_scalar_type(type_name).unwrap_or(PgTypeKind::Unknown),
    }
}

fn classify_pg_array_type(type_name: &str) -> Option<PgTypeKind<'_>> {
    if let Some(element_type) = type_name.strip_suffix("[][]") {
        Some(PgTypeKind::Array2D(element_type))
    } else {
        type_name.strip_suffix("[]").map(PgTypeKind::Array)
    }
}

fn classify_pg_scalar_type(type_name: &str) -> Option<PgTypeKind<'static>> {
    PG_SCALAR_TYPE_ALIASES
        .iter()
        .find_map(|(aliases, kind)| matches_pg_type(type_name, aliases).then_some(*kind))
}

pub(super) fn pg_type_is(type_name: &str, expected: &str) -> bool {
    type_name.eq_ignore_ascii_case(expected)
}

fn matches_pg_type(type_name: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| pg_type_is(type_name, candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_pg_type_is_case_insensitive_without_normalizing_strings() {
        assert_eq!(classify_pg_type("jsonb"), PgTypeKind::Json);
        assert_eq!(classify_pg_type("TeXt"), PgTypeKind::Text);
        assert_eq!(classify_pg_type("int4[]"), PgTypeKind::Array("int4"));
        assert_eq!(
            classify_pg_type("VaRcHaR[][]"),
            PgTypeKind::Array2D("VaRcHaR")
        );
    }

    #[test]
    fn plan_column_resolves_aliases_and_array_elements_once() {
        assert_eq!(plan_column_by_name("jsonb"), PgColumnDecode::Json);
        assert_eq!(plan_column_by_name("TeXt"), PgColumnDecode::Text);
        assert_eq!(plan_column_by_name("BIGSERIAL"), PgColumnDecode::Int8);
        assert_eq!(
            plan_column_by_name("int4[]"),
            PgColumnDecode::Array(PgArrayElem::Int4)
        );
        assert_eq!(
            plan_column_by_name("citext[]"),
            PgColumnDecode::Array(PgArrayElem::Text)
        );
        assert_eq!(
            plan_column_by_name("VaRcHaR[][]"),
            PgColumnDecode::Array2D("VaRcHaR".to_string())
        );
        assert_eq!(
            plan_column_by_name("my_enum"),
            PgColumnDecode::Unknown("my_enum".to_string())
        );
        assert_eq!(
            plan_column_by_name("interval[]"),
            PgColumnDecode::Array(PgArrayElem::Unsupported("interval".to_string()))
        );
    }
}
