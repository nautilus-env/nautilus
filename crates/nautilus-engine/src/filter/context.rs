use super::*;

pub(super) fn build_logical_to_db_map(model: &ModelIr) -> HashMap<String, String> {
    model
        .fields
        .iter()
        .filter(|f| !matches!(f.field_type, ResolvedFieldType::Relation(_)))
        .flat_map(|f| {
            let mut entries = vec![(f.logical_name.clone(), f.db_name.clone())];
            if f.db_name != f.logical_name {
                entries.push((f.db_name.clone(), f.db_name.clone()));
            }
            entries
        })
        .collect()
}

pub(super) type NestedIncludeContext = (RelationMap, FieldTypeMap, HashMap<String, String>, String);
pub(super) type RelationFilterContext = (RelationMap, FieldTypeMap, HashMap<String, String>);

pub(super) fn nested_include_context(
    field: &str,
    relations: &RelationMap,
    models: Option<&HashMap<String, ModelIr>>,
) -> Result<Option<NestedIncludeContext>, ProtocolError> {
    let Some(rel_info) = relations.get(field) else {
        return Ok(None);
    };
    let Some(all_models) = models else {
        return Ok(None);
    };
    let Some(target_model) = all_models.get(&rel_info.target_logical_name) else {
        return Ok(None);
    };

    Ok(Some((
        crate::handlers::build_relation_map(target_model, all_models)?,
        crate::handlers::build_field_type_map(target_model),
        build_logical_to_db_map(target_model),
        rel_info.target_table.clone(),
    )))
}

pub(super) fn relation_filter_context(
    rel: &RelationInfo,
    models: Option<&HashMap<String, ModelIr>>,
) -> Result<Option<RelationFilterContext>, ProtocolError> {
    let Some(all_models) = models else {
        return Ok(None);
    };
    let Some(target_model) = all_models.get(&rel.target_logical_name) else {
        return Ok(None);
    };

    Ok(Some((
        crate::handlers::build_relation_map(target_model, all_models)?,
        crate::handlers::build_field_type_map(target_model),
        build_logical_to_db_map(target_model),
    )))
}
