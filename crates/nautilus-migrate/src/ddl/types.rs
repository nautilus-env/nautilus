use super::{DatabaseProvider, DdlGenerator};
use crate::error::{MigrationError, Result};
use crate::provider::ProviderStrategy;
use nautilus_schema::ast::StorageStrategy;
use nautilus_schema::ir::{CompositeFieldIr, FieldIr, ResolvedFieldType, ScalarType};

impl DdlGenerator {
    /// Render the provider's column type and array/composite storage strategy.
    ///
    /// SQLite's DECIMAL, DATETIME and CHAR(36) spellings preserve type information
    /// for pull. MySQL DATETIME(6) preserves fractional seconds; its native ENUM
    /// supports defaults and indexes without TEXT's restrictions. PostgreSQL enum
    /// names stay lowercase and unquoted to match introspection.
    pub(super) fn generate_column_type(
        &self,
        field_type: &ResolvedFieldType,
        _is_optional: bool,
        is_array: bool,
        storage_strategy: Option<StorageStrategy>,
    ) -> Result<String> {
        let strategy = ProviderStrategy::new(self.provider);

        if is_array {
            if let ResolvedFieldType::Scalar(scalar) = field_type {
                if self.provider == DatabaseProvider::Postgres {
                    let base = self.scalar_to_pg_type(scalar)?;
                    return Ok(format!("{}[]", base));
                }
                if let Some(storage_sql) = strategy.array_storage_sql(storage_strategy) {
                    return Ok(storage_sql.to_string());
                }
                return Err(MigrationError::ValidationError(
                    strategy.native_array_support_error(),
                ));
            } else if let ResolvedFieldType::Enum { enum_name, .. } = field_type {
                if self.provider == DatabaseProvider::Postgres {
                    return Ok(format!("{}[]", enum_name.to_lowercase()));
                }
                if let Some(storage_sql) = strategy.array_storage_sql(storage_strategy) {
                    return Ok(storage_sql.to_string());
                }
                return Err(MigrationError::ValidationError(
                    strategy.native_array_support_error(),
                ));
            } else if let ResolvedFieldType::CompositeType { type_name, db_name } = field_type {
                if strategy.supports_user_defined_types() {
                    return Ok(format!("{}[]", self.quote_type_identifier(db_name)));
                }
                if let Some(storage_sql) = strategy.composite_storage_sql(storage_strategy) {
                    return Ok(storage_sql.to_string());
                }
                return Err(MigrationError::ValidationError(
                    strategy.native_composite_support_error(type_name, true),
                ));
            } else {
                return Ok("".to_string());
            }
        }

        let base_type = match field_type {
            ResolvedFieldType::Scalar(scalar) => match scalar {
                ScalarType::String => match self.provider {
                    DatabaseProvider::Postgres => "TEXT",
                    DatabaseProvider::Sqlite => "TEXT",
                    DatabaseProvider::Mysql => "VARCHAR(255)",
                },
                ScalarType::Boolean => match self.provider {
                    DatabaseProvider::Postgres => "BOOLEAN",
                    DatabaseProvider::Sqlite => "INTEGER",
                    DatabaseProvider::Mysql => "BOOLEAN",
                },
                ScalarType::Int => match self.provider {
                    DatabaseProvider::Postgres => "INTEGER",
                    DatabaseProvider::Sqlite => "INTEGER",
                    DatabaseProvider::Mysql => "INT",
                },
                ScalarType::BigInt => match self.provider {
                    DatabaseProvider::Postgres => "BIGINT",
                    DatabaseProvider::Sqlite => "INTEGER",
                    DatabaseProvider::Mysql => "BIGINT",
                },
                ScalarType::Float => match self.provider {
                    DatabaseProvider::Postgres => "DOUBLE PRECISION",
                    DatabaseProvider::Sqlite => "REAL",
                    DatabaseProvider::Mysql => "DOUBLE",
                },
                ScalarType::Decimal { precision, scale } => match self.provider {
                    DatabaseProvider::Postgres => &format!("DECIMAL({}, {})", precision, scale),
                    DatabaseProvider::Sqlite => &format!("DECIMAL({}, {})", precision, scale),
                    DatabaseProvider::Mysql => &format!("DECIMAL({}, {})", precision, scale),
                },
                ScalarType::DateTime => match self.provider {
                    DatabaseProvider::Postgres => "TIMESTAMP",
                    DatabaseProvider::Sqlite => "DATETIME",
                    DatabaseProvider::Mysql => "DATETIME(6)",
                },
                ScalarType::Bytes => match self.provider {
                    DatabaseProvider::Postgres => "BYTEA",
                    DatabaseProvider::Sqlite => "BLOB",
                    DatabaseProvider::Mysql => "BLOB",
                },
                ScalarType::Json => match self.provider {
                    DatabaseProvider::Postgres => "JSONB",
                    DatabaseProvider::Sqlite => "JSON",
                    DatabaseProvider::Mysql => "JSON",
                },
                ScalarType::Citext => "CITEXT",
                ScalarType::Hstore => "HSTORE",
                ScalarType::Ltree => "LTREE",
                ScalarType::Geometry => "GEOMETRY",
                ScalarType::Geography => "GEOGRAPHY",
                ScalarType::Vector { dimension } => {
                    return Ok(format!("VECTOR({})", dimension));
                }
                ScalarType::Jsonb => "JSONB",
                ScalarType::Xml => "XML",
                ScalarType::Char { length } => {
                    return Ok(format!("CHAR({})", length));
                }
                ScalarType::VarChar { length } => {
                    return Ok(format!("VARCHAR({})", length));
                }
                ScalarType::Uuid => match self.provider {
                    DatabaseProvider::Postgres => "UUID",
                    DatabaseProvider::Sqlite => "CHAR(36)",
                    DatabaseProvider::Mysql => "CHAR(36)",
                },
            },
            ResolvedFieldType::Enum {
                enum_name,
                variants,
            } => match self.provider {
                DatabaseProvider::Postgres => return Ok(enum_name.to_lowercase()),
                DatabaseProvider::Mysql => return Ok(mysql_enum_type(enum_name, variants)),
                DatabaseProvider::Sqlite => "TEXT",
            },
            ResolvedFieldType::Relation(_) => return Ok("".to_string()),
            ResolvedFieldType::CompositeType { type_name, db_name } => {
                if strategy.supports_user_defined_types() {
                    return Ok(self.quote_type_identifier(db_name));
                }
                if let Some(storage_sql) = strategy.composite_storage_sql(storage_strategy) {
                    storage_sql
                } else {
                    return Err(MigrationError::ValidationError(
                        strategy.native_composite_support_error(type_name, false),
                    ));
                }
            }
        };

        Ok(base_type.to_string())
    }

    /// Return the canonical SQL type string for a field (used by the diff engine).
    ///
    /// The result is lower-cased so it can be compared directly with the
    /// normalised live-DB type returned by `SchemaInspector`. Text inside
    /// single quotes keeps its case, because a MySQL `enum('DRAFT','PUBLISHED')`
    /// is reported by the server with the variants spelled as declared.
    pub fn column_type_sql(&self, field: &FieldIr) -> Result<String> {
        self.generate_column_type(
            &field.field_type,
            !field.is_required,
            field.is_array,
            field.storage_strategy,
        )
        .map(|s| crate::utils::lowercase_outside_quotes(&s))
    }

    /// Return the comparison type for a composite field, using the same storage
    /// mapping and case normalization as [`Self::column_type_sql`].
    pub fn column_type_sql_for_composite(&self, field: &CompositeFieldIr) -> Result<String> {
        self.generate_column_type(
            &field.field_type,
            !field.is_required,
            field.is_array,
            field.storage_strategy,
        )
        .map(|s| crate::utils::lowercase_outside_quotes(&s))
    }

    /// Map scalar type to PostgreSQL base type for arrays
    pub(super) fn scalar_to_pg_type(&self, scalar: &ScalarType) -> Result<String> {
        Ok(match scalar {
            ScalarType::String => "TEXT",
            ScalarType::Boolean => "BOOLEAN",
            ScalarType::Int => "INTEGER",
            ScalarType::BigInt => "BIGINT",
            ScalarType::Float => "DOUBLE PRECISION",
            ScalarType::Decimal { precision, scale } => {
                return Ok(format!("DECIMAL({}, {})", precision, scale));
            }
            ScalarType::DateTime => "TIMESTAMP",
            ScalarType::Bytes => "BYTEA",
            ScalarType::Json => "JSONB",
            ScalarType::Uuid => "UUID",
            ScalarType::Citext => "CITEXT",
            ScalarType::Hstore => "HSTORE",
            ScalarType::Ltree => "LTREE",
            ScalarType::Geometry => "GEOMETRY",
            ScalarType::Geography => "GEOGRAPHY",
            ScalarType::Vector { dimension } => {
                return Ok(format!("VECTOR({})", dimension));
            }
            ScalarType::Jsonb => "JSONB",
            ScalarType::Xml => "XML",
            ScalarType::Char { length } => {
                return Ok(format!("CHAR({})", length));
            }
            ScalarType::VarChar { length } => {
                return Ok(format!("VARCHAR({})", length));
            }
        }
        .to_string())
    }
}

/// Render a MySQL native column enum: `ENUM('DRAFT', 'PUBLISHED')`.
///
/// A variant containing a quote is escaped by doubling it, the only escape
/// MySQL accepts inside a string literal in ANSI_QUOTES mode as well.
fn mysql_enum_type(enum_name: &str, variants: &[String]) -> String {
    if variants.is_empty() {
        return "TEXT".to_string();
    }

    let mut out = String::with_capacity(enum_name.len() + variants.len() * 12);
    out.push_str("ENUM(");
    for (index, variant) in variants.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('\'');
        out.push_str(&variant.replace('\'', "''"));
        out.push('\'');
    }
    out.push(')');
    out
}
