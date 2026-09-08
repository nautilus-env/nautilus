//! The PostgreSQL binary wire form of a single value, rendered as the text the
//! server would have written.
//!
//! A composite arrives as one binary blob holding its fields, so the fields
//! cannot be handed back to sqlx for decoding: each type's binary layout is
//! read here and turned into the text form the record literal carries.

use sqlx::postgres::{PgTypeInfo, PgTypeKind as SqlxPgTypeKind};
use sqlx::TypeInfo;
use uuid::Uuid;

use super::decode_plan::{classify_pg_type, pg_type_is, PgTypeKind};
use crate::error::{ConnectorError as Error, Result};

pub(super) fn read_pg_i32(bytes: &[u8], offset: &mut usize, context: &str) -> Result<i32> {
    let chunk = take_pg_bytes(bytes, offset, 4, context)?;
    Ok(i32::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

pub(super) fn read_pg_i16(bytes: &[u8], offset: &mut usize, context: &str) -> Result<i16> {
    let chunk = take_pg_bytes(bytes, offset, 2, context)?;
    Ok(i16::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

pub(super) fn read_pg_u32(bytes: &[u8], offset: &mut usize, context: &str) -> Result<u32> {
    let chunk = take_pg_bytes(bytes, offset, 4, context)?;
    Ok(u32::from_be_bytes(
        chunk.try_into().expect("slice length checked"),
    ))
}

pub(super) fn take_pg_bytes<'a>(
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

pub(super) fn decode_pg_binary_field_text(
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
