//! PostgreSQL array literals parsed from their text form.
//!
//! sqlx decodes a one-dimensional array natively; a two-dimensional one and
//! the elements of an array whose type it does not know arrive as the literal
//! the server wrote, and are read here.

use nautilus_core::Value;

use crate::error::{ConnectorError as Error, Result};

/// Parse a PostgreSQL 2D array literal (e.g. `{{1,2},{3,4}}`) into `Value::Array2D`.
pub(super) fn parse_pg_2d_array(input: &str, element_type: &str) -> Result<Value> {
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
