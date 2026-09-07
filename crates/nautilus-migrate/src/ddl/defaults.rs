use super::{DatabaseProvider, DdlGenerator};
use crate::error::{MigrationError, Result};
use nautilus_schema::ir::{DefaultValue, FieldIr, ResolvedFieldType, ScalarType};

impl DdlGenerator {
    /// Render a default while preserving literal case and escaping.
    ///
    /// Boolean literals follow introspection: TRUE/FALSE on PostgreSQL, 1/0 on
    /// MySQL and SQLite. MySQL timestamp defaults match the column's precision.
    pub(crate) fn generate_default_value(
        &self,
        default: &DefaultValue,
        field_type: &ResolvedFieldType,
    ) -> Result<String> {
        match default {
            DefaultValue::String(s) => Ok(format!("'{}'", s.replace('\'', "''"))),
            DefaultValue::Number(n) => Ok(n.clone()),
            DefaultValue::Boolean(b) => match self.provider {
                DatabaseProvider::Postgres => Ok(if *b { "TRUE" } else { "FALSE" }.to_string()),
                DatabaseProvider::Mysql | DatabaseProvider::Sqlite => {
                    Ok(if *b { "1" } else { "0" }.to_string())
                }
            },
            DefaultValue::Function(func) => match func.name.as_str() {
                "autoincrement" => Ok("AUTOINCREMENT".to_string()),
                "uuid" => match self.provider {
                    DatabaseProvider::Postgres => Ok("gen_random_uuid()".to_string()),
                    DatabaseProvider::Sqlite => Ok("(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))))".to_string()),
                    DatabaseProvider::Mysql => Ok("(UUID())".to_string()),
                },
                "uuidv7" => match self.provider {
                    DatabaseProvider::Postgres => Ok("uuidv7()".to_string()),
                    DatabaseProvider::Sqlite => Err(MigrationError::ValidationError(
                        "uuidv7() defaults are not supported by SQLite".to_string(),
                    )),
                    DatabaseProvider::Mysql => Err(MigrationError::ValidationError(
                        "uuidv7() defaults are not supported by MySQL".to_string(),
                    )),
                },
                "now" => match self.provider {
                    DatabaseProvider::Postgres | DatabaseProvider::Sqlite => {
                        Ok("CURRENT_TIMESTAMP".to_string())
                    }
                    DatabaseProvider::Mysql => Ok(self.mysql_current_timestamp(field_type)),
                },
                _ => Ok(format!("{}()", func.name)),
            },
            DefaultValue::Array(values) => self.generate_array_default_value(values, field_type),
            DefaultValue::EnumVariant(variant) => Ok(format!("'{}'", variant)),
        }
    }

    fn generate_array_default_value(
        &self,
        values: &[DefaultValue],
        field_type: &ResolvedFieldType,
    ) -> Result<String> {
        match self.provider {
            DatabaseProvider::Postgres => {
                let elements = values
                    .iter()
                    .map(|value| self.generate_postgres_array_element(value))
                    .collect::<Result<Vec<_>>>()?;
                let cast_type = self.postgres_array_cast_type(field_type)?;
                Ok(format!("ARRAY[{}]::{}[]", elements.join(", "), cast_type))
            }
            DatabaseProvider::Sqlite => {
                let json = self.generate_json_array_literal(values)?;
                Ok(format!("'{}'", json.replace('\'', "''")))
            }
            DatabaseProvider::Mysql => {
                let json = self.generate_json_array_literal(values)?;
                Ok(format!("('{}')", json.replace('\'', "''")))
            }
        }
    }

    fn generate_postgres_array_element(&self, value: &DefaultValue) -> Result<String> {
        match value {
            DefaultValue::String(s) => Ok(format!("'{}'", s.replace('\'', "''"))),
            DefaultValue::Number(n) => Ok(n.clone()),
            DefaultValue::Boolean(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_string()),
            DefaultValue::EnumVariant(variant) => Ok(format!("'{}'", variant.replace('\'', "''"))),
            _ => Err(MigrationError::ValidationError(
                "Array defaults can only contain literal values".to_string(),
            )),
        }
    }

    fn postgres_array_cast_type(&self, field_type: &ResolvedFieldType) -> Result<String> {
        match field_type {
            ResolvedFieldType::Scalar(scalar) => self.scalar_to_pg_type(scalar),
            ResolvedFieldType::Enum { enum_name, .. } => {
                Ok(self.quote_type_identifier(&enum_name.to_lowercase()))
            }
            ResolvedFieldType::CompositeType { db_name, .. } => {
                Ok(self.quote_type_identifier(db_name))
            }
            ResolvedFieldType::Relation(_) => Err(MigrationError::ValidationError(
                "Relation fields cannot have array defaults".to_string(),
            )),
        }
    }

    fn generate_json_array_literal(&self, values: &[DefaultValue]) -> Result<String> {
        let elements = values
            .iter()
            .map(|value| self.generate_json_array_element(value))
            .collect::<Result<Vec<_>>>()?;
        Ok(format!("[{}]", elements.join(",")))
    }

    fn generate_json_array_element(&self, value: &DefaultValue) -> Result<String> {
        match value {
            DefaultValue::String(s) | DefaultValue::EnumVariant(s) => Ok(json_string_literal(s)),
            DefaultValue::Number(n) => Ok(n.clone()),
            DefaultValue::Boolean(b) => Ok(b.to_string()),
            DefaultValue::Array(values) => self.generate_json_array_literal(values),
            DefaultValue::Function(_) => Err(MigrationError::ValidationError(
                "Array defaults can only contain literal values".to_string(),
            )),
        }
    }

    /// Return the default used by the diff, preserving literal case because
    /// changes can emit it verbatim. Exclude autoincrement, which belongs to the
    /// column definition, and include the implicit default supplied by
    /// `@updatedAt` so a later push does not propose dropping it.
    pub fn column_default_sql(&self, field: &FieldIr) -> Result<Option<String>> {
        match &field.default_value {
            Some(DefaultValue::Function(f)) if f.name == "autoincrement" => Ok(None),
            Some(d) => self.generate_default_value(d, &field.field_type).map(Some),
            None if field.is_updated_at => Ok(Some(self.updated_at_default_sql(&field.field_type))),
            None => Ok(None),
        }
    }

    /// The `DEFAULT` expression an `@updatedAt` column is created with.
    pub(super) fn updated_at_default_sql(&self, field_type: &ResolvedFieldType) -> String {
        match self.provider {
            DatabaseProvider::Mysql => self.mysql_current_timestamp(field_type),
            _ => "CURRENT_TIMESTAMP".to_string(),
        }
    }

    /// MySQL's `CURRENT_TIMESTAMP` at the precision a `DateTime` column carries.
    pub(super) fn mysql_current_timestamp(&self, field_type: &ResolvedFieldType) -> String {
        match field_type {
            ResolvedFieldType::Scalar(ScalarType::DateTime) => "CURRENT_TIMESTAMP(6)".to_string(),
            _ => "CURRENT_TIMESTAMP".to_string(),
        }
    }
}

fn json_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
