//! One field as the IR carries it: its resolved type, the storage strategy it
//! is allowed to use, and its default value.

use crate::validator::*;

impl SchemaValidator<'_> {
    pub(in crate::validator) fn build_field_ir(
        &self,
        field: &FieldDecl,
        model: &ModelDecl,
    ) -> Result<FieldIr> {
        let logical_name = field.name.value.clone();
        let db_name = field.column_name().to_string();
        let field_type = self.resolve_field_type(field)?;
        let is_required = !field.is_optional() && !field.is_array();
        let is_array = field.is_array();
        let default_value = self.extract_default_value(field)?;
        let is_unique = field
            .attributes
            .iter()
            .any(|a| matches!(a, FieldAttribute::Unique));

        let is_updated_at = field
            .attributes
            .iter()
            .any(|a| matches!(a, FieldAttribute::UpdatedAt { .. }));

        let field_name_map: std::collections::HashMap<String, String> = model
            .fields
            .iter()
            .map(|f| (f.name.value.clone(), f.column_name().to_string()))
            .collect();

        let computed = field.attributes.iter().find_map(|a| {
            if let FieldAttribute::Computed { expr, kind, .. } = a {
                Some((
                    expr.to_sql_mapped(&|name: &str| {
                        field_name_map
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| name.to_string())
                    }),
                    *kind,
                ))
            } else {
                None
            }
        });

        let check = field.attributes.iter().find_map(|a| {
            if let FieldAttribute::Check { expr, .. } = a {
                Some(expr.to_sql_mapped(&|name: &str| {
                    field_name_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_string())
                }))
            } else {
                None
            }
        });

        let storage_strategy = field.attributes.iter().find_map(|attr| {
            if let FieldAttribute::Store { strategy, .. } = attr {
                Some(*strategy)
            } else {
                None
            }
        });

        if field.is_not_null() && matches!(field_type, ResolvedFieldType::Relation(_)) {
            return Err(SchemaError::Validation(
                "The `!` modifier cannot be used on relation fields — NOT NULL applies only to scalar and enum columns.".to_string(),
                field.span,
            ));
        }

        let datasource_provider = self
            .schema
            .datasource()
            .and_then(|datasource| datasource.provider());
        self.validate_composite_storage_strategy(
            field,
            &field_type,
            storage_strategy,
            datasource_provider,
        )?;
        self.validate_array_storage_strategy(
            field,
            &field_type,
            is_array,
            storage_strategy,
            datasource_provider,
        )?;

        Ok(FieldIr {
            logical_name,
            db_name,
            field_type,
            is_required,
            is_array,
            storage_strategy,
            default_value,
            is_unique,
            is_updated_at,
            computed,
            check,
            is_ignored: field.is_ignored(),
            span: field.span,
        })
    }

    fn validate_composite_storage_strategy(
        &self,
        field: &FieldDecl,
        field_type: &ResolvedFieldType,
        storage_strategy: Option<StorageStrategy>,
        datasource_provider: Option<&str>,
    ) -> Result<()> {
        if !matches!(field_type, ResolvedFieldType::CompositeType { .. }) {
            return Ok(());
        }

        let Some(provider_str) = datasource_provider else {
            return Ok(());
        };

        match provider_str.parse::<DatabaseProvider>() {
            Ok(DatabaseProvider::Postgres) => {
                if storage_strategy == Some(StorageStrategy::Json) {
                    return Err(SchemaError::Validation(
                        "PostgreSQL supports native composite types. Remove @store(Json) from this field.".to_string(),
                        field.span,
                    ));
                }
            }
            Ok(db_provider @ (DatabaseProvider::Mysql | DatabaseProvider::Sqlite)) => {
                if storage_strategy.is_none() {
                    return Err(SchemaError::Validation(
                        format!(
                            "{} does not support native composite types. Add @store(Json) to store as JSON.",
                            db_provider.display_name()
                        ),
                        field.span,
                    ));
                }
                if storage_strategy == Some(StorageStrategy::Native) {
                    return Err(SchemaError::Validation(
                        format!(
                            "{} does not support native composite types. Use @store(Json) instead.",
                            db_provider.display_name()
                        ),
                        field.span,
                    ));
                }
            }
            Err(_) => {
                if storage_strategy.is_none() {
                    return Err(SchemaError::Validation(
                        "Composite type fields require explicit storage strategy via @store(Json) or are only natively supported on PostgreSQL.".to_string(),
                        field.span,
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_array_storage_strategy(
        &self,
        field: &FieldDecl,
        field_type: &ResolvedFieldType,
        is_array: bool,
        storage_strategy: Option<StorageStrategy>,
        datasource_provider: Option<&str>,
    ) -> Result<()> {
        if !is_array
            || !matches!(
                field_type,
                ResolvedFieldType::Scalar(_) | ResolvedFieldType::Enum { .. }
            )
        {
            return Ok(());
        }

        let Some(provider_str) = datasource_provider else {
            return Ok(());
        };

        match provider_str.parse::<DatabaseProvider>() {
            Ok(DatabaseProvider::Postgres) => {
                if storage_strategy == Some(StorageStrategy::Json) {
                    return Err(SchemaError::Validation(
                        "PostgreSQL supports native arrays. Use @store(native) or omit @store attribute.".to_string(),
                        field.span,
                    ));
                }
            }
            Ok(db_provider @ (DatabaseProvider::Mysql | DatabaseProvider::Sqlite)) => {
                if storage_strategy.is_none() {
                    return Err(SchemaError::Validation(
                        format!(
                            "{} does not support native array types. Add @store(json) to use JSON serialization for array fields.",
                            db_provider.display_name()
                        ),
                        field.span,
                    ));
                }
                if storage_strategy == Some(StorageStrategy::Native) {
                    return Err(SchemaError::Validation(
                        format!(
                            "{} does not support native array types. Use @store(json) instead.",
                            db_provider.display_name()
                        ),
                        field.span,
                    ));
                }
            }
            Err(_) => {
                if storage_strategy.is_none() {
                    return Err(SchemaError::Validation(
                        "Array fields require explicit storage strategy via @store(json) or @store(native)".to_string(),
                        field.span,
                    ));
                }
            }
        }

        Ok(())
    }

    pub(in crate::validator) fn resolve_field_type(
        &self,
        field: &FieldDecl,
    ) -> Result<ResolvedFieldType> {
        match &field.field_type {
            FieldType::String => Ok(ResolvedFieldType::Scalar(ScalarType::String)),
            FieldType::Boolean => Ok(ResolvedFieldType::Scalar(ScalarType::Boolean)),
            FieldType::Int => Ok(ResolvedFieldType::Scalar(ScalarType::Int)),
            FieldType::BigInt => Ok(ResolvedFieldType::Scalar(ScalarType::BigInt)),
            FieldType::Float => Ok(ResolvedFieldType::Scalar(ScalarType::Float)),
            FieldType::Decimal { precision, scale } => {
                Ok(ResolvedFieldType::Scalar(ScalarType::Decimal {
                    precision: *precision,
                    scale: *scale,
                }))
            }
            FieldType::DateTime => Ok(ResolvedFieldType::Scalar(ScalarType::DateTime)),
            FieldType::Bytes => Ok(ResolvedFieldType::Scalar(ScalarType::Bytes)),
            FieldType::Json => Ok(ResolvedFieldType::Scalar(ScalarType::Json)),
            FieldType::Uuid => Ok(ResolvedFieldType::Scalar(ScalarType::Uuid)),
            FieldType::Citext => Ok(ResolvedFieldType::Scalar(ScalarType::Citext)),
            FieldType::Hstore => Ok(ResolvedFieldType::Scalar(ScalarType::Hstore)),
            FieldType::Ltree => Ok(ResolvedFieldType::Scalar(ScalarType::Ltree)),
            FieldType::Geometry => Ok(ResolvedFieldType::Scalar(ScalarType::Geometry)),
            FieldType::Geography => Ok(ResolvedFieldType::Scalar(ScalarType::Geography)),
            FieldType::Vector { dimension } => Ok(ResolvedFieldType::Scalar(ScalarType::Vector {
                dimension: *dimension,
            })),
            FieldType::Jsonb => Ok(ResolvedFieldType::Scalar(ScalarType::Jsonb)),
            FieldType::Xml => Ok(ResolvedFieldType::Scalar(ScalarType::Xml)),
            FieldType::Char { length } => Ok(ResolvedFieldType::Scalar(ScalarType::Char {
                length: *length,
            })),
            FieldType::VarChar { length } => Ok(ResolvedFieldType::Scalar(ScalarType::VarChar {
                length: *length,
            })),
            FieldType::UserType(type_name) => {
                if self.enums.contains_key(type_name) {
                    let variants = self
                        .schema
                        .enums()
                        .find(|decl| decl.name.value == *type_name)
                        .map(|decl| {
                            decl.variants
                                .iter()
                                .map(|variant| variant.name.value.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    return Ok(ResolvedFieldType::Enum {
                        enum_name: type_name.clone(),
                        variants,
                    });
                }

                if self.composite_types.contains_key(type_name) {
                    let db_name = self
                        .schema
                        .types()
                        .find(|t| &t.name.value == type_name)
                        .map(|t| t.db_type_name())
                        .unwrap_or_else(|| type_name.to_lowercase());
                    return Ok(ResolvedFieldType::CompositeType {
                        type_name: type_name.clone(),
                        db_name,
                    });
                }

                if self.models.contains_key(type_name) {
                    for attr in &field.attributes {
                        if let FieldAttribute::Relation {
                            name,
                            fields,
                            references,
                            on_delete,
                            on_update,
                            ..
                        } = attr
                        {
                            return Ok(ResolvedFieldType::Relation(RelationIr {
                                name: name.clone(),
                                target_model: type_name.clone(),
                                fields: fields
                                    .as_ref()
                                    .map(|f| f.iter().map(|i| i.value.clone()).collect())
                                    .unwrap_or_default(),
                                references: references
                                    .as_ref()
                                    .map(|r| r.iter().map(|i| i.value.clone()).collect())
                                    .unwrap_or_default(),
                                on_delete: *on_delete,
                                on_update: *on_update,
                                join: None,
                            }));
                        }
                    }

                    return Ok(ResolvedFieldType::Relation(RelationIr {
                        name: None,
                        target_model: type_name.clone(),
                        fields: vec![],
                        references: vec![],
                        on_delete: None,
                        on_update: None,
                        join: None,
                    }));
                }

                Err(SchemaError::Validation(
                    format!("Unknown type '{}'", type_name),
                    field.span,
                ))
            }
        }
    }

    pub(in crate::validator) fn extract_default_value(
        &self,
        field: &FieldDecl,
    ) -> Result<Option<DefaultValue>> {
        for attr in &field.attributes {
            if let FieldAttribute::Default(expr, _) = attr {
                return Ok(Some(self.expr_to_default_value(expr)?));
            }
        }
        Ok(None)
    }

    pub(in crate::validator) fn expr_to_default_value(&self, expr: &Expr) -> Result<DefaultValue> {
        match expr {
            Expr::Literal(Literal::String(s, _)) => Ok(DefaultValue::String(s.clone())),
            Expr::Literal(Literal::Number(n, _)) => Ok(DefaultValue::Number(n.clone())),
            Expr::Literal(Literal::Boolean(b, _)) => Ok(DefaultValue::Boolean(*b)),
            Expr::Ident(ident) => Ok(DefaultValue::EnumVariant(ident.value.clone())),
            Expr::Array { elements, .. } => Ok(DefaultValue::Array(
                elements
                    .iter()
                    .map(|element| self.expr_to_default_value(element))
                    .collect::<Result<Vec<_>>>()?,
            )),
            Expr::FunctionCall { name, args, .. } => Ok(DefaultValue::Function(FunctionCall {
                name: name.value.clone(),
                args: args
                    .iter()
                    .filter_map(|arg| {
                        if let Expr::Literal(Literal::String(s, _)) = arg {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
            })),
            _ => Err(SchemaError::Validation(
                "Unsupported default value expression".to_string(),
                expr.span(),
            )),
        }
    }
}
