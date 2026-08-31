use super::SchemaValidator;
use crate::ast::*;
use crate::error::SchemaError;
use std::collections::HashSet;

impl SchemaValidator<'_> {
    /// Validate `view` blocks.
    ///
    /// A view names a read-only relation that already exists in the database.
    /// Nautilus queries it, never creates, alters, drops or writes to it, and
    /// this pass rejects everything in the schema language that would assume
    /// otherwise.
    pub(super) fn validate_views(&mut self) {
        let models: Vec<_> = self.schema.models().cloned().collect();
        let views: HashSet<&str> = models
            .iter()
            .filter(|model| model.is_view)
            .map(|model| model.name.value.as_str())
            .collect();

        let model_names: HashSet<&str> = models
            .iter()
            .map(|model| model.name.value.as_str())
            .collect();

        for model in &models {
            if model.is_view {
                self.reject_unwritable_view_attributes(model);
                self.reject_relation_fields(model, &model_names);
            } else {
                self.reject_relations_to_views(model, &views);
            }
        }
    }

    fn reject_unwritable_view_attributes(&mut self, view: &ModelDecl) {
        for attr in &view.attributes {
            let (label, span) = match attr {
                ModelAttribute::Index { span, .. } => ("@@index", *span),
                ModelAttribute::Check { span, .. } => ("@@check", *span),
                _ => continue,
            };
            self.errors.push_back(SchemaError::Validation(
                format!(
                    "View '{}' cannot declare {}: a view has no storage of its own",
                    view.name.value, label
                ),
                span,
            ));
        }

        for field in &view.fields {
            for attr in &field.attributes {
                let (label, span) = match attr {
                    FieldAttribute::Default(_, span) => ("@default", *span),
                    FieldAttribute::UpdatedAt { span } => ("@updatedAt", *span),
                    FieldAttribute::Computed { span, .. } => ("@computed", *span),
                    FieldAttribute::Check { span, .. } => ("@check", *span),
                    _ => continue,
                };
                self.errors.push_back(SchemaError::Validation(
                    format!(
                        "Field '{}' in view '{}' cannot declare {}: a view is read-only",
                        field.name.value, view.name.value, label
                    ),
                    span,
                ));
            }
        }
    }

    fn reject_relation_fields(&mut self, view: &ModelDecl, model_names: &HashSet<&str>) {
        for field in &view.fields {
            let FieldType::UserType(target) = &field.field_type else {
                continue;
            };
            if !model_names.contains(target.as_str()) {
                continue;
            }
            self.errors.push_back(SchemaError::Validation(
                format!(
                    "Field '{}' in view '{}' relates to '{}': a view carries no foreign key, so it cannot take part in a relation",
                    field.name.value, view.name.value, target
                ),
                field.span,
            ));
        }
    }

    fn reject_relations_to_views(&mut self, model: &ModelDecl, views: &HashSet<&str>) {
        for field in &model.fields {
            let FieldType::UserType(target) = &field.field_type else {
                continue;
            };
            if !views.contains(target.as_str()) {
                continue;
            }
            self.errors.push_back(SchemaError::Validation(
                format!(
                    "Field '{}' in model '{}' points at view '{}': a view carries no foreign key, so it cannot take part in a relation",
                    field.name.value, model.name.value, target
                ),
                field.span,
            ));
        }
    }
}
