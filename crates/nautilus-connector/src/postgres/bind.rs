//! Parameter binding: a [`Value`] handed to sqlx as a PostgreSQL parameter.
//!
//! Arrays bind as typed slices when every element shares a type, and the
//! values PostgreSQL has no sqlx type for — vectors and composites — are bound
//! as the literal text the server parses back.

use nautilus_core::Value;
use sqlx::postgres::types::PgHstore;

use crate::error::{ConnectorError as Error, Result};

#[derive(Debug, Clone, PartialEq)]
enum PgArrayBinding {
    Strings(Vec<String>),
    Hstores(Vec<PgHstore>),
    Geometries(Vec<String>),
    Geographies(Vec<String>),
    I32s(Vec<i32>),
    I64s(Vec<i64>),
    F64s(Vec<f64>),
    Bools(Vec<bool>),
}

/// Collect a homogeneous slice of [`Value`]s into a typed vector for array binding.
///
/// Matches every element against `Value::$variant`, applying `$elem => $map` to
/// extract the bound element. A `Value::Null` element, or any element of a
/// different variant, produces a descriptive `expected $expected` error.
macro_rules! collect_pg_array {
    ($items:expr, $variant:ident, $elem:pat => $map:expr, $expected:literal) => {{
        let mut values = Vec::with_capacity($items.len());
        for (idx, item) in $items.iter().enumerate() {
            match item {
                Value::$variant($elem) => values.push($map),
                Value::Null => {
                    return Err(Error::database_msg(format!(
                        "PostgreSQL typed array binding does not support NULL element at index {}",
                        idx
                    )));
                }
                other => {
                    return Err(Error::database_msg(format!(
                        "PostgreSQL array element at index {} has type {:?}; expected {}",
                        idx, other, $expected
                    )));
                }
            }
        }
        values
    }};
}

fn bindable_pg_array(items: &[Value]) -> Result<Option<PgArrayBinding>> {
    let Some(first) = items.first() else {
        return Ok(Some(PgArrayBinding::Strings(Vec::new())));
    };

    let binding = match first {
        Value::String(_) => {
            PgArrayBinding::Strings(collect_pg_array!(items, String, v => v.clone(), "String"))
        }
        // citext / ltree elements bind as text; the column's element type gives
        // the server the real type.
        Value::Extension { .. } => PgArrayBinding::Strings(collect_pg_extension_array(items)?),
        Value::Hstore(_) => PgArrayBinding::Hstores(
            collect_pg_array!(items, Hstore, v => PgHstore(v.clone()), "Hstore"),
        ),
        Value::Geometry(_) => PgArrayBinding::Geometries(
            collect_pg_array!(items, Geometry, v => v.clone(), "Geometry"),
        ),
        Value::Geography(_) => PgArrayBinding::Geographies(
            collect_pg_array!(items, Geography, v => v.clone(), "Geography"),
        ),
        Value::I32(_) => PgArrayBinding::I32s(collect_pg_array!(items, I32, v => *v, "I32")),
        Value::I64(_) => PgArrayBinding::I64s(collect_pg_array!(items, I64, v => *v, "I64")),
        Value::F64(_) => PgArrayBinding::F64s(collect_pg_array!(items, F64, v => *v, "F64")),
        Value::Bool(_) => PgArrayBinding::Bools(collect_pg_array!(items, Bool, v => *v, "Bool")),
        _ => return Ok(None),
    };

    Ok(Some(binding))
}

fn collect_pg_extension_array(items: &[Value]) -> Result<Vec<String>> {
    let mut values = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        match item {
            Value::Extension { value, .. } | Value::String(value) => values.push(value.clone()),
            Value::Null => {
                return Err(Error::database_msg(format!(
                    "PostgreSQL typed array binding does not support NULL element at index {}",
                    idx
                )));
            }
            other => {
                return Err(Error::database_msg(format!(
                    "PostgreSQL array element at index {} has type {:?}; expected an extension scalar",
                    idx, other
                )));
            }
        }
    }
    Ok(values)
}

/// Binds a [`Value`] to a PostgreSQL sqlx query as a typed parameter.
///
/// Uses native binding for `Decimal`, `DateTime`, and `Uuid` (PG-specific).
/// Array values are bound as typed slices when the element type is known; unknown
/// or mixed-type arrays fall back to JSON string serialization.
pub(crate) fn bind_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &'q Value,
) -> Result<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    match value {
        Value::Null => Ok(query.bind(None::<String>)),
        Value::Bool(b) => Ok(query.bind(b)),
        Value::I32(i) => Ok(query.bind(i)),
        Value::I64(i) => Ok(query.bind(i)),
        Value::F64(f) => Ok(query.bind(f)),
        Value::Decimal(d) => Ok(query.bind(d)),
        Value::DateTime(dt) => Ok(query.bind(*dt)),
        Value::Uuid(u) => Ok(query.bind(*u)),
        Value::String(s) => Ok(query.bind(s.as_str())),
        Value::Hstore(map) => Ok(query.bind(PgHstore(map.clone()))),
        Value::Geometry(raw) | Value::Geography(raw) => Ok(query.bind(raw.as_str())),
        Value::Vector(values) => Ok(query.bind(format_pg_vector(values)?)),
        Value::Bytes(b) => Ok(query.bind(b.as_slice())),
        Value::Json(j) => Ok(query.bind(j.to_string())),
        Value::Array(items) => match bindable_pg_array(items)? {
            Some(PgArrayBinding::Strings(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::Hstores(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::Geometries(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::Geographies(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::I32s(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::I64s(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::F64s(values)) => Ok(query.bind(values)),
            Some(PgArrayBinding::Bools(values)) => Ok(query.bind(values)),
            None => {
                let strings: Vec<String> = items
                    .iter()
                    .map(|v| crate::utils::value_to_json(v).to_string())
                    .collect();
                Ok(query.bind(strings))
            }
        },
        Value::Array2D(_) => {
            // Bind 2D arrays as a JSON string.
            // sqlx does not support multi-dimensional PostgreSQL arrays directly,
            // so we serialize to JSON and let the query cast if necessary.
            Ok(query.bind(crate::utils::value_to_json(value).to_string()))
        }
        // The PG dialect already appends `::type_name` to the placeholder, so
        // we only need to bind the underlying string value here.
        Value::Extension { value, .. } | Value::Enum { value, .. } => {
            Ok(query.bind(value.as_str()))
        }
        // The PG dialect appends `::type_name`; we bind the composite as its
        // record-literal text form and let PostgreSQL parse and cast it.
        Value::Composite { fields, .. } => Ok(query.bind(encode_pg_composite_literal(fields)?)),
    }
}

/// Encode composite-type field values as a PostgreSQL record literal, e.g.
/// `("0","0","")`. Every non-NULL field is double-quoted (PostgreSQL strips the
/// quotes and re-parses each field with the target column's input function), and
/// NULL fields are emitted as an empty slot. This keeps the encoder free of
/// per-type quoting heuristics.
fn encode_pg_composite_literal(fields: &[Value]) -> Result<String> {
    let mut out = String::with_capacity(fields.len().saturating_mul(8) + 2);
    out.push('(');
    for (idx, field) in fields.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        if let Some(text) = composite_field_text(field)? {
            push_quoted_composite_field(&mut out, &text);
        }
        // `None` => SQL NULL => empty slot.
    }
    out.push(')');
    Ok(out)
}

/// Render a single composite field value to the text PostgreSQL expects inside a
/// record literal. Returns `None` for NULL fields.
fn composite_field_text(value: &Value) -> Result<Option<String>> {
    let text = match value {
        Value::Null => return Ok(None),
        Value::Bool(b) => if *b { "t" } else { "f" }.to_string(),
        Value::I32(i) => i.to_string(),
        Value::I64(i) => i.to_string(),
        Value::F64(f) => f.to_string(),
        Value::Decimal(d) => d.to_string(),
        Value::DateTime(dt) => dt.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
        Value::Uuid(u) => u.to_string(),
        Value::String(s) => s.clone(),
        Value::Extension { value, .. } | Value::Enum { value, .. } => value.clone(),
        Value::Geometry(raw) | Value::Geography(raw) => raw.clone(),
        Value::Vector(values) => format_pg_vector(values)?,
        Value::Json(j) => j.to_string(),
        Value::Composite { fields, .. } => encode_pg_composite_literal(fields)?,
        other => crate::utils::value_to_json(other).to_string(),
    };
    Ok(Some(text))
}

/// Append `text` as a double-quoted composite field, escaping `"` and `\`.
fn push_quoted_composite_field(out: &mut String, text: &str) {
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

fn format_pg_vector(values: &[f32]) -> Result<String> {
    let mut out = String::with_capacity(values.len().saturating_mul(8) + 2);
    out.push('[');
    for (idx, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(Error::database_msg(format!(
                "PostgreSQL vector element at index {} is not finite",
                idx
            )));
        }
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&value.to_string());
    }
    out.push(']');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindable_pg_array_keeps_homogeneous_strings() {
        let binding = bindable_pg_array(&[
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ])
        .expect("string array should bind");

        assert_eq!(
            binding,
            Some(PgArrayBinding::Strings(vec![
                "a".to_string(),
                "b".to_string()
            ]))
        );
    }

    #[test]
    fn bindable_pg_array_rejects_nulls_in_typed_arrays() {
        let err = bindable_pg_array(&[Value::I32(1), Value::Null]).unwrap_err();
        assert!(err.to_string().contains("NULL element"));
    }

    #[test]
    fn composite_literal_encodes_scalar_fields() {
        let literal = encode_pg_composite_literal(&[
            Value::I32(0),
            Value::I32(3),
            Value::F64(1.5),
            Value::Bool(true),
        ])
        .expect("composite should encode");

        assert_eq!(literal, "(\"0\",\"3\",\"1.5\",\"t\")");
    }

    #[test]
    fn composite_literal_emits_empty_slot_for_null() {
        let literal =
            encode_pg_composite_literal(&[Value::I32(7), Value::Null, Value::String("x".into())])
                .expect("composite should encode");

        assert_eq!(literal, "(\"7\",,\"x\")");
    }

    #[test]
    fn composite_literal_escapes_quotes_and_backslashes() {
        let literal =
            encode_pg_composite_literal(&[Value::String("a\"b\\c".into())]).expect("should encode");

        assert_eq!(literal, "(\"a\"\"b\\\\c\")");
    }

    #[test]
    fn bindable_pg_array_keeps_homogeneous_hstores() {
        let binding = bindable_pg_array(&[
            Value::Hstore(std::collections::BTreeMap::from([(
                "display_name".to_string(),
                Some("Bob".to_string()),
            )])),
            Value::Hstore(std::collections::BTreeMap::from([(
                "nickname".to_string(),
                None,
            )])),
        ])
        .expect("hstore array should bind");

        assert_eq!(
            binding,
            Some(PgArrayBinding::Hstores(vec![
                PgHstore(std::collections::BTreeMap::from([(
                    "display_name".to_string(),
                    Some("Bob".to_string()),
                )])),
                PgHstore(std::collections::BTreeMap::from([(
                    "nickname".to_string(),
                    None,
                )])),
            ]))
        );
    }

    #[test]
    fn bindable_pg_array_rejects_mixed_typed_arrays() {
        let err =
            bindable_pg_array(&[Value::Bool(true), Value::String("nope".to_string())]).unwrap_err();
        assert!(err.to_string().contains("expected Bool"));
    }

    #[test]
    fn bindable_pg_array_falls_back_for_unsupported_types() {
        let binding = bindable_pg_array(&[Value::Decimal(rust_decimal::Decimal::new(123, 2))])
            .expect("unsupported arrays should fall back");
        assert_eq!(binding, None);
    }

    #[test]
    fn format_pg_vector_uses_pgvector_text_literal() {
        assert_eq!(format_pg_vector(&[1.0, 2.5, 3.25]).unwrap(), "[1,2.5,3.25]");
    }

    #[test]
    fn format_pg_vector_rejects_non_finite_values() {
        let err = format_pg_vector(&[1.0, f32::NAN]).unwrap_err();
        assert!(err.to_string().contains("not finite"));
    }
}
