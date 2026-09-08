//! The `datasource` block: which provider it names, the URLs it carries, and
//! the PostgreSQL schemas and extensions it declares.

use crate::validator::*;

impl SchemaValidator<'_> {
    pub(in crate::validator) fn validate_datasources(&mut self) {
        let datasources: Vec<_> = self
            .schema
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                Declaration::Datasource(datasource) => Some(datasource.clone()),
                _ => None,
            })
            .collect();

        self.reject_extra_blocks(
            "datasource",
            &datasources
                .iter()
                .map(|datasource| (datasource.name.value.clone(), datasource.name.span))
                .collect::<Vec<_>>(),
        );

        for datasource in &datasources {
            self.validate_datasource(datasource);
        }
    }

    /// Report every block after the first: a schema has exactly one
    /// `datasource` and one `generator`.
    ///
    /// This is what catches an `import` that reached another schema's root
    /// file, where the second block arrives from a file the developer was not
    /// looking at; the first block is named rather than located because its
    /// offset belongs to the assembled source, not to any one file.
    pub(in crate::validator) fn validate_datasource(&mut self, datasource: &DatasourceDecl) {
        for field in &datasource.fields {
            if !KNOWN_DATASOURCE_FIELDS.contains(&field.name.value.as_str()) {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Unknown field '{}' in datasource block. Valid fields: {}",
                        field.name.value,
                        KNOWN_DATASOURCE_FIELDS.join(", ")
                    ),
                    field.span,
                ));
            }
        }

        if let Err(err) = Self::datasource_provider_value(datasource) {
            self.errors.push_back(err);
        }

        if let Err(err) = Self::datasource_url_value(datasource) {
            self.errors.push_back(err);
        }

        if let Err(err) = Self::datasource_direct_url_value(datasource) {
            self.errors.push_back(err);
        }

        self.validate_datasource_extensions(datasource);
        self.validate_datasource_preserve_extensions(datasource);
        self.validate_datasource_schemas(datasource);
    }

    /// `schemas = ["public", "analytics"]` — the PostgreSQL schemas the
    /// datasource spans.
    pub(in crate::validator) fn validate_datasource_schemas(
        &mut self,
        datasource: &DatasourceDecl,
    ) {
        let Some(field) = datasource.find_field("schemas") else {
            return;
        };

        let provider_is_postgres = Self::datasource_provider_value(datasource)
            .ok()
            .and_then(|p| p.parse::<DatabaseProvider>().ok())
            .is_some_and(|p| p == DatabaseProvider::Postgres);

        if !provider_is_postgres {
            self.errors.push_back(SchemaError::Validation(
                "Datasource field 'schemas' is only supported for the 'postgresql' provider"
                    .to_string(),
                field.span,
            ));
            return;
        }

        let Expr::Array { elements, .. } = &field.value else {
            self.errors.push_back(SchemaError::Validation(
                "Datasource 'schemas' must be an array of string literals".to_string(),
                field.span,
            ));
            return;
        };

        if elements.is_empty() {
            self.errors.push_back(SchemaError::Validation(
                "Datasource 'schemas' must list at least one schema".to_string(),
                field.span,
            ));
            return;
        }

        let mut seen: HashSet<String> = HashSet::new();
        for element in elements {
            let Expr::Literal(Literal::String(name, span)) = element else {
                self.errors.push_back(SchemaError::Validation(
                    "Datasource 'schemas' entries must be string literals".to_string(),
                    element.span(),
                ));
                continue;
            };

            if name.trim().is_empty() {
                self.errors.push_back(SchemaError::Validation(
                    "Datasource 'schemas' entries must not be empty".to_string(),
                    *span,
                ));
                continue;
            }

            if !seen.insert(name.clone()) {
                self.errors.push_back(SchemaError::Validation(
                    format!("Duplicate schema '{}' in datasource 'schemas'", name),
                    *span,
                ));
            }
        }
    }

    /// The declared schema list, in declaration order and deduplicated.
    ///
    /// Assumes [`validate_datasource_schemas`](Self::validate_datasource_schemas)
    /// has already reported structural problems: malformed entries are skipped.
    pub(in crate::validator) fn datasource_schemas_value(
        datasource: &DatasourceDecl,
    ) -> Vec<String> {
        let Some(field) = datasource.find_field("schemas") else {
            return Vec::new();
        };
        let Expr::Array { elements, .. } = &field.value else {
            return Vec::new();
        };

        let mut schemas: Vec<String> = Vec::new();
        for element in elements {
            if let Expr::Literal(Literal::String(name, _)) = element {
                if !name.trim().is_empty() && !schemas.iter().any(|s| s == name) {
                    schemas.push(name.clone());
                }
            }
        }
        schemas
    }

    pub(in crate::validator) fn validate_datasource_extensions(
        &mut self,
        datasource: &DatasourceDecl,
    ) {
        let Some(field) = datasource.find_field("extensions") else {
            return;
        };

        let provider_is_postgres = Self::datasource_provider_value(datasource)
            .ok()
            .and_then(|p| p.parse::<DatabaseProvider>().ok())
            .is_some_and(|p| p == DatabaseProvider::Postgres);

        if !provider_is_postgres {
            self.errors.push_back(SchemaError::Validation(
                "Datasource field 'extensions' is only supported for the \
                 'postgresql' provider"
                    .to_string(),
                field.span,
            ));
            return;
        }

        let Expr::Array { elements, .. } = &field.value else {
            self.errors.push_back(SchemaError::Validation(
                "Datasource 'extensions' must be an array of identifiers or \
                 string literals (e.g. [pg_trgm, \"uuid-ossp\"])"
                    .to_string(),
                field.span,
            ));
            return;
        };

        let mut seen: HashSet<String> = HashSet::new();
        for element in elements {
            let parsed = Self::parse_extension_entry(element);
            let (name, span) = match parsed {
                Ok(entry) => (entry.name, entry.span),
                Err(err) => {
                    self.errors.push_back(err);
                    continue;
                }
            };

            let normalized = name.to_lowercase();
            if normalized.is_empty() {
                self.errors.push_back(SchemaError::Validation(
                    "Extension name must not be empty".to_string(),
                    span,
                ));
                continue;
            }

            if !seen.insert(normalized.clone()) {
                self.errors.push_back(SchemaError::Validation(
                    format!("Duplicate extension '{}' in datasource", normalized),
                    span,
                ));
                continue;
            }

            if !KNOWN_POSTGRES_EXTENSIONS.contains(&normalized.as_str()) {
                self.warnings.push_back(SchemaError::Warning(
                    format!(
                        "Extension '{}' is not in Nautilus' curated list of \
                         supported PostgreSQL extensions. It will still be \
                         installed via CREATE EXTENSION IF NOT EXISTS, but \
                         Nautilus has not verified its availability",
                        normalized
                    ),
                    span,
                ));
            }
        }
    }

    /// Parse a single `extensions = [...]` array entry into a (name, span)
    /// pair. Accepts three forms:
    ///
    /// - `pg_trgm` (bare identifier)
    /// - `"uuid-ossp"` (quoted string literal)
    /// - `extension(name = vector, schema = "extensions")` (structured)
    pub(in crate::validator) fn parse_extension_entry(expr: &Expr) -> Result<ParsedExtensionEntry> {
        match expr {
            Expr::Ident(ident) => Ok(ParsedExtensionEntry {
                name: ident.value.clone(),
                schema: None,
                span: ident.span,
            }),
            Expr::Literal(Literal::String(s, span)) => Ok(ParsedExtensionEntry {
                name: s.clone(),
                schema: None,
                span: *span,
            }),
            Expr::FunctionCall { name, args, span } if name.value == "extension" => {
                let mut ext_name: Option<String> = None;
                let mut schema: Option<String> = None;
                let mut saw_positional = false;

                for (idx, arg) in args.iter().enumerate() {
                    match arg {
                        Expr::NamedArg {
                            name: arg_name,
                            value,
                            span: arg_span,
                        } => match arg_name.value.as_str() {
                            "name" => {
                                ext_name = Some(extract_extension_string(value, *arg_span)?);
                            }
                            "schema" => {
                                schema = Some(extract_extension_string(value, *arg_span)?);
                            }
                            other => {
                                return Err(SchemaError::Validation(
                                    format!(
                                        "Unknown 'extension(...)' argument '{}'. \
                                         Supported: name, schema",
                                        other
                                    ),
                                    *arg_span,
                                ));
                            }
                        },
                        _ if idx == 0 && !saw_positional => {
                            saw_positional = true;
                            ext_name = Some(extract_extension_string(arg, arg.span())?);
                        }
                        _ => {
                            return Err(SchemaError::Validation(
                                "'extension(...)' arguments after the first must \
                                 be named (e.g. schema = \"extensions\")"
                                    .to_string(),
                                arg.span(),
                            ));
                        }
                    }
                }

                let Some(name) = ext_name else {
                    return Err(SchemaError::Validation(
                        "'extension(...)' requires a 'name' argument".to_string(),
                        *span,
                    ));
                };

                Ok(ParsedExtensionEntry {
                    name,
                    schema,
                    span: *span,
                })
            }
            Expr::FunctionCall { name, span, .. } => Err(SchemaError::Validation(
                format!(
                    "Unsupported extension entry '{}(...)'. Use an identifier, \
                     a string literal, or the 'extension(name = ..., schema = ...)' form",
                    name.value
                ),
                *span,
            )),
            other => Err(SchemaError::Validation(
                "Extension entries must be identifiers, string literals, or \
                 'extension(name = ..., schema = ...)' calls"
                    .to_string(),
                other.span(),
            )),
        }
    }

    pub(in crate::validator) fn validate_datasource_preserve_extensions(
        &mut self,
        datasource: &DatasourceDecl,
    ) {
        let Some(field) = datasource.find_field("preserve_extensions") else {
            return;
        };

        let provider_is_postgres = Self::datasource_provider_value(datasource)
            .ok()
            .and_then(|p| p.parse::<DatabaseProvider>().ok())
            .is_some_and(|p| p == DatabaseProvider::Postgres);

        if !provider_is_postgres {
            self.errors.push_back(SchemaError::Validation(
                "Datasource field 'preserve_extensions' is only supported for the \
                 'postgresql' provider"
                    .to_string(),
                field.span,
            ));
            return;
        }

        if !matches!(field.value, Expr::Literal(Literal::Boolean(_, _))) {
            self.errors.push_back(SchemaError::Validation(
                "Datasource 'preserve_extensions' must be a boolean literal".to_string(),
                field.span,
            ));
        }
    }

    pub(in crate::validator) fn datasource_provider_value(
        datasource: &DatasourceDecl,
    ) -> Result<String> {
        let provider_field = datasource.find_field("provider").ok_or_else(|| {
            SchemaError::Validation(
                "Datasource missing required 'provider' field".to_string(),
                datasource.span,
            )
        })?;

        let provider = if let Expr::Literal(Literal::String(s, _)) = &provider_field.value {
            s.clone()
        } else {
            return Err(SchemaError::Validation(
                "Datasource 'provider' must be a string literal".to_string(),
                provider_field.span,
            ));
        };

        if provider.parse::<DatabaseProvider>().is_err() {
            return Err(SchemaError::Validation(
                format!(
                    "Unknown datasource provider '{}'. Valid providers: {}",
                    provider,
                    DatabaseProvider::ALL.join(", ")
                ),
                provider_field.span,
            ));
        }

        Ok(provider)
    }

    pub(in crate::validator) fn datasource_url_value(
        datasource: &DatasourceDecl,
    ) -> Result<String> {
        Self::datasource_optional_url_value(datasource, "url")?.ok_or_else(|| {
            SchemaError::Validation(
                "Datasource missing required 'url' field".to_string(),
                datasource.span,
            )
        })
    }

    pub(in crate::validator) fn datasource_direct_url_value(
        datasource: &DatasourceDecl,
    ) -> Result<Option<String>> {
        Self::datasource_optional_url_value(datasource, "direct_url")
    }

    pub(in crate::validator) fn datasource_preserve_extensions_value(
        datasource: &DatasourceDecl,
    ) -> Result<bool> {
        let Some(field) = datasource.find_field("preserve_extensions") else {
            return Ok(false);
        };

        match &field.value {
            Expr::Literal(Literal::Boolean(value, _)) => Ok(*value),
            _ => Err(SchemaError::Validation(
                "Datasource 'preserve_extensions' must be a boolean literal".to_string(),
                field.span,
            )),
        }
    }

    fn datasource_optional_url_value(
        datasource: &DatasourceDecl,
        field_name: &str,
    ) -> Result<Option<String>> {
        let Some(url_field) = datasource.find_field(field_name) else {
            return Ok(None);
        };

        match &url_field.value {
            Expr::Literal(Literal::String(s, _)) => Ok(Some(s.clone())),
            Expr::FunctionCall { name, args, .. } if name.value == "env" => match args.as_slice() {
                [Expr::Literal(Literal::String(var_name, _))] => {
                    Ok(Some(format!("env({})", var_name)))
                }
                _ => Err(SchemaError::Validation(
                    format!(
                        "Datasource '{}' env() call requires a single string argument",
                        field_name
                    ),
                    url_field.span,
                )),
            },
            _ => Err(SchemaError::Validation(
                format!(
                    "Datasource '{}' must be a string literal or env() call",
                    field_name
                ),
                url_field.span,
            )),
        }
    }
}

/// Result of parsing a single array entry in `extensions = [...]`.
#[derive(Debug, Clone)]
pub(in crate::validator) struct ParsedExtensionEntry {
    pub(in crate::validator) name: String,
    pub(in crate::validator) schema: Option<String>,
    pub(in crate::validator) span: Span,
}

fn extract_extension_string(expr: &Expr, span: Span) -> Result<String> {
    match expr {
        Expr::Ident(ident) => Ok(ident.value.clone()),
        Expr::Literal(Literal::String(s, _)) => Ok(s.clone()),
        _ => Err(SchemaError::Validation(
            "Extension argument must be an identifier or string literal".to_string(),
            span,
        )),
    }
}
