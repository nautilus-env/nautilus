use super::*;

impl SchemaValidator<'_> {
    pub(super) fn validate_defaults(&mut self) {
        let models: Vec<_> = self.schema.models().cloned().collect();
        for model in &models {
            for field in &model.fields {
                for attr in &field.attributes {
                    if let FieldAttribute::Default(expr, _) = attr {
                        self.validate_default_value(
                            field,
                            expr,
                            &field.name.value,
                            &model.name.value,
                        );
                    }
                }
            }
        }
    }

    pub(super) fn validate_default_value(
        &mut self,
        field: &FieldDecl,
        expr: &Expr,
        field_name: &str,
        model_name: &str,
    ) {
        let field_type = &field.field_type;

        if field.is_array() {
            match expr {
                Expr::Array { elements, span } => {
                    self.validate_array_default(
                        field_type, elements, *span, field_name, model_name,
                    );
                }
                _ => {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "Default value for array field '{}' in model '{}' must be an array literal",
                            field_name, model_name
                        ),
                        expr.span(),
                    ));
                }
            }
            return;
        }

        match expr {
            Expr::Literal(lit) => {
                self.validate_literal_default(field_type, lit, field_name, model_name);
            }
            Expr::Ident(ident) => {
                self.validate_ident_default(field_type, ident, field_name, model_name);
            }
            Expr::FunctionCall { name, args, span } => {
                self.validate_function_default(
                    field_type,
                    &name.value,
                    args,
                    *span,
                    field_name,
                    model_name,
                );
            }
            Expr::Array { .. } => {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Array default value can only be used with array field '{}' in model '{}'",
                        field_name, model_name
                    ),
                    expr.span(),
                ));
            }
            _ => {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Unsupported default value expression for field '{}' in model '{}'",
                        field_name, model_name
                    ),
                    expr.span(),
                ));
            }
        }
    }

    fn validate_array_default(
        &mut self,
        field_type: &FieldType,
        elements: &[Expr],
        span: Span,
        field_name: &str,
        model_name: &str,
    ) {
        if let FieldType::UserType(type_name) = field_type {
            if self.models.contains_key(type_name) {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Relation field '{}' in model '{}' cannot use @default",
                        field_name, model_name
                    ),
                    span,
                ));
                return;
            }

            if self.composite_types.contains_key(type_name) {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Array default for composite type field '{}' in model '{}' is not supported",
                        field_name, model_name
                    ),
                    span,
                ));
                return;
            }
        }

        for element in elements {
            match element {
                Expr::Literal(lit) => {
                    self.validate_literal_default(field_type, lit, field_name, model_name);
                }
                Expr::Ident(ident) => {
                    self.validate_ident_default(field_type, ident, field_name, model_name);
                }
                _ => {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "Array default for field '{}' in model '{}' can only contain literal values",
                            field_name, model_name
                        ),
                        element.span(),
                    ));
                }
            }
        }
    }

    fn validate_ident_default(
        &mut self,
        field_type: &FieldType,
        ident: &Ident,
        field_name: &str,
        model_name: &str,
    ) {
        let FieldType::UserType(type_name) = field_type else {
            self.push_non_enum_default_identifier_error(ident, field_name, model_name);
            return;
        };
        let Some(enum_decl) = self.schema.enums().find(|e| e.name.value == *type_name) else {
            self.push_non_enum_default_identifier_error(ident, field_name, model_name);
            return;
        };

        if enum_decl
            .variants
            .iter()
            .any(|variant| variant.name.value == ident.value)
        {
            return;
        }

        self.errors.push_back(SchemaError::Validation(
            format!(
                "Enum variant '{}' does not exist in enum '{}' for field '{}' in model '{}'",
                ident.value, type_name, field_name, model_name
            ),
            ident.span,
        ));
    }

    fn push_non_enum_default_identifier_error(
        &mut self,
        ident: &Ident,
        field_name: &str,
        model_name: &str,
    ) {
        self.errors.push_back(SchemaError::Validation(
            format!(
                "Default value for field '{}' in model '{}' uses identifier '{}' but field type is not an enum",
                field_name, model_name, ident.value
            ),
            ident.span,
        ));
    }

    pub(super) fn validate_literal_default(
        &mut self,
        field_type: &FieldType,
        lit: &Literal,
        field_name: &str,
        model_name: &str,
    ) {
        match (field_type, lit) {
            (FieldType::String, Literal::String(_, _)) => {}
            (FieldType::Boolean, Literal::Boolean(_, _)) => {}
            (
                FieldType::Int | FieldType::BigInt | FieldType::Float | FieldType::Decimal { .. },
                Literal::Number(_, _),
            ) => {}
            _ => {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Type mismatch: field '{}' in model '{}' has type {:?} but default value is {:?}",
                        field_name, model_name, field_type, lit
                    ),
                    lit.span(),
                ));
            }
        }
    }

    pub(super) fn validate_function_default(
        &mut self,
        field_type: &FieldType,
        func_name: &str,
        _args: &[Expr],
        span: Span,
        field_name: &str,
        model_name: &str,
    ) {
        match func_name {
            "autoincrement" => {
                if !matches!(field_type, FieldType::Int | FieldType::BigInt) {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "autoincrement() can only be used with Int or BigInt fields, but field '{}' in model '{}' has type {:?}",
                            field_name, model_name, field_type
                        ),
                        span,
                    ));
                }
            }
            "uuid" | "uuidv7" => {
                if !matches!(field_type, FieldType::Uuid) {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "{}() can only be used with Uuid fields, but field '{}' in model '{}' has type {:?}",
                            func_name, field_name, model_name, field_type
                        ),
                        span,
                    ));
                }

                if func_name == "uuidv7" {
                    let provider = self
                        .schema
                        .datasource()
                        .and_then(|datasource| datasource.provider())
                        .and_then(|provider| provider.parse::<DatabaseProvider>().ok());

                    match provider {
                        Some(provider) if !provider.supports_uuidv7_default() => {
                            self.errors.push_back(SchemaError::Validation(
                                format!(
                                    "uuidv7() defaults are not supported by provider '{}' (supported by: PostgreSQL)",
                                    provider
                                ),
                                span,
                            ));
                        }
                        Some(_) => {}
                        None => {
                            self.warnings.push_back(SchemaError::Warning(
                                "uuidv7() defaults are only supported by PostgreSQL, but the schema has no datasource with a recognized provider to validate against".to_string(),
                                span,
                            ));
                        }
                    }
                }
            }
            "now" => {
                if !matches!(field_type, FieldType::DateTime) {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "now() can only be used with DateTime fields, but field '{}' in model '{}' has type {:?}",
                            field_name, model_name, field_type
                        ),
                        span,
                    ));
                }
            }
            "env" => {}
            _ => {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Unknown function '{}' in default value for field '{}' in model '{}'",
                        func_name, field_name, model_name
                    ),
                    span,
                ));
            }
        }
    }
}
