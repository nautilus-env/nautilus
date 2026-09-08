//! The declarations the IR names: enums, composite types and models, with the
//! primary key, unique constraints and indexes a model brings with it.

use crate::validator::*;

impl SchemaValidator<'_> {
    pub(in crate::validator) fn build_enum_ir(&self, enum_decl: &EnumDecl) -> EnumIr {
        EnumIr {
            logical_name: enum_decl.name.value.clone(),
            variants: enum_decl
                .variants
                .iter()
                .map(|v| v.name.value.clone())
                .collect(),
            span: enum_decl.span,
        }
    }

    pub(in crate::validator) fn build_composite_type_ir(
        &self,
        type_decl: &TypeDecl,
    ) -> Result<CompositeTypeIr> {
        let fields = type_decl
            .fields
            .iter()
            .map(|f| {
                let logical_name = f.name.value.clone();
                let db_name = f.column_name().to_string();
                let field_type = self.resolve_field_type(f)?;
                let is_required = !f.is_optional() && !f.is_array();
                let is_array = f.is_array();
                let storage_strategy = f.attributes.iter().find_map(|attr| {
                    if let FieldAttribute::Store { strategy, .. } = attr {
                        Some(*strategy)
                    } else {
                        None
                    }
                });
                Ok(CompositeFieldIr {
                    logical_name,
                    db_name,
                    field_type,
                    is_required,
                    is_array,
                    storage_strategy,
                    span: f.span,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(CompositeTypeIr {
            logical_name: type_decl.name.value.clone(),
            db_name: type_decl.db_type_name(),
            fields,
            span: type_decl.span,
        })
    }

    pub(in crate::validator) fn build_model_ir(&self, model: &ModelDecl) -> Result<ModelIr> {
        let logical_name = model.name.value.clone();
        let db_name = model.table_name().to_string();

        let fields = model
            .fields
            .iter()
            .map(|f| self.build_field_ir(f, model))
            .collect::<Result<Vec<_>>>()?;

        let primary_key = self.build_primary_key_ir(model);
        let unique_constraints = self.build_unique_constraints(model);
        let indexes = self.build_indexes(model);

        let field_name_map: std::collections::HashMap<String, String> = model
            .fields
            .iter()
            .map(|f| (f.name.value.clone(), f.column_name().to_string()))
            .collect();

        let check_constraints: Vec<String> = model
            .attributes
            .iter()
            .filter_map(|attr| match attr {
                ModelAttribute::Check { expr, .. } => Some(expr.to_sql_mapped(&|name: &str| {
                    field_name_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_string())
                })),
                _ => None,
            })
            .collect();

        Ok(ModelIr {
            logical_name,
            db_name,
            schema: model.schema_name().map(str::to_string),
            fields,
            primary_key,
            unique_constraints,
            indexes,
            check_constraints,
            is_ignored: model.is_ignored(),
            is_view: model.is_view,
            is_join_table: false,
            span: model.span,
        })
    }

    pub(in crate::validator) fn build_primary_key_ir(&self, model: &ModelDecl) -> PrimaryKeyIr {
        for attr in &model.attributes {
            if let ModelAttribute::Id(fields) = attr {
                let field_names = fields.iter().map(|f| f.value.clone()).collect();
                return PrimaryKeyIr::Composite(field_names);
            }
        }

        for field in &model.fields {
            for attr in &field.attributes {
                if matches!(attr, FieldAttribute::Id) {
                    return PrimaryKeyIr::Single(field.name.value.clone());
                }
            }
        }

        if let Some(first_field) = model.fields.first() {
            PrimaryKeyIr::Single(first_field.name.value.clone())
        } else {
            PrimaryKeyIr::Composite(vec![])
        }
    }

    pub(in crate::validator) fn build_unique_constraints(
        &self,
        model: &ModelDecl,
    ) -> Vec<UniqueConstraintIr> {
        let mut constraints = Vec::new();

        for field in &model.fields {
            for attr in &field.attributes {
                if matches!(attr, FieldAttribute::Unique) {
                    constraints.push(UniqueConstraintIr {
                        fields: vec![field.name.value.clone()],
                    });
                }
            }
        }

        for attr in &model.attributes {
            if let ModelAttribute::Unique(fields) = attr {
                constraints.push(UniqueConstraintIr {
                    fields: fields.iter().map(|f| f.value.clone()).collect(),
                });
            }
        }

        constraints
    }

    pub(in crate::validator) fn build_indexes(&self, model: &ModelDecl) -> Vec<IndexIr> {
        let mut indexes = Vec::new();

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
                name,
                map,
                predicate,
                ..
            } = attr
            {
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

                let (kind, _diagnostics) = super::index::build_index_kind(
                    &args,
                    provider,
                    indexed_field_type,
                    &model.name.value,
                );

                indexes.push(IndexIr {
                    fields: fields.iter().map(|f| f.value.clone()).collect(),
                    kind,
                    name: name.clone(),
                    map: map.clone(),
                    predicate: predicate.as_ref().map(|expr| {
                        expr.to_sql_mapped(&|name: &str| {
                            model
                                .find_field(name)
                                .map(|f| f.column_name().to_string())
                                .unwrap_or_else(|| name.to_string())
                        })
                    }),
                });
            }
        }

        indexes
    }
}
