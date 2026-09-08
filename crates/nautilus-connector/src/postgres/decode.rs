//! One row decoded through the statement's column plan.
//!
//! Every column kind the plan can carry is answered here; the families that
//! need more than one sqlx call of their own live in the sibling modules.

use nautilus_core::Value;
use sqlx::postgres::types::PgHstore;
use sqlx::postgres::PgRow;
use sqlx::{Column, Row as SqlxRow, ValueRef};
use uuid::Uuid;

use super::arrays::parse_pg_2d_array;
use super::composite::decode_pg_composite_literal;
use super::decode_plan::{PgArrayElem, PgColumnDecode, PgColumnPlan};
use super::vector::decode_pg_vector;
use crate::error::{ConnectorError as Error, Result};
use crate::Row;

pub(super) fn decode_row_with_plan(plan: &PgColumnPlan, row: &PgRow) -> Result<Row> {
    let column_count = row.columns().len();
    if column_count != plan.kinds.len() {
        return Err(Error::row_decode_msg(format!(
            "Column plan covers {} columns but the row has {}",
            plan.kinds.len(),
            column_count
        )));
    }

    let mut row_data = Row::with_capacity(column_count);
    for (i, (name, kind)) in plan.names.iter().zip(&plan.kinds).enumerate() {
        let value = decode_value(row, i, kind)?;
        row_data.push_column(std::sync::Arc::clone(name), value);
    }

    Ok(row_data)
}

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

        PgColumnDecode::Vector => decode_pg_vector(row, idx),

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
