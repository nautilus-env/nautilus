//! Serializer: converts a [`LiveSchema`] snapshot into canonical `.nautilus` source text.
//!
//! Used by `nautilus db pull` to introspect an existing database and emit a
//! schema file that can be fed back into `db push`.
//!
//! The work is in two halves. `naming`, `types` and `relations` analyse the
//! live schema and produce the logical names, scalar types and relation fields
//! it implies; `datasource`, `models` and `expressions` render those results as
//! source text. Nothing in the rendering half decides what a thing is called.

mod datasource;
mod expressions;
mod models;
mod naming;
mod relations;
mod types;

use std::collections::HashMap;

use crate::ddl::DatabaseProvider;
use crate::live::LiveSchema;
use nautilus_core::TableName;

use datasource::render_datasource_block;
use models::{
    render_composite_type_block, render_enum_block, render_model_block, render_view_block,
};
use naming::build_table_naming_contexts;
use relations::{
    build_back_relations, build_directional_relation_counts, build_forward_relations,
    build_many_to_many_ends, build_relation_pair_counts, find_join_tables,
};
use types::lift_inline_enums;

/// Naming mode for identifiers emitted by `db pull`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullNameCase {
    /// Preserve the current serializer behaviour.
    #[default]
    Auto,
    /// Render identifiers in `snake_case`.
    Snake,
    /// Render identifiers in `PascalCase`.
    Pascal,
}

/// Naming options used when serialising an introspected schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PullNamingOptions {
    /// Logical model name rendering mode.
    pub model_case: PullNameCase,
    /// Logical field name rendering mode.
    pub field_case: PullNameCase,
}

/// Convert a [`LiveSchema`] to a `.nautilus` schema source string.
///
/// * `live` - the introspected live database schema
/// * `provider` - which SQL dialect was used during introspection
/// * `url` - what to write as the datasource `url`: an `env("NAME")` reference
///   is emitted as-is, anything else is quoted as a literal connection string
pub fn serialize_live_schema(live: &LiveSchema, provider: DatabaseProvider, url: &str) -> String {
    serialize_live_schema_with_options(live, provider, url, PullNamingOptions::default())
}

/// Convert a [`LiveSchema`] to a `.nautilus` schema source string using custom
/// logical naming options for models and fields.
pub fn serialize_live_schema_with_options(
    live: &LiveSchema,
    provider: DatabaseProvider,
    url: &str,
    options: PullNamingOptions,
) -> String {
    // MySQL has no CREATE TYPE: an enum lives inline in the column type. Lift
    // those into real enum declarations so a pulled schema round-trips instead
    // of degrading every enum column to a plain string.
    let lifted = lift_inline_enums(live);
    let live = lifted.as_ref().unwrap_or(live);

    let mut parts: Vec<String> = vec![render_datasource_block(live, provider, url)];

    let mut composite_names: Vec<&String> = live.composite_types.keys().collect();
    composite_names.sort();
    for db_name in composite_names {
        parts.push(render_composite_type_block(live, db_name, options));
    }

    let mut enum_names: Vec<&String> = live.enums.keys().collect();
    enum_names.sort();
    for db_name in enum_names {
        parts.push(render_enum_block(db_name, &live.enums[db_name]));
    }

    let mut table_names: Vec<&TableName> = live.tables.keys().collect();
    table_names.sort();
    let mut view_names: Vec<&TableName> = live.views.keys().collect();
    view_names.sort();

    // A join table is the storage of a relation, not a model: it comes back as
    // an array field on each of the two models it links.
    let joins = find_join_tables(live, &table_names);
    table_names.retain(|name| !joins.iter().any(|join| join.name == **name));

    let table_naming = build_table_naming_contexts(live, &table_names, &view_names, options);
    let relation_pair_counts = build_relation_pair_counts(live, &table_names);
    let directional_relation_counts = build_directional_relation_counts(live, &table_names);
    let forward_relations = build_forward_relations(
        live,
        &table_names,
        &table_naming,
        &relation_pair_counts,
        options,
    );
    let back_relations = build_back_relations(
        live,
        &table_names,
        &table_naming,
        &forward_relations,
        &directional_relation_counts,
        options,
    );

    let many_to_many = build_many_to_many_ends(
        &joins,
        &table_naming,
        &forward_relations,
        &back_relations,
        options,
    );

    for table_name in &table_names {
        parts.push(render_model_block(
            live,
            table_name,
            &table_naming,
            slice_for(&forward_relations, table_name),
            slice_for(&back_relations, table_name),
            slice_for(&many_to_many, table_name),
        ));
    }

    for view_name in &view_names {
        parts.push(render_view_block(live, view_name, &table_naming));
    }

    let mut out = parts.join("\n\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub(super) fn slice_for<'a, T>(
    map: &'a HashMap<TableName, Vec<T>>,
    table_name: &TableName,
) -> &'a [T] {
    map.get(table_name).map(Vec::as_slice).unwrap_or(&[])
}

pub(super) fn escape_schema_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
