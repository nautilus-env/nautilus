//! The schema an argument parser can reach while it walks a payload.
//!
//! [`SchemaContext`] carries what is known about the models at the top of a
//! request; the nested contexts below narrow it to one relation as the
//! `include` and `where` parsers descend into it.

use std::borrow::Cow;
use std::collections::HashMap;

use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::ModelIr;

use super::types::{FieldTypeMap, RelationInfo, RelationMap};
use crate::state::EngineState;

#[derive(Clone, Copy, Default)]
pub(crate) struct SchemaContext<'a> {
    models: Option<&'a HashMap<String, ModelIr>>,
    state: Option<&'a EngineState>,
}

impl<'a> SchemaContext<'a> {
    pub(crate) const fn none() -> Self {
        Self {
            models: None,
            state: None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn with_models(models: &'a HashMap<String, ModelIr>) -> Self {
        Self {
            models: Some(models),
            state: None,
        }
    }

    pub(crate) fn with_state(state: &'a EngineState) -> Self {
        Self {
            models: Some(state.models()),
            state: Some(state),
        }
    }

    pub(super) const fn models(self) -> Option<&'a HashMap<String, ModelIr>> {
        self.models
    }

    pub(super) const fn state(self) -> Option<&'a EngineState> {
        self.state
    }
}

pub(super) struct NestedIncludeContext<'a> {
    pub(super) relations: Cow<'a, RelationMap>,
    pub(super) field_types: Cow<'a, FieldTypeMap>,
    pub(super) logical_to_db: Cow<'a, HashMap<String, String>>,
    pub(super) target_table: Cow<'a, str>,
}

pub(super) struct RelationFilterContext<'a> {
    pub(super) relations: Cow<'a, RelationMap>,
    pub(super) field_types: Cow<'a, FieldTypeMap>,
    pub(super) logical_to_db: Cow<'a, HashMap<String, String>>,
}

pub(super) fn nested_include_context<'a>(
    field: &str,
    relations: &'a RelationMap,
    schema_context: SchemaContext<'a>,
) -> Result<Option<NestedIncludeContext<'a>>, ProtocolError> {
    let Some(rel_info) = relations.get(field) else {
        return Ok(None);
    };

    if let Some(state) = schema_context.state() {
        let Some((target_model, target_metadata)) =
            state.related_model(&rel_info.target_logical_name)
        else {
            return Ok(None);
        };

        return Ok(Some(NestedIncludeContext {
            relations: Cow::Borrowed(state.relation_map_for_model(target_model)?),
            field_types: Cow::Borrowed(target_metadata.field_types()),
            logical_to_db: Cow::Borrowed(target_metadata.logical_to_db()),
            target_table: Cow::Borrowed(rel_info.target_table.as_str()),
        }));
    }

    let Some(all_models) = schema_context.models() else {
        return Ok(None);
    };
    let Some(target_model) = all_models.get(&rel_info.target_logical_name) else {
        return Ok(None);
    };

    Ok(Some(NestedIncludeContext {
        relations: Cow::Owned(crate::metadata::build_relation_map(
            target_model,
            all_models,
        )?),
        field_types: Cow::Owned(crate::metadata::build_field_type_map(target_model)),
        logical_to_db: Cow::Owned(crate::metadata::build_logical_to_db_map(target_model)),
        target_table: Cow::Owned(rel_info.target_table.name.clone()),
    }))
}

pub(super) fn relation_filter_context<'a>(
    rel: &RelationInfo,
    schema_context: SchemaContext<'a>,
) -> Result<Option<RelationFilterContext<'a>>, ProtocolError> {
    if let Some(state) = schema_context.state() {
        let Some((target_model, target_metadata)) = state.related_model(&rel.target_logical_name)
        else {
            return Ok(None);
        };

        return Ok(Some(RelationFilterContext {
            relations: Cow::Borrowed(state.relation_map_for_model(target_model)?),
            field_types: Cow::Borrowed(target_metadata.field_types()),
            logical_to_db: Cow::Borrowed(target_metadata.logical_to_db()),
        }));
    }

    let Some(all_models) = schema_context.models() else {
        return Ok(None);
    };
    let Some(target_model) = all_models.get(&rel.target_logical_name) else {
        return Ok(None);
    };

    Ok(Some(RelationFilterContext {
        relations: Cow::Owned(crate::metadata::build_relation_map(
            target_model,
            all_models,
        )?),
        field_types: Cow::Owned(crate::metadata::build_field_type_map(target_model)),
        logical_to_db: Cow::Owned(crate::metadata::build_logical_to_db_map(target_model)),
    }))
}
