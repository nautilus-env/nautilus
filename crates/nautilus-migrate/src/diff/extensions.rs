//! Diff passes for the objects declared on the datasource rather than on a
//! model: PostgreSQL schemas and extensions.

use nautilus_schema::ir::{PostgresExtensionIr, SchemaIr};

use super::DiffAccumulator;
use crate::change::Change;
use crate::ddl::DatabaseProvider;
use crate::live::LiveSchema;

impl DiffAccumulator {
    /// Emit a [`Change::CreateSchema`] for every declared PostgreSQL schema
    /// that the live database does not have yet.
    ///
    /// No-op on other providers, and on a single-schema datasource.
    pub(super) fn diff_schemas(&mut self, live: &LiveSchema, target: &SchemaIr) {
        if self.provider != DatabaseProvider::Postgres {
            return;
        }

        let Some(datasource) = target.datasource.as_ref() else {
            return;
        };

        for schema in &datasource.schemas {
            if !live.schemas.contains(schema) {
                self.pre_type.push(Change::CreateSchema {
                    name: schema.clone(),
                });
            }
        }
    }

    /// Diff the declared PostgreSQL extensions against the installed ones.
    ///
    /// No-op on other providers.
    pub(super) fn diff_extensions(&mut self, live: &LiveSchema, target: &SchemaIr) {
        if self.provider != DatabaseProvider::Postgres {
            return;
        }

        let target_extensions: &[PostgresExtensionIr] = target
            .datasource
            .as_ref()
            .map(|d| d.extensions.as_slice())
            .unwrap_or(&[]);
        let preserve_extensions = target
            .datasource
            .as_ref()
            .is_some_and(|d| d.preserve_extensions);
        let target_extensions_set: std::collections::HashSet<&str> =
            target_extensions.iter().map(|e| e.name.as_str()).collect();

        for ext in target_extensions {
            if !live.extensions.contains_key(&ext.name) {
                self.pre_type.push(Change::CreateExtension {
                    name: ext.name.clone(),
                    schema: ext.schema.clone(),
                });
            }
        }

        if !preserve_extensions {
            let mut live_extension_names: Vec<&str> =
                live.extensions.keys().map(String::as_str).collect();
            live_extension_names.sort_unstable();
            for live_ext in live_extension_names {
                if !target_extensions_set.contains(live_ext) {
                    self.post_type.push(Change::DropExtension {
                        name: live_ext.to_string(),
                    });
                }
            }
        }
    }
}
