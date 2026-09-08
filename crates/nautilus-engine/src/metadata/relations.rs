//! Relation resolution: turn a model's relation fields into the `RelationMap`
//! the filter parser and include planner consume.

use std::collections::HashMap;

use nautilus_core::TableName;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{ModelIr, ResolvedFieldType};

use super::naming::to_snake_case;
use crate::filter::{JoinTableInfo, RelationInfo, RelationMap};
use crate::metadata::model_table;

/// The physical column name of a model's logical field.
fn column_of(model: &ModelIr, logical_name: &str) -> Option<String> {
    model
        .find_field(logical_name)
        .map(|field| field.db_name.clone())
}

/// The join table of an implicit many-to-many, qualified by the schema of the
/// synthesised join model.
fn join_table_name(table: &str, models: &HashMap<String, ModelIr>) -> TableName {
    let schema = models.get(table).and_then(|model| model.schema.clone());
    TableName::with_schema(schema, table)
}

/// Build a `RelationMap` for the given model so that the filter parser can resolve
/// `some` / `none` / `every` predicates and `include` entries at runtime.
pub(crate) fn build_relation_map(
    model: &ModelIr,
    models: &HashMap<String, ModelIr>,
) -> Result<RelationMap, ProtocolError> {
    let mut map = RelationMap::new();

    for field in model.relation_fields() {
        if let ResolvedFieldType::Relation(rel) = &field.field_type {
            let target_logical_name = rel.target_model.clone();

            if let Some(target_model) = models.get(&target_logical_name) {
                if let Some(join) = &rel.join {
                    let Some(pk_db) = column_of(model, &join.self_reference) else {
                        continue;
                    };
                    let Some(fk_db) = column_of(target_model, &join.target_reference) else {
                        continue;
                    };
                    map.insert(
                        to_snake_case(&field.logical_name),
                        RelationInfo {
                            parent_table: model.db_name.clone(),
                            target_logical_name,
                            target_table: model_table(target_model),
                            fk_db,
                            pk_db,
                            is_array: true,
                            via: Some(JoinTableInfo {
                                table: join_table_name(&join.table, models),
                                parent_column: join.self_column.clone(),
                                child_column: join.target_column.clone(),
                            }),
                        },
                    );
                    continue;
                }

                // Resolve (fk_db, pk_db) based on which side carries the FK.
                let (fk_db, pk_db) = if rel.fields.is_empty() {
                    // Array / many-side: FK is in the target model.
                    // Find the inverse relation (the FK side) in the target model.
                    let matching_name = rel.name.as_deref();
                    let candidates: Vec<_> = target_model
                        .relation_fields()
                        .filter_map(|candidate_field| {
                            let ResolvedFieldType::Relation(inv_rel) = &candidate_field.field_type
                            else {
                                return None;
                            };
                            if inv_rel.target_model != model.logical_name
                                || inv_rel.fields.is_empty()
                            {
                                return None;
                            }
                            if let Some(name) = matching_name {
                                if inv_rel.name.as_deref() != Some(name) {
                                    return None;
                                }
                            }
                            Some((candidate_field, inv_rel))
                        })
                        .collect();

                    let inverse = match candidates.len() {
                        0 if matching_name.is_some() => {
                            return Err(ProtocolError::QueryPlanning(format!(
                                "Relation '{}.{}' expects inverse relation name '{}' on model '{}', but no matching FK-side relation was found",
                                model.logical_name,
                                field.logical_name,
                                matching_name.unwrap_or_default(),
                                target_model.logical_name,
                            )));
                        }
                        0 => None,
                        1 => Some(candidates[0]),
                        _ => {
                            let relation_hint = matching_name
                                .map(|name| format!(" named '{}'", name))
                                .unwrap_or_default();
                            return Err(ProtocolError::QueryPlanning(format!(
                                "Relation '{}.{}' has ambiguous inverse relation{} on model '{}'",
                                model.logical_name,
                                field.logical_name,
                                relation_hint,
                                target_model.logical_name,
                            )));
                        }
                    };

                    if let Some((_, inv_rel)) = inverse {
                        let fk = inv_rel
                            .fields
                            .first()
                            .and_then(|name| {
                                target_model
                                    .fields
                                    .iter()
                                    .find(|f| &f.logical_name == name)
                                    .map(|f| f.db_name.clone())
                            })
                            .unwrap_or_default();
                        let pk = inv_rel
                            .references
                            .first()
                            .and_then(|name| {
                                model
                                    .fields
                                    .iter()
                                    .find(|f| &f.logical_name == name)
                                    .map(|f| f.db_name.clone())
                            })
                            .unwrap_or_default();
                        (fk, pk)
                    } else {
                        (String::new(), String::new())
                    }
                } else {
                    // FK-side: rel.fields = FK logical names in this model,
                    // rel.references = referenced (PK) logical names in target model.
                    // For EXISTS from this model into target:
                    //   EXISTS (SELECT * FROM target WHERE target.ref_col = this.fk_col)
                    let fk = rel
                        .references
                        .first()
                        .and_then(|name| {
                            target_model
                                .fields
                                .iter()
                                .find(|f| &f.logical_name == name)
                                .map(|f| f.db_name.clone())
                        })
                        .unwrap_or_default();
                    let pk = rel
                        .fields
                        .first()
                        .and_then(|name| {
                            model
                                .fields
                                .iter()
                                .find(|f| &f.logical_name == name)
                                .map(|f| f.db_name.clone())
                        })
                        .unwrap_or_default();
                    (fk, pk)
                };

                if !fk_db.is_empty() && !pk_db.is_empty() {
                    // Use snake_case logical name as the map key (matches JSON field names)
                    let field_key = to_snake_case(&field.logical_name);
                    map.insert(
                        field_key,
                        RelationInfo {
                            parent_table: model.db_name.clone(),
                            target_logical_name,
                            target_table: model_table(target_model),
                            fk_db,
                            pk_db,
                            is_array: field.is_array,
                            via: None,
                        },
                    );
                }
            }
        }
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_schema::ir::{FieldIr, PrimaryKeyIr, RelationIr, ScalarType};
    use nautilus_schema::{validate_schema_source, Span};

    fn parse_ir(source: &str) -> nautilus_schema::ir::SchemaIr {
        validate_schema_source(source)
            .expect("validation failed")
            .ir
    }

    fn scalar_field(logical: &str, db: &str) -> FieldIr {
        FieldIr {
            logical_name: logical.to_string(),
            db_name: db.to_string(),
            field_type: ResolvedFieldType::Scalar(ScalarType::Int),
            is_required: true,
            is_array: false,
            storage_strategy: None,
            default_value: None,
            is_unique: false,
            is_updated_at: false,
            computed: None,
            check: None,
            span: Span::new(0, 0),
            is_ignored: false,
        }
    }

    fn relation_field(
        logical: &str,
        target_model: &str,
        fields: &[&str],
        references: &[&str],
        name: Option<&str>,
        is_array: bool,
    ) -> FieldIr {
        FieldIr {
            logical_name: logical.to_string(),
            db_name: logical.to_string(),
            field_type: ResolvedFieldType::Relation(RelationIr {
                name: name.map(str::to_string),
                target_model: target_model.to_string(),
                fields: fields.iter().map(|s| (*s).to_string()).collect(),
                references: references.iter().map(|s| (*s).to_string()).collect(),
                on_delete: None,
                on_update: None,
                join: None,
            }),
            is_required: !is_array,
            is_array,
            storage_strategy: None,
            default_value: None,
            is_unique: false,
            is_updated_at: false,
            computed: None,
            check: None,
            span: Span::new(0, 0),
            is_ignored: false,
        }
    }

    #[test]
    fn build_relation_map_uses_relation_names_for_multiple_inverse_relations() {
        let schema = r#"
model User {
  id            Int    @id @default(autoincrement())
  authoredPosts Post[] @relation(name: "AuthoredPosts")
  reviewedPosts Post[] @relation(name: "ReviewedPosts")
}

model Post {
  id         Int  @id @default(autoincrement())
  authorId   Int  @map("author_id")
  reviewerId Int  @map("reviewer_id")
  author     User @relation(name: "AuthoredPosts", fields: [authorId], references: [id])
  reviewer   User @relation(name: "ReviewedPosts", fields: [reviewerId], references: [id])
}
"#;
        let ir = parse_ir(schema);
        let user_model = ir.models.get("User").expect("User model missing");
        let relation_map =
            build_relation_map(user_model, &ir.models).expect("relation map should build");

        let authored = relation_map
            .get("authored_posts")
            .expect("authored_posts relation missing");
        assert_eq!(authored.fk_db, "author_id");
        assert_eq!(authored.pk_db, "id");

        let reviewed = relation_map
            .get("reviewed_posts")
            .expect("reviewed_posts relation missing");
        assert_eq!(reviewed.fk_db, "reviewer_id");
        assert_eq!(reviewed.pk_db, "id");
    }

    #[test]
    fn build_relation_map_handles_named_self_relations() {
        let node_model = ModelIr {
            logical_name: "Node".to_string(),
            db_name: "nodes".to_string(),
            schema: None,
            fields: vec![
                scalar_field("id", "id"),
                scalar_field("parentId", "parent_id"),
                relation_field(
                    "parent",
                    "Node",
                    &["parentId"],
                    &["id"],
                    Some("Tree"),
                    false,
                ),
                relation_field("children", "Node", &[], &[], Some("Tree"), true),
            ],
            primary_key: PrimaryKeyIr::Single("id".to_string()),
            unique_constraints: vec![],
            indexes: vec![],
            check_constraints: vec![],
            span: Span::new(0, 0),
            is_ignored: false,
            is_view: false,
            is_join_table: false,
        };
        let mut models = HashMap::new();
        models.insert(node_model.logical_name.clone(), node_model.clone());
        let relation_map =
            build_relation_map(&node_model, &models).expect("relation map should build");

        let children = relation_map
            .get("children")
            .expect("children relation missing");
        assert_eq!(children.fk_db, "parent_id");
        assert_eq!(children.pk_db, "id");
    }

    #[test]
    fn build_relation_map_rejects_ambiguous_array_inverses() {
        let user_model = ModelIr {
            logical_name: "User".to_string(),
            db_name: "users".to_string(),
            schema: None,
            fields: vec![
                scalar_field("id", "id"),
                relation_field("posts", "Post", &[], &[], None, true),
            ],
            primary_key: PrimaryKeyIr::Single("id".to_string()),
            unique_constraints: vec![],
            indexes: vec![],
            check_constraints: vec![],
            span: Span::new(0, 0),
            is_ignored: false,
            is_view: false,
            is_join_table: false,
        };
        let post_model = ModelIr {
            logical_name: "Post".to_string(),
            db_name: "posts".to_string(),
            schema: None,
            fields: vec![
                scalar_field("id", "id"),
                scalar_field("authorId", "author_id"),
                scalar_field("reviewerId", "reviewer_id"),
                relation_field("author", "User", &["authorId"], &["id"], None, false),
                relation_field("reviewer", "User", &["reviewerId"], &["id"], None, false),
            ],
            primary_key: PrimaryKeyIr::Single("id".to_string()),
            unique_constraints: vec![],
            indexes: vec![],
            check_constraints: vec![],
            span: Span::new(0, 0),
            is_ignored: false,
            is_view: false,
            is_join_table: false,
        };

        let mut models = HashMap::new();
        models.insert(user_model.logical_name.clone(), user_model.clone());
        models.insert(post_model.logical_name.clone(), post_model);

        match build_relation_map(&user_model, &models) {
            Err(ProtocolError::QueryPlanning(message)) => {
                assert!(
                    message.contains("ambiguous inverse relation"),
                    "unexpected error message: {message}"
                );
            }
            Ok(map) => panic!("expected ambiguous inverse relation error, got {map:?}"),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
}
