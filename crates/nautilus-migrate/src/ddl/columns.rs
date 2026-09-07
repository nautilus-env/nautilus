use super::{field_is_autoincrement, DatabaseProvider, DdlGenerator};
use crate::error::{MigrationError, Result};
use nautilus_schema::ir::{
    ComputedKind, DefaultValue, FieldIr, ResolvedFieldType, ScalarType, SchemaIr,
};

impl DdlGenerator {
    /// Render a managed column, or `None` for a relation or ignored field.
    ///
    /// `is_autoincrement_pk` identifies a single-column generated primary key:
    /// SQLite inlines it, MySQL requires it for `AUTO_INCREMENT`, and PostgreSQL
    /// uses SERIAL/BIGSERIAL. MySQL `MODIFY COLUMN` must restate that attribute.
    /// Required computed fields still receive NOT NULL; PostgreSQL supports only
    /// STORED expressions. CHECKs are inline here only on SQLite; the other
    /// providers name them in the table definition for introspection.
    /// `@updatedAt` supplies a timestamp default and MySQL's ON UPDATE clause.
    pub(crate) fn generate_column_definition(
        &self,
        field: &FieldIr,
        _schema: &SchemaIr,
        is_autoincrement_pk: bool,
    ) -> Result<Option<String>> {
        if field.is_ignored || matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            return Ok(None);
        }
        if is_autoincrement_pk && self.provider == DatabaseProvider::Sqlite {
            return Ok(Some(format!(
                "{} INTEGER PRIMARY KEY AUTOINCREMENT",
                self.quote_identifier(&field.db_name)
            )));
        }

        let is_autoincrement = field_is_autoincrement(field);
        if is_autoincrement && self.provider == DatabaseProvider::Mysql {
            if !is_autoincrement_pk {
                return Err(MigrationError::ValidationError(format!(
                    "MySQL only supports autoincrement() on a single-column primary key, but column '{}' is not one",
                    field.db_name
                )));
            }

            let int_type = if matches!(
                &field.field_type,
                ResolvedFieldType::Scalar(ScalarType::BigInt)
            ) {
                "BIGINT"
            } else {
                "INT"
            };
            return Ok(Some(format!(
                "{} {} NOT NULL AUTO_INCREMENT",
                self.quote_identifier(&field.db_name),
                int_type,
            )));
        }
        if is_autoincrement && self.provider == DatabaseProvider::Postgres {
            let serial_type = if matches!(
                &field.field_type,
                ResolvedFieldType::Scalar(ScalarType::BigInt)
            ) {
                "BIGSERIAL"
            } else {
                "SERIAL"
            };
            return Ok(Some(format!(
                "{} {}",
                self.quote_identifier(&field.db_name),
                serial_type,
            )));
        }
        if let Some((ref expr, kind)) = field.computed {
            let col_name = self.quote_identifier(&field.db_name);
            let col_type =
                self.generate_column_type(&field.field_type, !field.is_required, false, None)?;
            let sql = match self.provider {
                DatabaseProvider::Postgres => {
                    format!(
                        "{} {} GENERATED ALWAYS AS ({}) STORED",
                        col_name, col_type, expr
                    )
                }
                DatabaseProvider::Mysql => {
                    let kind_str = match kind {
                        ComputedKind::Stored => "STORED",
                        ComputedKind::Virtual => "VIRTUAL",
                    };
                    format!(
                        "{} {} GENERATED ALWAYS AS ({}) {}",
                        col_name, col_type, expr, kind_str
                    )
                }
                DatabaseProvider::Sqlite => {
                    let kind_str = match kind {
                        ComputedKind::Stored => "STORED",
                        ComputedKind::Virtual => "VIRTUAL",
                    };
                    format!("{} {} AS ({}) {}", col_name, col_type, expr, kind_str)
                }
            };
            if field.is_required {
                return Ok(Some(format!("{} NOT NULL", sql)));
            }
            return Ok(Some(sql));
        }

        let mut parts = Vec::new();

        parts.push(self.quote_identifier(&field.db_name));

        let is_optional = !field.is_required;
        parts.push(self.generate_column_type(
            &field.field_type,
            is_optional,
            field.is_array,
            field.storage_strategy,
        )?);

        if field.is_required {
            parts.push("NOT NULL".to_string());
        }
        if let Some(default) = &field.default_value {
            if !matches!(default, DefaultValue::Function(f) if f.name == "autoincrement") {
                parts.push(format!(
                    "DEFAULT {}",
                    self.generate_default_value(default, &field.field_type)?
                ));
            }
        }
        if field.is_updated_at {
            let now = self.updated_at_default_sql(&field.field_type);
            parts.push(format!("DEFAULT {}", now));
            if self.provider == DatabaseProvider::Mysql {
                parts.push(format!("ON UPDATE {}", now));
            }
        }
        if self.provider == DatabaseProvider::Sqlite {
            if let Some(ref check_expr) = field.check {
                parts.push(format!("CHECK ({})", check_expr));
            }
        }

        Ok(Some(parts.join(" ")))
    }
}
