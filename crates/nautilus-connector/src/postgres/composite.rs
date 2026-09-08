//! PostgreSQL composite columns.
//!
//! The engine reads a composite as the record literal the server writes in
//! text mode, so a binary answer is decoded field by field and written back
//! into that same literal form.

use nautilus_core::Value;
use sqlx::postgres::{PgRow, PgTypeInfo, PgTypeKind as SqlxPgTypeKind, PgValueFormat};
use sqlx::{TypeInfo, ValueRef};

use super::binary::{decode_pg_binary_field_text, read_pg_i32, read_pg_u32, take_pg_bytes};
use crate::error::{ConnectorError as Error, Result};

pub(super) fn decode_pg_composite_literal(
    row: &PgRow,
    idx: usize,
    type_info: &PgTypeInfo,
) -> Result<Value> {
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
