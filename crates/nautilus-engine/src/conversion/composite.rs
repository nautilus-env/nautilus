//! PostgreSQL composite types on both directions of the wire.
//!
//! Input objects are projected onto the declared field order for binding, and
//! the record literal the backend answers with is decoded back into a JSON
//! object keyed by the composite's field names.

use nautilus_core::Value;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{CompositeFieldIr, CompositeTypeIr, ResolvedFieldType, ScalarType};

use super::input::json_to_value_field;

/// Convert a JSON value into a [`Value::Composite`] for a PostgreSQL native
/// composite-type column.
///
/// The incoming object is projected onto the composite type's fields **in their
/// declared order** so the resulting record literal lines up with the column's
/// structure. Missing keys become `Value::Null` (an empty slot in the literal).
/// The carried type name is the composite's physical SQL name (`@@map` value or
/// the lowercased logical name, e.g. `ChampionStats` -> `championstats`) so the
/// dialect can emit the required `$n::championstats` cast.
pub fn json_to_value_composite(
    json: &serde_json::Value,
    composite: &CompositeTypeIr,
) -> Result<Value, ProtocolError> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Object(object) => {
            let mut fields = Vec::with_capacity(composite.fields.len());
            for field in &composite.fields {
                let raw = object
                    .get(&field.logical_name)
                    .or_else(|| object.get(&field.db_name))
                    .unwrap_or(&serde_json::Value::Null);
                fields.push(json_to_value_field(raw, &field.field_type)?);
            }
            Ok(Value::Composite {
                type_name: composite.db_name.clone(),
                fields,
            })
        }
        other => Err(ProtocolError::InvalidParams(format!(
            "Composite type '{}' expects a JSON object, got {:?}",
            composite.logical_name, other
        ))),
    }
}
/// Decode a PostgreSQL composite column (returned by the connector as the raw
/// record-literal text, e.g. `(0,3,1.5)`) into a [`Value::Json`] object keyed by
/// the composite type's field names, with each field coerced to its schema type.
pub(super) fn normalize_composite_value(
    column: &str,
    index: usize,
    value: Value,
    composite: &CompositeTypeIr,
) -> Result<Value, ProtocolError> {
    let raw = match value {
        Value::String(raw) => raw,
        // Already an object (e.g. a JSON-stored composite) or an otherwise
        // structured value — leave it untouched.
        other => return Ok(other),
    };

    let parts = parse_pg_record_literal(&raw).ok_or_else(|| {
        ProtocolError::DatabaseExecution(format!(
            "Failed to decode composite '{}' for column '{}' at position {} from value {:?}",
            composite.logical_name, column, index, raw
        ))
    })?;

    let mut object = serde_json::Map::with_capacity(composite.fields.len());
    for (field_index, field) in composite.fields.iter().enumerate() {
        let part = parts.get(field_index).and_then(Option::as_deref);
        object.insert(
            field.logical_name.clone(),
            composite_field_to_json(part, field),
        );
    }

    Ok(Value::Json(serde_json::Value::Object(object)))
}

/// Coerce one parsed composite field (raw text, or `None` for SQL NULL) into the
/// JSON value matching its schema type. Array fields and unrecognised types fall
/// back to the raw string form.
fn composite_field_to_json(raw: Option<&str>, field: &CompositeFieldIr) -> serde_json::Value {
    let Some(raw) = raw else {
        return serde_json::Value::Null;
    };

    if field.is_array {
        return serde_json::Value::String(raw.to_string());
    }

    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Int | ScalarType::BigInt) => raw
            .parse::<i64>()
            .ok()
            .map(|n| serde_json::Value::Number(n.into()))
            .unwrap_or_else(|| serde_json::Value::String(raw.to_string())),
        ResolvedFieldType::Scalar(ScalarType::Float) => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::String(raw.to_string())),
        ResolvedFieldType::Scalar(ScalarType::Boolean) => match raw {
            "t" | "true" | "TRUE" | "1" => serde_json::Value::Bool(true),
            "f" | "false" | "FALSE" | "0" => serde_json::Value::Bool(false),
            other => serde_json::Value::String(other.to_string()),
        },
        _ => serde_json::Value::String(raw.to_string()),
    }
}

/// Parse a PostgreSQL record/composite literal into its top-level fields.
///
/// Returns `None` when `input` is not parenthesised. Each field is `None` for an
/// unquoted empty slot (SQL NULL) or `Some(text)` with quotes stripped and the
/// `""`/`\\` escapes decoded. Mirrors the encoder in the PostgreSQL connector.
fn parse_pg_record_literal(input: &str) -> Option<Vec<Option<String>>> {
    let trimmed = input.trim();
    let inner = trimmed.strip_prefix('(')?.strip_suffix(')')?;

    let mut fields = Vec::new();
    let mut chars = inner.chars().peekable();

    loop {
        let mut buf = String::new();
        let mut quoted = false;

        if matches!(chars.peek(), Some('"')) {
            quoted = true;
            chars.next(); // opening quote
            while let Some(ch) = chars.next() {
                match ch {
                    '"' => {
                        if matches!(chars.peek(), Some('"')) {
                            chars.next();
                            buf.push('"');
                        } else {
                            break; // closing quote
                        }
                    }
                    '\\' => {
                        if let Some(next) = chars.next() {
                            buf.push(next);
                        }
                    }
                    other => buf.push(other),
                }
            }
        } else {
            while let Some(&ch) = chars.peek() {
                if ch == ',' {
                    break;
                }
                buf.push(ch);
                chars.next();
            }
        }

        // An unquoted empty field is SQL NULL; a quoted empty field is "".
        fields.push(if !quoted && buf.is_empty() {
            None
        } else {
            Some(buf)
        });

        match chars.peek() {
            Some(',') => {
                chars.next();
            }
            _ => break,
        }
    }

    Some(fields)
}
#[cfg(test)]
mod composite_tests {
    use super::*;
    use nautilus_schema::validate_schema_source;

    fn address_composite() -> CompositeTypeIr {
        let schema = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

type Address {
  street String
  number Int
  lat    Float
}

model User {
  id      Int     @id @default(autoincrement())
  address Address
}
"#;
        validate_schema_source(schema)
            .expect("schema should validate")
            .ir
            .composite_types
            .remove("Address")
            .expect("Address composite type missing")
    }

    #[test]
    fn composite_object_projects_fields_in_declared_order() {
        let composite = address_composite();
        let json = serde_json::json!({
            "lat": 1.5,
            "street": "Main",
            "number": 3,
        });

        let value = json_to_value_composite(&json, &composite).expect("should convert");

        assert_eq!(
            value,
            Value::Composite {
                type_name: "address".to_string(),
                fields: vec![
                    Value::String("Main".to_string()),
                    Value::I32(3),
                    Value::F64(1.5)
                ],
            }
        );
    }

    #[test]
    fn composite_uses_mapped_type_name_for_cast() {
        let schema = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

type Address {
  street String
  zip    String @map("zip_code")
  @@map("address_t")
}

model User {
  id      Int     @id @default(autoincrement())
  address Address
}
"#;
        let composite = validate_schema_source(schema)
            .expect("schema should validate")
            .ir
            .composite_types
            .remove("Address")
            .expect("Address composite type missing");

        // Input keyed by the @map'd db name is also accepted.
        let json = serde_json::json!({ "street": "Main", "zip_code": "12345" });
        let value = json_to_value_composite(&json, &composite).expect("should convert");

        assert_eq!(
            value,
            Value::Composite {
                type_name: "address_t".to_string(),
                fields: vec![
                    Value::String("Main".to_string()),
                    Value::String("12345".to_string()),
                ],
            }
        );
    }

    #[test]
    fn composite_missing_keys_become_null_slots() {
        let composite = address_composite();
        let json = serde_json::json!({ "number": 5 });

        let value = json_to_value_composite(&json, &composite).expect("should convert");

        assert_eq!(
            value,
            Value::Composite {
                type_name: "address".to_string(),
                fields: vec![Value::Null, Value::I32(5), Value::Null],
            }
        );
    }

    #[test]
    fn composite_null_input_is_null() {
        let composite = address_composite();
        let value =
            json_to_value_composite(&serde_json::Value::Null, &composite).expect("should convert");
        assert_eq!(value, Value::Null);
    }

    #[test]
    fn composite_non_object_is_rejected() {
        let composite = address_composite();
        let err = json_to_value_composite(&serde_json::json!("oops"), &composite).unwrap_err();
        assert!(err.to_string().contains("expects a JSON object"));
    }

    #[test]
    fn record_literal_parses_unquoted_and_quoted_fields() {
        let parts = parse_pg_record_literal("(0,\"3\",1.5)").expect("should parse");
        assert_eq!(
            parts,
            vec![
                Some("0".to_string()),
                Some("3".to_string()),
                Some("1.5".to_string())
            ]
        );
    }

    #[test]
    fn record_literal_distinguishes_null_from_empty_string() {
        let parts = parse_pg_record_literal("(,\"\")").expect("should parse");
        assert_eq!(parts, vec![None, Some(String::new())]);
    }

    #[test]
    fn record_literal_decodes_escapes() {
        let parts = parse_pg_record_literal("(\"a\"\"b\\\\c\")").expect("should parse");
        assert_eq!(parts, vec![Some("a\"b\\c".to_string())]);
    }

    #[test]
    fn record_literal_rejects_non_parenthesised() {
        assert!(parse_pg_record_literal("0,1,2").is_none());
    }

    #[test]
    fn composite_string_is_decoded_into_typed_object() {
        let composite = address_composite();
        let decoded = normalize_composite_value(
            "users__address",
            0,
            Value::String("(Main,3,1.5)".to_string()),
            &composite,
        )
        .expect("should decode");

        assert_eq!(
            decoded,
            Value::Json(serde_json::json!({
                "street": "Main",
                "number": 3,
                "lat": 1.5,
            }))
        );
    }
}
