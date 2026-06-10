//! PostgreSQL row stream and value decoder.

use crate::error::{ConnectorError as Error, Result};
use crate::row_stream::RowStream;
use crate::Row;
use nautilus_core::Value;
use sqlx::postgres::types::PgHstore;
use sqlx::postgres::{PgRow, PgTypeInfo, PgTypeKind as SqlxPgTypeKind, PgValueFormat};
use sqlx::{Column, Row as SqlxRow, TypeInfo, ValueRef};
use uuid::Uuid;

/// Stream type for PostgreSQL query results.
///
/// A thin alias for the shared [`RowStream`] type.
pub type PgRowStream<'conn> = RowStream<'conn>;

/// Decode a sqlx `PgRow` into a Nautilus `Row`.
///
/// Standalone single-row entry point: column classification is paid per call.
/// Multi-row paths should prefer [`decode_rows`] or [`streaming_decoder`],
/// which classify each column once per statement.
pub(crate) fn decode_row_internal(row: PgRow) -> Result<Row> {
    let plan = PgColumnPlan::for_row(&row);
    decode_row_with_plan(&plan, &row)
}

/// Decode a batch of rows produced by a single statement, classifying the
/// column types once (on the first row) instead of once per cell.
pub(crate) fn decode_rows(rows: &[PgRow]) -> Result<Vec<Row>> {
    let Some(first) = rows.first() else {
        return Ok(Vec::new());
    };
    let plan = PgColumnPlan::for_row(first);
    rows.iter()
        .map(|row| decode_row_with_plan(&plan, row))
        .collect()
}

/// Stateful decoder for streaming paths: builds the column plan from the
/// first row that arrives and reuses it for every subsequent row of the
/// statement.
pub(crate) fn streaming_decoder() -> impl FnMut(PgRow) -> Result<Row> + Send + 'static {
    let mut plan: Option<PgColumnPlan> = None;
    move |row| {
        let plan = plan.get_or_insert_with(|| PgColumnPlan::for_row(&row));
        decode_row_with_plan(plan, &row)
    }
}

/// Per-statement decode plan: one [`PgColumnDecode`] per column.
///
/// Hoists the dispatch previously done per cell — the composite check and the
/// `classify_pg_type` scan over the case-insensitive alias table (plus the
/// alias chains for array element types) — so it runs once per column for the
/// whole result set.
#[derive(Debug, Clone, PartialEq)]
struct PgColumnPlan {
    kinds: Vec<PgColumnDecode>,
}

#[derive(Debug, Clone, PartialEq)]
enum PgColumnDecode {
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
enum PgArrayElem {
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
    fn for_row(row: &PgRow) -> Self {
        Self {
            kinds: row
                .columns()
                .iter()
                .map(|column| plan_column(column.type_info()))
                .collect(),
        }
    }
}

fn plan_column(type_info: &PgTypeInfo) -> PgColumnDecode {
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

fn decode_row_with_plan(plan: &PgColumnPlan, row: &PgRow) -> Result<Row> {
    let columns = row.columns();
    if columns.len() != plan.kinds.len() {
        return Err(Error::row_decode_msg(format!(
            "Column plan covers {} columns but the row has {}",
            plan.kinds.len(),
            columns.len()
        )));
    }

    let mut row_data = Row::with_capacity(columns.len());
    for (i, (column, kind)) in columns.iter().zip(&plan.kinds).enumerate() {
        let value = decode_value(row, i, kind)?;
        row_data.push_column(column.name().to_string(), value);
    }

    Ok(row_data)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PgTypeKind<'a> {
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

/// Decode a value from a sqlx row by index, using the pre-resolved column kind.
fn decode_value(row: &PgRow, idx: usize, kind: &PgColumnDecode) -> Result<Value> {
    if let Ok(is_null) = sqlx::Row::try_get_raw(row, idx).map(|raw| raw.is_null()) {
        if is_null {
            return Ok(Value::Null);
        }
    }

    match kind {
        PgColumnDecode::Composite => {
            decode_pg_composite_literal(row, idx, row.columns()[idx].type_info())
        }

        PgColumnDecode::Bool => sqlx::Row::try_get_unchecked::<bool, _>(row, idx)
            .map(Value::Bool)
            .map_err(|e| Error::row_decode(e, "Failed to decode BOOL")),

        PgColumnDecode::Int2 => sqlx::Row::try_get_unchecked::<i16, _>(row, idx)
            .map(|value| Value::I64(value as i64))
            .map_err(|e| Error::row_decode(e, "Failed to decode INT2")),

        PgColumnDecode::Int4 => sqlx::Row::try_get_unchecked::<i32, _>(row, idx)
            .map(|value| Value::I64(value as i64))
            .map_err(|e| Error::row_decode(e, "Failed to decode INT4")),

        PgColumnDecode::Int8 => sqlx::Row::try_get_unchecked::<i64, _>(row, idx)
            .map(Value::I64)
            .map_err(|e| Error::row_decode(e, "Failed to decode INT8")),

        PgColumnDecode::Float4 => sqlx::Row::try_get_unchecked::<f32, _>(row, idx)
            .map(|value| Value::F64(value as f64))
            .map_err(|e| Error::row_decode(e, "Failed to decode FLOAT4")),

        PgColumnDecode::Float8 => sqlx::Row::try_get_unchecked::<f64, _>(row, idx)
            .map(Value::F64)
            .map_err(|e| Error::row_decode(e, "Failed to decode FLOAT8")),

        PgColumnDecode::Text => sqlx::Row::try_get_unchecked::<String, _>(row, idx)
            .map(Value::String)
            .map_err(|e| Error::row_decode(e, "Failed to decode string")),

        PgColumnDecode::Geometry => sqlx::Row::try_get_unchecked::<String, _>(row, idx)
            .map(Value::Geometry)
            .map_err(|e| Error::row_decode(e, "Failed to decode GEOMETRY")),

        PgColumnDecode::Geography => sqlx::Row::try_get_unchecked::<String, _>(row, idx)
            .map(Value::Geography)
            .map_err(|e| Error::row_decode(e, "Failed to decode GEOGRAPHY")),

        PgColumnDecode::Hstore => sqlx::Row::try_get_unchecked::<PgHstore, _>(row, idx)
            .map(|map| Value::Hstore(map.0))
            .map_err(|e| Error::row_decode(e, "Failed to decode HSTORE")),

        PgColumnDecode::Vector => sqlx::Row::try_get_unchecked::<String, _>(row, idx)
            .map_err(|e| Error::row_decode(e, "Failed to decode VECTOR"))
            .and_then(|raw| parse_pg_vector(&raw)),

        PgColumnDecode::Bytes => sqlx::Row::try_get_unchecked::<Vec<u8>, _>(row, idx)
            .map(Value::Bytes)
            .map_err(|e| Error::row_decode(e, "Failed to decode bytes")),

        PgColumnDecode::Uuid => sqlx::Row::try_get_unchecked::<Uuid, _>(row, idx)
            .map(Value::Uuid)
            .map_err(|e| Error::row_decode(e, "Failed to decode UUID")),

        PgColumnDecode::Timestamp => {
            sqlx::Row::try_get_unchecked::<chrono::NaiveDateTime, _>(row, idx)
                .map(Value::DateTime)
                .map_err(|e| Error::row_decode(e, "Failed to decode TIMESTAMP"))
        }

        PgColumnDecode::TimestampTz => {
            sqlx::Row::try_get_unchecked::<chrono::DateTime<chrono::Utc>, _>(row, idx)
                .map(|dt| Value::DateTime(dt.naive_utc()))
                .map_err(|e| Error::row_decode(e, "Failed to decode TIMESTAMPTZ"))
        }

        PgColumnDecode::Date => sqlx::Row::try_get_unchecked::<chrono::NaiveDate, _>(row, idx)
            .map(|d| {
                Value::DateTime(
                    d.and_hms_opt(0, 0, 0)
                        .expect("midnight (0, 0, 0) is always a valid time"),
                )
            })
            .map_err(|e| Error::row_decode(e, "Failed to decode DATE")),

        PgColumnDecode::Time => sqlx::Row::try_get_unchecked::<chrono::NaiveTime, _>(row, idx)
            .map(|t| Value::String(t.to_string()))
            .map_err(|e| Error::row_decode(e, "Failed to decode TIME")),

        PgColumnDecode::Numeric => {
            sqlx::Row::try_get_unchecked::<rust_decimal::Decimal, _>(row, idx)
                .map(Value::Decimal)
                .map_err(|e| Error::row_decode(e, "Failed to decode NUMERIC"))
        }

        // Handle PostgreSQL 2D array types (TEXT[][], INT4[][], etc.)
        // sqlx doesn't support 2D array decoding natively, so we decode
        // the text representation and parse the PostgreSQL array literal.
        PgColumnDecode::Array2D(element_type) => {
            sqlx::Row::try_get_unchecked::<String, _>(row, idx)
                .map_err(|e| Error::row_decode(e, "Failed to decode 2D array"))
                .and_then(|s| parse_pg_2d_array(&s, element_type))
        }

        PgColumnDecode::Array(element) => match element {
            PgArrayElem::Text => sqlx::Row::try_get_unchecked::<Vec<String>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::String).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode TEXT[]")),
            PgArrayElem::Geometry => sqlx::Row::try_get_unchecked::<Vec<String>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::Geometry).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode GEOMETRY[]")),
            PgArrayElem::Geography => sqlx::Row::try_get_unchecked::<Vec<String>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::Geography).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode GEOGRAPHY[]")),
            PgArrayElem::Hstore => sqlx::Row::try_get_unchecked::<Vec<PgHstore>, _>(row, idx)
                .map(|vec| {
                    Value::Array(vec.into_iter().map(|item| Value::Hstore(item.0)).collect())
                })
                .map_err(|e| Error::row_decode(e, "Failed to decode HSTORE[]")),
            PgArrayElem::Int2 => sqlx::Row::try_get_unchecked::<Vec<i16>, _>(row, idx)
                .map(|vec| {
                    Value::Array(
                        vec.into_iter()
                            .map(|item| Value::I32(item as i32))
                            .collect(),
                    )
                })
                .map_err(|e| Error::row_decode(e, "Failed to decode SMALLINT[]")),
            PgArrayElem::Int4 => sqlx::Row::try_get_unchecked::<Vec<i32>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::I32).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode INT[]")),
            PgArrayElem::Int8 => sqlx::Row::try_get_unchecked::<Vec<i64>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::I64).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode BIGINT[]")),
            PgArrayElem::Float4 => sqlx::Row::try_get_unchecked::<Vec<f32>, _>(row, idx)
                .map(|vec| {
                    Value::Array(
                        vec.into_iter()
                            .map(|item| Value::F64(item as f64))
                            .collect(),
                    )
                })
                .map_err(|e| Error::row_decode(e, "Failed to decode REAL[]")),
            PgArrayElem::Float8 => sqlx::Row::try_get_unchecked::<Vec<f64>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::F64).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode FLOAT[]")),
            PgArrayElem::Bool => sqlx::Row::try_get_unchecked::<Vec<bool>, _>(row, idx)
                .map(|vec| Value::Array(vec.into_iter().map(Value::Bool).collect()))
                .map_err(|e| Error::row_decode(e, "Failed to decode BOOL[]")),
            PgArrayElem::Unsupported(element_type) => Err(Error::row_decode_msg(format!(
                "Unsupported array element type: {}",
                element_type
            ))),
        },

        PgColumnDecode::Json => sqlx::Row::try_get_unchecked::<serde_json::Value, _>(row, idx)
            .map(Value::Json)
            .map_err(|e| Error::row_decode(e, "Failed to decode JSON")),

        PgColumnDecode::Unknown(type_name) => {
            // For unknown types (custom enums, domains, composite types, etc.)
            // we bypass sqlx's type-compatibility check so the raw text
            // representation is returned regardless of the server-side type OID.
            sqlx::Row::try_get_unchecked::<String, _>(row, idx)
                .map(Value::String)
                .map_err(|e| {
                    Error::row_decode_msg(format!(
                        "Unsupported type '{}' at column {}: {}",
                        type_name, idx, e
                    ))
                })
        }
    }
}

fn decode_pg_composite_literal(row: &PgRow, idx: usize, type_info: &PgTypeInfo) -> Result<Value> {
    let raw = sqlx::Row::try_get_raw(row, idx)
        .map_err(|e| Error::row_decode(e, "Failed to read composite value"))?;

    if raw.is_null() {
        return Ok(Value::Null);
    }

    if raw.format() == PgValueFormat::Text {
        return raw
            .as_str()
            .map(|value| Value::String(value.to_string()))
            .map_err(|e| {
                Error::row_decode_msg(format!(
                    "Failed to decode composite '{}' as text: {}",
                    type_info.name(),
                    e
                ))
            });
    }

    let SqlxPgTypeKind::Composite(fields) = type_info.kind() else {
        return Err(Error::row_decode_msg(format!(
            "Type '{}' is not a PostgreSQL composite",
            type_info.name()
        )));
    };

    let bytes = raw.as_bytes().map_err(|e| {
        Error::row_decode_msg(format!(
            "Failed to read binary composite '{}': {}",
            type_info.name(),
            e
        ))
    })?;
    let mut offset = 0;
    let field_count = read_pg_i32(bytes, &mut offset, "composite field count")?;
    if field_count < 0 {
        return Err(Error::row_decode_msg(format!(
            "Composite '{}' reported a negative field count: {}",
            type_info.name(),
            field_count
        )));
    }

    let field_count = field_count as usize;
    if field_count != fields.len() {
        return Err(Error::row_decode_msg(format!(
            "Composite '{}' returned {} fields but type metadata has {} fields",
            type_info.name(),
            field_count,
            fields.len()
        )));
    }

    let mut values = Vec::with_capacity(field_count);
    for (field_index, (_, field_type)) in fields.iter().enumerate() {
        let _field_oid = read_pg_u32(bytes, &mut offset, "composite field type OID")?;
        let field_len = read_pg_i32(bytes, &mut offset, "composite field length")?;
        if field_len == -1 {
            values.push(None);
            continue;
        }
        if field_len < -1 {
            return Err(Error::row_decode_msg(format!(
                "Composite '{}' field {} has invalid length {}",
                type_info.name(),
                field_index,
                field_len
            )));
        }

        let field_bytes = take_pg_bytes(
            bytes,
            &mut offset,
            field_len as usize,
            "composite field value",
        )?;
        let decoded = decode_pg_binary_field_text(field_bytes, field_type).map_err(|error| {
            Error::row_decode_msg(format!(
                "Failed to decode composite '{}' field {} ('{}') as '{}': {}",
                type_info.name(),
                field_index,
                fields[field_index].0,
                field_type.name(),
                error
            ))
        })?;
        values.push(Some(decoded));
    }

    if offset != bytes.len() {
        return Err(Error::row_decode_msg(format!(
            "Composite '{}' had {} trailing bytes after decoding",
            type_info.name(),
            bytes.len() - offset
        )));
    }

    Ok(Value::String(pg_record_literal_from_fields(&values)))
}

fn read_pg_i32(bytes: &[u8], offset: &mut usize, context: &str) -> Result<i32> {
    let chunk = take_pg_bytes(bytes, offset, 4, context)?;
    Ok(i32::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

fn read_pg_u32(bytes: &[u8], offset: &mut usize, context: &str) -> Result<u32> {
    let chunk = take_pg_bytes(bytes, offset, 4, context)?;
    Ok(u32::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

fn take_pg_bytes<'a>(
    bytes: &'a [u8],
    offset: &mut usize,
    len: usize,
    context: &str,
) -> Result<&'a [u8]> {
    let end = offset.checked_add(len).ok_or_else(|| {
        Error::row_decode_msg(format!("PostgreSQL binary {} offset overflow", context))
    })?;
    if end > bytes.len() {
        return Err(Error::row_decode_msg(format!(
            "PostgreSQL binary {} expected {} bytes at offset {}, but only {} bytes remain",
            context,
            len,
            *offset,
            bytes.len().saturating_sub(*offset)
        )));
    }
    let chunk = &bytes[*offset..end];
    *offset = end;
    Ok(chunk)
}

fn decode_pg_binary_field_text(
    bytes: &[u8],
    type_info: &PgTypeInfo,
) -> std::result::Result<String, String> {
    if let SqlxPgTypeKind::Domain(inner) = type_info.kind() {
        return decode_pg_binary_field_text(bytes, inner);
    }
    if matches!(type_info.kind(), SqlxPgTypeKind::Enum(_)) {
        return decode_pg_utf8(bytes, type_info.name());
    }

    match classify_pg_type(type_info.name()) {
        PgTypeKind::Bool => {
            let byte = expect_pg_len(bytes, 1, type_info.name())?[0];
            Ok(if byte == 0 { "f" } else { "t" }.to_string())
        }
        PgTypeKind::Int2 => {
            let chunk = expect_pg_len(bytes, 2, type_info.name())?;
            Ok(i16::from_be_bytes(chunk.try_into().expect("slice length checked")).to_string())
        }
        PgTypeKind::Int4 => {
            let chunk = expect_pg_len(bytes, 4, type_info.name())?;
            Ok(i32::from_be_bytes(chunk.try_into().expect("slice length checked")).to_string())
        }
        PgTypeKind::Int8 => {
            let chunk = expect_pg_len(bytes, 8, type_info.name())?;
            Ok(i64::from_be_bytes(chunk.try_into().expect("slice length checked")).to_string())
        }
        PgTypeKind::Float4 => {
            let chunk = expect_pg_len(bytes, 4, type_info.name())?;
            let bits = u32::from_be_bytes(chunk.try_into().expect("slice length checked"));
            Ok(f32::from_bits(bits).to_string())
        }
        PgTypeKind::Float8 => {
            let chunk = expect_pg_len(bytes, 8, type_info.name())?;
            let bits = u64::from_be_bytes(chunk.try_into().expect("slice length checked"));
            Ok(f64::from_bits(bits).to_string())
        }
        PgTypeKind::Text | PgTypeKind::Geometry | PgTypeKind::Geography | PgTypeKind::Vector => {
            decode_pg_utf8(bytes, type_info.name())
        }
        PgTypeKind::Bytes => Ok(format_pg_bytea_hex(bytes)),
        PgTypeKind::Uuid => Uuid::from_slice(expect_pg_len(bytes, 16, type_info.name())?)
            .map(|uuid| uuid.to_string())
            .map_err(|e| e.to_string()),
        PgTypeKind::Timestamp | PgTypeKind::TimestampTz => decode_pg_timestamp_text(bytes),
        PgTypeKind::Date => decode_pg_date_text(bytes),
        PgTypeKind::Time => decode_pg_time_text(bytes),
        PgTypeKind::Numeric => decode_pg_numeric_text(bytes),
        PgTypeKind::Json => decode_pg_json_text(bytes, type_info.name()),
        PgTypeKind::Hstore | PgTypeKind::Array(_) | PgTypeKind::Array2D(_) => Err(format!(
            "binary decoding is not supported for composite field type '{}'",
            type_info.name()
        )),
        PgTypeKind::Unknown => decode_pg_utf8(bytes, type_info.name()),
    }
}

fn expect_pg_len<'a>(
    bytes: &'a [u8],
    len: usize,
    type_name: &str,
) -> std::result::Result<&'a [u8], String> {
    if bytes.len() == len {
        Ok(bytes)
    } else {
        Err(format!(
            "expected {} bytes for '{}', got {}",
            len,
            type_name,
            bytes.len()
        ))
    }
}

fn decode_pg_utf8(bytes: &[u8], type_name: &str) -> std::result::Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|e| format!("invalid UTF-8 for '{}': {}", type_name, e))
}

fn format_pg_bytea_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push_str("\\x");
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_pg_timestamp_text(bytes: &[u8]) -> std::result::Result<String, String> {
    let chunk = expect_pg_len(bytes, 8, "timestamp")?;
    let micros = i64::from_be_bytes(chunk.try_into().expect("slice length checked"));
    if micros == i64::MAX {
        return Ok("infinity".to_string());
    }
    if micros == i64::MIN {
        return Ok("-infinity".to_string());
    }

    let epoch = chrono::NaiveDate::from_ymd_opt(2000, 1, 1)
        .expect("PostgreSQL epoch date is valid")
        .and_hms_opt(0, 0, 0)
        .expect("PostgreSQL epoch time is valid");
    Ok((epoch + chrono::Duration::microseconds(micros))
        .format("%Y-%m-%d %H:%M:%S%.f")
        .to_string())
}

fn decode_pg_date_text(bytes: &[u8]) -> std::result::Result<String, String> {
    let chunk = expect_pg_len(bytes, 4, "date")?;
    let days = i32::from_be_bytes(chunk.try_into().expect("slice length checked"));
    if days == i32::MAX {
        return Ok("infinity".to_string());
    }
    if days == i32::MIN {
        return Ok("-infinity".to_string());
    }

    let epoch =
        chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("PostgreSQL epoch date is valid");
    Ok((epoch + chrono::Duration::days(i64::from(days)))
        .format("%Y-%m-%d")
        .to_string())
}

fn decode_pg_time_text(bytes: &[u8]) -> std::result::Result<String, String> {
    let chunk = expect_pg_len(bytes, 8, "time")?;
    let micros = i64::from_be_bytes(chunk.try_into().expect("slice length checked"));
    if !(0..86_400_000_000).contains(&micros) {
        return Err(format!("time value is out of range: {}", micros));
    }
    let seconds = (micros / 1_000_000) as u32;
    let nanos = ((micros % 1_000_000) as u32) * 1_000;
    chrono::NaiveTime::from_num_seconds_from_midnight_opt(seconds, nanos)
        .map(|time| time.format("%H:%M:%S%.f").to_string())
        .ok_or_else(|| format!("time value is out of range: {}", micros))
}

fn decode_pg_json_text(bytes: &[u8], type_name: &str) -> std::result::Result<String, String> {
    if pg_type_is(type_name, "JSONB") {
        let Some((&version, rest)) = bytes.split_first() else {
            return Err("jsonb value is empty".to_string());
        };
        if version != 1 {
            return Err(format!("unsupported jsonb version byte: {}", version));
        }
        decode_pg_utf8(rest, type_name)
    } else {
        decode_pg_utf8(bytes, type_name)
    }
}

fn decode_pg_numeric_text(bytes: &[u8]) -> std::result::Result<String, String> {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(2) {
        return Err(format!("invalid numeric payload length: {}", bytes.len()));
    }

    let raw_ndigits = read_i16_at(bytes, 0)?;
    if raw_ndigits < 0 {
        return Err(format!(
            "invalid negative numeric digit count: {raw_ndigits}"
        ));
    }
    let ndigits = raw_ndigits as usize;
    let weight = read_i16_at(bytes, 2)?;
    let sign = read_u16_at(bytes, 4)?;
    let dscale = read_u16_at(bytes, 6)? as usize;

    let expected_len = 8 + ndigits * 2;
    if bytes.len() != expected_len {
        return Err(format!(
            "numeric payload has {} bytes, expected {} for {} base-10000 digits",
            bytes.len(),
            expected_len,
            ndigits
        ));
    }

    if sign == 0xC000 {
        return Ok("NaN".to_string());
    }
    let negative = match sign {
        0x0000 => false,
        0x4000 => true,
        other => return Err(format!("invalid numeric sign marker: {:#06x}", other)),
    };

    let mut digits = Vec::with_capacity(ndigits);
    for idx in 0..ndigits {
        let digit = read_u16_at(bytes, 8 + idx * 2)?;
        if digit >= 10_000 {
            return Err(format!("numeric base-10000 digit out of range: {}", digit));
        }
        digits.push(digit);
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }

    if ndigits == 0 {
        out.push('0');
        if dscale > 0 {
            out.push('.');
            out.extend(std::iter::repeat_n('0', dscale));
        }
        return Ok(out);
    }

    if weight < 0 {
        out.push('0');
    } else {
        for group_index in 0..=weight as usize {
            let digit = digits.get(group_index).copied().unwrap_or(0);
            if group_index == 0 {
                out.push_str(&digit.to_string());
            } else {
                push_padded_base10000(&mut out, digit);
            }
        }
    }

    if dscale > 0 {
        out.push('.');
        let fractional_groups = dscale.div_ceil(4);
        let fractional_start = out.len();
        for group_offset in 1..=fractional_groups {
            let digit_index = isize::from(weight) + group_offset as isize;
            let digit = if digit_index >= 0 {
                digits.get(digit_index as usize).copied().unwrap_or(0)
            } else {
                0
            };
            push_padded_base10000(&mut out, digit);
        }
        out.truncate(fractional_start + dscale);
    }

    Ok(out)
}

fn read_i16_at(bytes: &[u8], offset: usize) -> std::result::Result<i16, String> {
    let chunk = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("expected i16 at offset {}", offset))?;
    Ok(i16::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

fn read_u16_at(bytes: &[u8], offset: usize) -> std::result::Result<u16, String> {
    let chunk = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("expected u16 at offset {}", offset))?;
    Ok(u16::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

fn push_padded_base10000(out: &mut String, digit: u16) {
    use std::fmt::Write as _;
    write!(out, "{:04}", digit).expect("writing to String cannot fail");
}

fn pg_record_literal_from_fields(fields: &[Option<String>]) -> String {
    let mut out = String::with_capacity(fields.len().saturating_mul(8) + 2);
    out.push('(');
    for (idx, field) in fields.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        if let Some(text) = field {
            push_pg_record_literal_field(&mut out, text);
        }
    }
    out.push(')');
    out
}

fn push_pg_record_literal_field(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\"\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

fn classify_pg_type(type_name: &str) -> PgTypeKind<'_> {
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

fn pg_type_is(type_name: &str, expected: &str) -> bool {
    type_name.eq_ignore_ascii_case(expected)
}

fn matches_pg_type(type_name: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| pg_type_is(type_name, candidate))
}

fn parse_pg_vector(input: &str) -> Result<Value> {
    let trimmed = input.trim();
    let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Err(Error::row_decode_msg(format!(
            "Invalid vector literal: {}",
            input
        )));
    };

    if inner.trim().is_empty() {
        return Ok(Value::Vector(Vec::new()));
    }

    let parts = inner.split(',');
    let mut values = Vec::with_capacity(parts.size_hint().0);
    for (idx, raw) in parts.enumerate() {
        let value = raw.trim().parse::<f32>().map_err(|e| {
            Error::row_decode_msg(format!(
                "Invalid vector element at index {} in {:?}: {}",
                idx, input, e
            ))
        })?;
        if !value.is_finite() {
            return Err(Error::row_decode_msg(format!(
                "Invalid non-finite vector element at index {} in {:?}",
                idx, input
            )));
        }
        values.push(value);
    }

    Ok(Value::Vector(values))
}

/// Parse a PostgreSQL 2D array literal (e.g. `{{1,2},{3,4}}`) into `Value::Array2D`.
fn parse_pg_2d_array(input: &str, element_type: &str) -> Result<Value> {
    let trimmed = input.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err(Error::row_decode_msg(format!(
            "Invalid 2D array literal: {}",
            input
        )));
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    let rows = split_pg_inner_arrays(inner)?;

    let mut result = Vec::with_capacity(rows.len());
    for row_str in rows {
        let elements = split_pg_array_elements(row_str)?;
        let row: Vec<Value> = elements
            .into_iter()
            .map(|elem| parse_pg_element(elem, element_type))
            .collect::<Result<_>>()?;
        result.push(row);
    }

    Ok(Value::Array2D(result))
}

/// Split the inner content of a 2D array into individual sub-array strings.
///
/// Input: `{1,2},{3,4}` -> `["1,2", "3,4"]`
fn split_pg_inner_arrays(input: &str) -> Result<Vec<&str>> {
    let mut arrays = Vec::new();
    let mut depth = 0;
    let mut start = None;

    for (i, ch) in input.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(i + 1);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let s = start.ok_or_else(|| {
                        Error::row_decode_msg("Malformed 2D array: unmatched brace".to_string())
                    })?;
                    arrays.push(&input[s..i]);
                    start = None;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(Error::row_decode_msg(
            "Malformed 2D array: unbalanced braces".to_string(),
        ));
    }

    Ok(arrays)
}

/// Split a comma-separated list of PostgreSQL array elements, respecting quoted strings.
///
/// Input: `"hello","world"` -> `[r#""hello""#, r#""world""#]`
/// Input: `1,2,NULL` -> `["1", "2", "NULL"]`
fn split_pg_array_elements(input: &str) -> Result<Vec<&str>> {
    let mut elements = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                in_quotes = !in_quotes;
            }
            b'\\' if in_quotes => {
                i += 1;
            }
            b',' if !in_quotes => {
                elements.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    if start <= input.len() {
        elements.push(&input[start..]);
    }

    Ok(elements)
}

/// Parse a single PostgreSQL array element string into a `Value`.
fn parse_pg_element(elem: &str, element_type: &str) -> Result<Value> {
    let trimmed = elem.trim();

    if trimmed.eq_ignore_ascii_case("NULL") {
        return Ok(Value::Null);
    }

    match element_type {
        "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" => Ok(Value::String(unquote_pg_string(trimmed))),
        "INT2" | "INT4" => trimmed
            .parse::<i32>()
            .map(Value::I32)
            .map_err(|e| Error::row_decode_msg(format!("Invalid integer '{}': {}", trimmed, e))),
        "INT8" | "BIGINT" => trimmed
            .parse::<i64>()
            .map(Value::I64)
            .map_err(|e| Error::row_decode_msg(format!("Invalid bigint '{}': {}", trimmed, e))),
        "FLOAT4" | "FLOAT8" | "REAL" | "DOUBLE PRECISION" => trimmed
            .parse::<f64>()
            .map(Value::F64)
            .map_err(|e| Error::row_decode_msg(format!("Invalid float '{}': {}", trimmed, e))),
        "BOOL" => match trimmed {
            "t" | "true" | "TRUE" => Ok(Value::Bool(true)),
            "f" | "false" | "FALSE" => Ok(Value::Bool(false)),
            _ => Err(Error::row_decode_msg(format!(
                "Invalid boolean: {}",
                trimmed
            ))),
        },
        _ => Ok(Value::String(unquote_pg_string(trimmed))),
    }
}

/// Remove surrounding double-quotes and unescape backslash sequences.
fn unquote_pg_string(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut result = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    result.push(escaped);
                }
            } else {
                result.push(ch);
            }
        }
        result
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_2d_int_array() {
        let result = parse_pg_2d_array("{{1,2},{3,4}}", "INT4").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![
                vec![Value::I32(1), Value::I32(2)],
                vec![Value::I32(3), Value::I32(4)],
            ])
        );
    }

    #[test]
    fn parse_2d_bigint_array() {
        let result = parse_pg_2d_array("{{100,200},{300,400}}", "INT8").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![
                vec![Value::I64(100), Value::I64(200)],
                vec![Value::I64(300), Value::I64(400)],
            ])
        );
    }

    #[test]
    fn parse_2d_text_array() {
        let result = parse_pg_2d_array(r#"{{"hello","world"},{"foo","bar"}}"#, "TEXT").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![
                vec![
                    Value::String("hello".to_string()),
                    Value::String("world".to_string())
                ],
                vec![
                    Value::String("foo".to_string()),
                    Value::String("bar".to_string())
                ],
            ])
        );
    }

    #[test]
    fn parse_2d_float_array() {
        let result = parse_pg_2d_array("{{1.5,2.5},{3.5,4.5}}", "FLOAT8").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![
                vec![Value::F64(1.5), Value::F64(2.5)],
                vec![Value::F64(3.5), Value::F64(4.5)],
            ])
        );
    }

    #[test]
    fn parse_vector_literal() {
        assert_eq!(
            parse_pg_vector("[1,2.5,3.25]").unwrap(),
            Value::Vector(vec![1.0, 2.5, 3.25])
        );
    }

    #[test]
    fn parse_vector_rejects_invalid_literal() {
        assert!(parse_pg_vector("{1,2,3}").is_err());
    }

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

    #[test]
    fn parse_2d_bool_array() {
        let result = parse_pg_2d_array("{{t,f},{f,t}}", "BOOL").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![
                vec![Value::Bool(true), Value::Bool(false)],
                vec![Value::Bool(false), Value::Bool(true)],
            ])
        );
    }

    #[test]
    fn parse_2d_array_with_nulls() {
        let result = parse_pg_2d_array("{{1,NULL},{NULL,4}}", "INT4").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![
                vec![Value::I32(1), Value::Null],
                vec![Value::Null, Value::I32(4)],
            ])
        );
    }

    #[test]
    fn parse_2d_text_with_escaped_quotes() {
        let result = parse_pg_2d_array(r#"{{"say \"hi\"","normal"}}"#, "TEXT").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![vec![
                Value::String("say \"hi\"".to_string()),
                Value::String("normal".to_string())
            ],])
        );
    }

    #[test]
    fn parse_2d_single_row() {
        let result = parse_pg_2d_array("{{1,2,3}}", "INT4").unwrap();
        assert_eq!(
            result,
            Value::Array2D(vec![vec![Value::I32(1), Value::I32(2), Value::I32(3)],])
        );
    }

    #[test]
    fn parse_2d_array_invalid_format() {
        assert!(parse_pg_2d_array("not an array", "INT4").is_err());
    }

    #[test]
    fn unquote_plain_string() {
        assert_eq!(unquote_pg_string("hello"), "hello");
    }

    #[test]
    fn unquote_quoted_string() {
        assert_eq!(unquote_pg_string(r#""hello""#), "hello");
    }

    #[test]
    fn unquote_escaped_string() {
        assert_eq!(unquote_pg_string(r#""say \"hi\"""#), r#"say "hi""#);
    }
}
