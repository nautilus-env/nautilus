//! A `model` or `view` declaration: its primary key, the schema it belongs to,
//! the shape of its fields, and the rules an `@@ignore`d declaration has to
//! keep.

use super::*;

impl SchemaValidator<'_> {
    pub(super) fn validate_models(&mut self) {
        let models: Vec<_> = self.schema.models().cloned().collect();
        for model in &models {
            self.validate_model(model);
            self.validate_model_schema(model);
        }
    }

    /// `@@schema("...")` must name one of the datasource's declared schemas,
    /// and in multi-schema mode every block must say which schema owns it.
    ///
    /// Requiring the attribute rather than defaulting to the first entry is
    /// deliberate: `search_path` decides where an unqualified name lands at
    /// runtime, so a silent default would let the diff and the query planner
    /// disagree about which table a model means.
    fn validate_model_schema(&mut self, model: &ModelDecl) {
        let declared: Vec<String> = self
            .schema
            .datasource()
            .map(Self::datasource_schemas_value)
            .unwrap_or_default();

        let attribute = model.attributes.iter().find_map(|attr| match attr {
            ModelAttribute::Schema { name, span } => Some((name.clone(), *span)),
            _ => None,
        });

        match (attribute, declared.is_empty()) {
            (Some((_, span)), true) => self.errors.push_back(SchemaError::Validation(
                format!(
                    "{} '{}' declares @@schema but the datasource has no 'schemas' list.                      Add `schemas = [...]` to the datasource block.",
                    model.keyword(),
                    model.name.value
                ),
                span,
            )),
            (Some((name, span)), false) if !declared.contains(&name) => {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Schema '{}' on {} '{}' is not declared in the datasource 'schemas' list ({})",
                        name,
                        model.keyword(),
                        model.name.value,
                        declared.join(", ")
                    ),
                    span,
                ))
            }
            (None, false) => self.errors.push_back(SchemaError::Validation(
                format!(
                    "{} '{}' must declare @@schema(\"...\") because the datasource lists                      multiple schemas ({})",
                    model.keyword(),
                    model.name.value,
                    declared.join(", ")
                ),
                model.name.span,
            )),
            _ => {}
        }
    }

    /// Every model must name its primary key.
    ///
    /// Inventing one from the first declared field gives the user a uniqueness
    /// constraint they never asked for, discovered only when a second row
    /// collides with it at runtime. A view is exempt: it names a relation the
    /// database owns, which need not have a key at all, so `db pull` cannot
    /// produce one either.
    fn validate_primary_key(&mut self, model: &ModelDecl) {
        if model.is_view {
            return;
        }

        let has_composite_id = model
            .attributes
            .iter()
            .any(|attr| matches!(attr, ModelAttribute::Id(fields) if !fields.is_empty()));
        let has_field_id = model.fields.iter().any(|field| {
            field
                .attributes
                .iter()
                .any(|attr| matches!(attr, FieldAttribute::Id))
        });

        if has_composite_id || has_field_id {
            return;
        }

        self.errors.push_back(SchemaError::Validation(
            format!(
                "{} '{}' has no primary key. Mark a field with @id, or declare @@id([...]) for a composite key.",
                model.keyword(),
                model.name.value
            ),
            model.name.span,
        ));
    }

    pub(super) fn validate_model(&mut self, model: &ModelDecl) {
        self.validate_primary_key(model);

        let mut field_names = HashMap::new();
        for field in &model.fields {
            let name = &field.name.value;
            if field_names.contains_key(name) {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Duplicate field name '{}' in model '{}'",
                        name, model.name.value
                    ),
                    field.name.span,
                ));
            } else {
                field_names.insert(name.clone(), field.name.span);
            }

            self.validate_field_type(field, &model.name.value);

            if let FieldType::Decimal { precision, scale } = field.field_type {
                if precision == 0 {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "Decimal precision must be greater than 0, got {}",
                            precision
                        ),
                        field.span,
                    ));
                }
                if scale > precision {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "Decimal scale ({}) cannot exceed precision ({})",
                            scale, precision
                        ),
                        field.span,
                    ));
                }
            }

            if let FieldType::Vector { dimension } = field.field_type {
                if dimension == 0 {
                    self.errors.push_back(SchemaError::Validation(
                        "Vector dimension must be greater than 0".to_string(),
                        field.span,
                    ));
                }
                if dimension > 16000 {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "Vector dimension ({}) exceeds pgvector's maximum of 16000",
                            dimension
                        ),
                        field.span,
                    ));
                }
                if field.is_array() {
                    self.errors.push_back(SchemaError::Validation(
                        "Vector[] fields are not supported; use a scalar Vector(n) field"
                            .to_string(),
                        field.span,
                    ));
                }
            }
        }

        if model.has_composite_key() {
            for attr in &model.attributes {
                if let ModelAttribute::Id(fields) = attr {
                    for field_ident in fields {
                        match model.find_field(&field_ident.value) {
                            Some(field) => {
                                if field.is_array() {
                                    self.errors.push_back(SchemaError::Validation(
                                        format!(
                                            "Composite primary key field '{}' cannot be an array",
                                            field_ident.value
                                        ),
                                        field_ident.span,
                                    ));
                                }
                                if matches!(field.field_type, FieldType::UserType(_))
                                    && field.has_relation_attribute()
                                {
                                    self.errors.push_back(SchemaError::Validation(
                                        format!(
                                            "Composite primary key field '{}' cannot be a relation",
                                            field_ident.value
                                        ),
                                        field_ident.span,
                                    ));
                                }
                            }
                            None => {
                                self.errors.push_back(SchemaError::Validation(
                                    format!(
                                        "@@id references non-existent field '{}' in model '{}'",
                                        field_ident.value, model.name.value
                                    ),
                                    field_ident.span,
                                ));
                            }
                        }
                    }
                }
            }
        }

        for attr in &model.attributes {
            if let ModelAttribute::Unique(fields) = attr {
                for field_ident in fields {
                    if model.find_field(&field_ident.value).is_none() {
                        self.errors.push_back(SchemaError::Validation(
                            format!(
                                "@@unique references non-existent field '{}' in model '{}'",
                                field_ident.value, model.name.value
                            ),
                            field_ident.span,
                        ));
                    }
                }
            }
        }

        let provider = self
            .schema
            .datasource()
            .and_then(|ds| ds.provider())
            .and_then(|p| p.parse::<DatabaseProvider>().ok());
        for attr in &model.attributes {
            if let ModelAttribute::Index {
                fields,
                index_type,
                opclass,
                m,
                ef_construction,
                lists,
                predicate,
                span,
                ..
            } = attr
            {
                for field_ident in fields {
                    if model.find_field(&field_ident.value).is_none() {
                        self.errors.push_back(SchemaError::Validation(
                            format!(
                                "@@index references non-existent field '{}' in model '{}'",
                                field_ident.value, model.name.value
                            ),
                            field_ident.span,
                        ));
                    }
                }

                let indexed_field_type = fields
                    .first()
                    .and_then(|f| model.find_field(&f.value))
                    .map(|f| &f.field_type);

                let args = super::index::RawIndexArgs {
                    fields,
                    index_type: index_type.as_ref(),
                    opclass: opclass.as_ref(),
                    m: *m,
                    ef_construction: *ef_construction,
                    lists: *lists,
                    model_span: model.span,
                };

                let (_kind, diagnostics) = super::index::build_index_kind(
                    &args,
                    provider,
                    indexed_field_type,
                    &model.name.value,
                );
                for diag in diagnostics {
                    self.errors.push_back(diag);
                }

                if let Some(expr) = predicate {
                    let scalar_field_names = self.scalar_field_names(model);
                    for diag in super::index::validate_index_predicate(
                        expr,
                        *span,
                        provider,
                        &scalar_field_names,
                        &model.name.value,
                    ) {
                        self.errors.push_back(diag);
                    }
                }
            }
        }
    }

    /// Logical names of every field on `model` that maps to a column, i.e.
    /// everything except relation fields.
    pub(super) fn scalar_field_names<'m>(&self, model: &'m ModelDecl) -> Vec<&'m str> {
        model
            .fields
            .iter()
            .filter(|f| {
                !matches!(&f.field_type, FieldType::UserType(name) if self.models.contains_key(name))
            })
            .map(|f| f.name.value.as_str())
            .collect()
    }
}

impl SchemaValidator<'_> {
    /// Validate `@ignore` / `@@ignore`.
    ///
    /// An ignored declaration is one Nautilus does not manage at all: it never
    /// reaches a generated client, and migrations neither create nor drop it.
    /// That only stays coherent as long as nothing Nautilus *does* manage
    /// depends on it, which is what this pass enforces.
    pub(super) fn validate_ignored_declarations(&mut self) {
        let models: Vec<_> = self.schema.models().cloned().collect();
        let ignored_models: HashSet<&str> = models
            .iter()
            .filter(|model| model.is_ignored())
            .map(|model| model.name.value.as_str())
            .collect();

        for model in &models {
            if model.is_ignored() {
                continue;
            }
            self.reject_ignored_key_fields(model);
            self.require_ignored_model_for_unwritable_fields(model);
            self.reject_relations_into_ignored_models(model, &ignored_models);
        }
    }

    /// An ignored column is not created by migrations, so nothing that becomes
    /// part of the table's shape may reference it.
    fn reject_ignored_key_fields(&mut self, model: &ModelDecl) {
        for field in model.fields.iter().filter(|f| f.is_ignored()) {
            if field.attributes.iter().any(|attr| {
                matches!(
                    attr,
                    FieldAttribute::Id | FieldAttribute::Unique | FieldAttribute::Relation { .. }
                )
            }) {
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Field '{}' in model '{}' cannot combine @ignore with @id, @unique or @relation",
                        field.name.value, model.name.value
                    ),
                    field.span,
                ));
            }
        }

        let ignored: HashSet<&str> = model
            .fields
            .iter()
            .filter(|f| f.is_ignored())
            .map(|f| f.name.value.as_str())
            .collect();
        if ignored.is_empty() {
            return;
        }

        for attr in &model.attributes {
            let (label, fields) = match attr {
                ModelAttribute::Id(fields) => ("@@id", fields),
                ModelAttribute::Unique(fields) => ("@@unique", fields),
                ModelAttribute::Index { fields, .. } => ("@@index", fields),
                _ => continue,
            };
            for field_ident in fields {
                if ignored.contains(field_ident.value.as_str()) {
                    self.errors.push_back(SchemaError::Validation(
                        format!(
                            "{} references '{}' in model '{}', which is @ignore'd",
                            label, field_ident.value, model.name.value
                        ),
                        field_ident.span,
                    ));
                }
            }
        }
    }

    /// A required column with no default that Nautilus does not manage makes
    /// the whole model unwritable: every generated `create` would omit it and
    /// the database would reject the insert. The model has to be `@@ignore`d
    /// too, which is exactly what `db pull` emits for such a table.
    fn require_ignored_model_for_unwritable_fields(&mut self, model: &ModelDecl) {
        for field in model.fields.iter().filter(|f| f.is_ignored()) {
            let has_default = field
                .attributes
                .iter()
                .any(|attr| matches!(attr, FieldAttribute::Default(_, _)));
            if field.is_optional() || field.is_array() || has_default {
                continue;
            }

            self.errors.push_back(SchemaError::Validation(
                format!(
                    "Field '{}' in model '{}' is @ignore'd but required and has no @default, so no \
                     row could ever be created. Give it a @default, make it optional, or add \
                     @@ignore to the model.",
                    field.name.value, model.name.value
                ),
                field.span,
            ));
        }
    }

    /// A relation into an ignored model would generate an `include` for a model
    /// the client does not have.
    fn reject_relations_into_ignored_models(
        &mut self,
        model: &ModelDecl,
        ignored_models: &HashSet<&str>,
    ) {
        for field in &model.fields {
            if field.is_ignored() {
                continue;
            }
            let FieldType::UserType(type_name) = &field.field_type else {
                continue;
            };
            if !ignored_models.contains(type_name.as_str()) {
                continue;
            }

            self.errors.push_back(SchemaError::Validation(
                format!(
                    "Field '{}' in model '{}' relates to '{}', which is @@ignore'd. Mark the field \
                     @ignore or drop @@ignore from '{}'.",
                    field.name.value, model.name.value, type_name, type_name
                ),
                field.span,
            ));
        }
    }
}
