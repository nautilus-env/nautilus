//! Diff passes for PostgreSQL user-defined types: composite types and enums.

use nautilus_schema::ir::SchemaIr;

use super::DiffAccumulator;
use crate::change::Change;
use crate::ddl::DatabaseProvider;
use crate::live::LiveSchema;

impl DiffAccumulator {
    /// Diff PostgreSQL composite types: creations, drops, and field-level
    /// alterations.
    ///
    /// No-op on other providers.
    pub(super) fn diff_composite_types(&mut self, live: &LiveSchema, target: &SchemaIr) {
        if self.provider != DatabaseProvider::Postgres {
            return;
        }

        for ct in target.composite_types.values() {
            let db_name = ct.db_name.clone();
            if !live.composite_types.contains_key(&db_name) {
                self.pre_type
                    .push(Change::CreateCompositeType { name: db_name });
            }
        }

        for live_ct_name in live.composite_types.keys() {
            let still_in_target = target
                .composite_types
                .values()
                .any(|ct| ct.db_name == *live_ct_name);
            if !still_in_target {
                self.post_type.push(Change::DropCompositeType {
                    name: live_ct_name.clone(),
                });
            }
        }

        for ct in target.composite_types.values() {
            let db_name = ct.db_name.clone();
            let Some(live_ct) = live.composite_types.get(&db_name) else {
                continue;
            };

            let live_field_map: std::collections::HashMap<&str, &str> = live_ct
                .fields
                .iter()
                .map(|f| (f.name.as_str(), f.col_type.as_str()))
                .collect();
            let target_field_map: std::collections::HashMap<&str, String> = ct
                .fields
                .iter()
                .filter_map(|f| {
                    self.ddl
                        .column_type_sql_for_composite(f)
                        .ok()
                        .map(|t| (f.db_name.as_str(), t))
                })
                .collect();

            let mut added_fields: Vec<(String, String)> = Vec::new();
            let mut type_changed_fields: Vec<(String, String, String)> = Vec::new();
            let mut dropped_fields: Vec<String> = Vec::new();

            for (db_name_f, sql_type) in &target_field_map {
                match live_field_map.get(db_name_f) {
                    None => added_fields.push((db_name_f.to_string(), sql_type.clone())),
                    Some(&live_type) if live_type != sql_type.as_str() => {
                        type_changed_fields.push((
                            db_name_f.to_string(),
                            live_type.to_string(),
                            sql_type.clone(),
                        ));
                    }
                    _ => {}
                }
            }
            for live_field in &live_ct.fields {
                if !target_field_map.contains_key(live_field.name.as_str()) {
                    dropped_fields.push(live_field.name.clone());
                }
            }

            if added_fields.is_empty()
                && dropped_fields.is_empty()
                && type_changed_fields.is_empty()
            {
                continue;
            }

            let destructive = !dropped_fields.is_empty() || !type_changed_fields.is_empty();
            let change = Change::AlterCompositeType {
                name: db_name,
                added_fields,
                dropped_fields,
                type_changed_fields,
            };
            if destructive {
                self.post_type.push(change);
            } else {
                self.pre_type.push(change);
            }
        }
    }

    /// Diff PostgreSQL enum types: creations, drops, and variant-list changes.
    ///
    /// No-op on other providers.
    pub(super) fn diff_enums(&mut self, live: &LiveSchema, target: &SchemaIr) {
        if self.provider != DatabaseProvider::Postgres {
            return;
        }

        for enum_def in target.enums.values() {
            let db_name = enum_def.logical_name.to_lowercase();
            if !live.enums.contains_key(&db_name) {
                self.pre_type.push(Change::CreateEnum {
                    name: db_name,
                    variants: enum_def.variants.clone(),
                });
            }
        }

        for live_enum_name in live.enums.keys() {
            let still_in_target = target
                .enums
                .values()
                .any(|e| e.logical_name.to_lowercase() == *live_enum_name);
            if !still_in_target {
                self.post_type.push(Change::DropEnum {
                    name: live_enum_name.clone(),
                });
            }
        }

        for enum_def in target.enums.values() {
            let db_name = enum_def.logical_name.to_lowercase();
            let Some(live_variants) = live.enums.get(&db_name) else {
                continue;
            };

            let added: Vec<String> = enum_def
                .variants
                .iter()
                .filter(|v| !live_variants.contains(*v))
                .cloned()
                .collect();
            let removed: Vec<String> = live_variants
                .iter()
                .filter(|v| !enum_def.variants.contains(*v))
                .cloned()
                .collect();

            if added.is_empty() && removed.is_empty() {
                continue;
            }

            let destructive = !removed.is_empty();
            let change = Change::AlterEnum {
                name: db_name,
                added_variants: added,
                removed_variants: removed,
            };
            if destructive {
                self.post_type.push(change);
            } else {
                self.pre_type.push(change);
            }
        }
    }
}
