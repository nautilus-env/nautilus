//! pgvector `vector` columns, in the text literal and in the binary layout the
//! extended protocol answers with.

use nautilus_core::Value;
use sqlx::postgres::{PgRow, PgValueFormat};
use sqlx::ValueRef;

use super::binary::{read_pg_i16, take_pg_bytes};
use crate::error::{ConnectorError as Error, Result};

pub(super) fn decode_pg_vector(row: &PgRow, idx: usize) -> Result<Value> {
    let raw = sqlx::Row::try_get_raw(row, idx)
        .map_err(|e| Error::row_decode(e, "Failed to read VECTOR value"))?;

    if raw.is_null() {
        return Ok(Value::Null);
    }

    if raw.format() == PgValueFormat::Text {
        return raw
            .as_str()
            .map_err(|e| Error::row_decode_msg(format!("Failed to decode VECTOR as text: {}", e)))
            .and_then(parse_pg_vector);
    }

    let bytes = raw
        .as_bytes()
        .map_err(|e| Error::row_decode_msg(format!("Failed to read binary VECTOR: {}", e)))?;
    parse_pg_vector_binary(bytes)
}

/// Parse pgvector's binary layout: `int16` dimension, `int16` unused, then one
/// big-endian `float4` per dimension.
fn parse_pg_vector_binary(bytes: &[u8]) -> Result<Value> {
    let mut offset = 0;
    let dimensions = read_pg_i16(bytes, &mut offset, "vector dimension count")?;
    let _unused = read_pg_i16(bytes, &mut offset, "vector flags")?;
    if dimensions < 0 {
        return Err(Error::row_decode_msg(format!(
            "Vector reported a negative dimension count: {}",
            dimensions
        )));
    }

    let dimensions = dimensions as usize;
    let mut values = Vec::with_capacity(dimensions);
    for index in 0..dimensions {
        let chunk = take_pg_bytes(bytes, &mut offset, 4, "vector element")?;
        let value = f32::from_be_bytes(chunk.try_into().expect("slice length checked"));
        if !value.is_finite() {
            return Err(Error::row_decode_msg(format!(
                "Invalid non-finite vector element at index {}",
                index
            )));
        }
        values.push(value);
    }

    Ok(Value::Vector(values))
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// pgvector's binary layout, which is what the extended protocol returns:
    /// `int16` dimensions, `int16` unused, then one big-endian `float4` each.
    fn pg_vector_binary(values: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + values.len() * 4);
        bytes.extend_from_slice(&(values.len() as i16).to_be_bytes());
        bytes.extend_from_slice(&0i16.to_be_bytes());
        for value in values {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn parse_vector_binary_payload() {
        assert_eq!(
            parse_pg_vector_binary(&pg_vector_binary(&[1.0, 2.5, 3.25])).unwrap(),
            Value::Vector(vec![1.0, 2.5, 3.25])
        );
        assert_eq!(
            parse_pg_vector_binary(&pg_vector_binary(&[])).unwrap(),
            Value::Vector(Vec::new())
        );
    }

    #[test]
    fn parse_vector_binary_rejects_truncated_and_non_finite_payloads() {
        let truncated = &pg_vector_binary(&[1.0, 2.0])[..8];
        assert!(parse_pg_vector_binary(truncated).is_err());
        assert!(parse_pg_vector_binary(&pg_vector_binary(&[f32::NAN])).is_err());
        assert!(parse_pg_vector_binary(&[]).is_err());
    }
}
