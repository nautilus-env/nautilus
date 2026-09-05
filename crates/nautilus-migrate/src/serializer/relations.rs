//! Relation reconstruction.
//!
//! Foreign keys become forward relation fields, their inverses become back
//! relation fields, and a join table the database still carries becomes an
//! array field on each of the two models it links. The result is what the
//! renderers consume; none of it is rendered here.

use std::collections::{HashMap, HashSet};

use super::naming::{
    apply_derived_field_case, choose_unique_field_name, default_back_relation_field_name,
    pluralize_name, qualify_back_relation_field_name, relation_field_name_base, singular_name,
    to_snake_case_identifier, TableNamingContext,
};
use super::slice_for;
use super::PullNamingOptions;
use crate::live::{LiveForeignKey, LiveSchema, LiveTable};
use nautilus_core::TableName;

#[derive(Debug, Clone)]
pub(super) struct ForwardRelation {
    pub(super) fk_index: usize,
    pub(super) field_name: String,
    pub(super) relation_name: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct BackRelation {
    pub(super) owning_table: TableName,
    pub(super) field_name: String,
    pub(super) relation_name: Option<String>,
    pub(super) is_one_to_one: bool,
}

/// One end of an implicit many-to-many recovered from the live database.
pub(super) struct ManyToManyEnd {
    /// The live table on the other side of the join.
    pub(super) target_table: TableName,
    pub(super) field_name: String,
    /// Set when the join table's name does not spell the default
    /// `_<A model>To<B model>` for this pair, so the schema has to say which
    /// relation it is.
    pub(super) relation_name: Option<String>,
}

/// A live table that is the join table of an implicit many-to-many.
pub(super) struct JoinTable {
    pub(super) name: TableName,
    /// The table the `A` column points at.
    pub(super) a_table: TableName,
    /// The table the `B` column points at.
    pub(super) b_table: TableName,
}

/// Recognise the join tables among the live ones.
///
/// The shape is the one Nautilus creates and nothing else plausibly is: a name
/// starting with `_`, exactly the two required columns `A` and `B`, a primary
/// key over the pair, and one foreign key per column. Recovering them is what
/// makes `db pull` return the schema that produced the database rather than a
/// second, explicit spelling of it.
pub(super) fn find_join_tables(live: &LiveSchema, table_names: &[&TableName]) -> Vec<JoinTable> {
    let mut joins: Vec<JoinTable> = table_names
        .iter()
        .filter_map(|name| {
            let table = &live.tables[*name];
            if !name.name.starts_with('_') || table.primary_key != ["A", "B"] {
                return None;
            }
            let columns: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
            if columns != ["A", "B"] || table.columns.iter().any(|column| column.nullable) {
                return None;
            }

            let referenced = |column: &str| {
                table
                    .foreign_keys
                    .iter()
                    .find(|fk| fk.columns == [column])
                    .filter(|fk| live.tables.contains_key(&fk.referenced_table))
                    .map(|fk| fk.referenced_table.clone())
            };
            if table.foreign_keys.len() != 2 {
                return None;
            }

            Some(JoinTable {
                name: (*name).clone(),
                a_table: referenced("A")?,
                b_table: referenced("B")?,
            })
        })
        .collect();
    joins.sort_by(|a, b| a.name.cmp(&b.name));
    joins
}

pub(super) fn build_relation_pair_counts(
    live: &LiveSchema,
    table_names: &[&TableName],
) -> HashMap<(TableName, TableName), usize> {
    let mut counts = HashMap::new();
    for &table_name in table_names {
        for fk in &live.tables[table_name].foreign_keys {
            *counts
                .entry(relation_pair_key(table_name, &fk.referenced_table))
                .or_insert(0) += 1;
        }
    }
    counts
}

pub(super) fn build_directional_relation_counts(
    live: &LiveSchema,
    table_names: &[&TableName],
) -> HashMap<(TableName, TableName), usize> {
    let mut counts = HashMap::new();
    for &table_name in table_names {
        for fk in &live.tables[table_name].foreign_keys {
            *counts
                .entry((table_name.clone(), fk.referenced_table.clone()))
                .or_insert(0) += 1;
        }
    }
    counts
}

pub(super) fn build_forward_relations(
    live: &LiveSchema,
    table_names: &[&TableName],
    table_naming: &HashMap<TableName, TableNamingContext>,
    relation_pair_counts: &HashMap<(TableName, TableName), usize>,
    options: PullNamingOptions,
) -> HashMap<TableName, Vec<ForwardRelation>> {
    let mut result = HashMap::new();

    for &table_name in table_names {
        let table = &live.tables[table_name];
        let mut used_fields: HashSet<String> = table_naming[table_name]
            .logical_field_order
            .iter()
            .cloned()
            .collect();
        let mut relations = Vec::new();

        for (fk_index, fk) in table.foreign_keys.iter().enumerate() {
            let base_name = relation_field_name_base(&fk.columns, &fk.referenced_table.name);
            let fallback_name = apply_derived_field_case(
                &to_snake_case_identifier(&singular_name(&fk.referenced_table.name)),
                options.field_case,
            );
            let mut candidates = vec![apply_derived_field_case(&base_name, options.field_case)];
            if fallback_name != candidates[0] {
                candidates.push(fallback_name);
            }
            if let Some(first_col) = fk.columns.first() {
                let qualified = apply_derived_field_case(
                    &format!("{}_{}", base_name, to_snake_case_identifier(first_col)),
                    options.field_case,
                );
                if qualified != candidates[0] {
                    candidates.push(qualified);
                }
            }

            let field_name = choose_unique_field_name(candidates, &mut used_fields);
            let relation_name = needs_explicit_relation_name(
                table_name,
                &fk.referenced_table,
                relation_pair_counts,
            )
            .then(|| format!("{}_{}", table_naming[table_name].model_name, field_name));

            relations.push(ForwardRelation {
                fk_index,
                field_name,
                relation_name,
            });
        }

        result.insert(table_name.clone(), relations);
    }

    result
}

pub(super) fn build_back_relations(
    live: &LiveSchema,
    table_names: &[&TableName],
    table_naming: &HashMap<TableName, TableNamingContext>,
    forward_relations: &HashMap<TableName, Vec<ForwardRelation>>,
    directional_relation_counts: &HashMap<(TableName, TableName), usize>,
    options: PullNamingOptions,
) -> HashMap<TableName, Vec<BackRelation>> {
    type IncomingEntry = (TableName, String, Option<String>, bool);
    let mut incoming: HashMap<TableName, Vec<IncomingEntry>> = HashMap::new();

    for &table_name in table_names {
        let table = &live.tables[table_name];
        for relation in forward_relations
            .get(table_name)
            .into_iter()
            .flat_map(|relations| relations.iter())
        {
            let fk = &table.foreign_keys[relation.fk_index];
            incoming
                .entry(fk.referenced_table.clone())
                .or_default()
                .push((
                    table_name.clone(),
                    relation.field_name.clone(),
                    relation.relation_name.clone(),
                    is_one_to_one_back_relation(live, table_name, fk),
                ));
        }
    }

    let mut result = HashMap::new();

    for &table_name in table_names {
        let mut used_fields: HashSet<String> = table_naming[table_name]
            .logical_field_order
            .iter()
            .cloned()
            .collect();
        if let Some(relations) = forward_relations.get(table_name) {
            used_fields.extend(relations.iter().map(|relation| relation.field_name.clone()));
        }

        let mut back_refs = Vec::new();
        if let Some(entries) = incoming.remove(table_name) {
            for (owning_table, forward_field_name, relation_name, is_one_to_one) in entries {
                let is_self_relation = owning_table == *table_name;
                let default_name =
                    default_back_relation_field_name(&owning_table.name, is_one_to_one, options);
                let qualified_name =
                    qualify_back_relation_field_name(&default_name, &forward_field_name, options);
                let direction_count = directional_relation_counts
                    .get(&(owning_table.clone(), table_name.clone()))
                    .copied()
                    .unwrap_or(0);

                let mut candidates = Vec::new();
                if direction_count <= 1 {
                    candidates.push(default_name.clone());
                }
                if qualified_name != default_name {
                    candidates.push(qualified_name);
                }
                candidates.push(default_name);

                let field_name = choose_unique_field_name(candidates, &mut used_fields);
                back_refs.push(BackRelation {
                    owning_table,
                    field_name,
                    relation_name: if is_self_relation {
                        None
                    } else {
                        relation_name
                    },
                    is_one_to_one,
                });
            }
        }

        result.insert(table_name.clone(), back_refs);
    }

    result
}

/// Build the array relation field each side of a recovered many-to-many needs.
///
/// Field names are chosen against the names the model already carries, so a
/// recovered relation never collides with a column or another relation.
pub(super) fn build_many_to_many_ends(
    joins: &[JoinTable],
    table_naming: &HashMap<TableName, TableNamingContext>,
    forward_relations: &HashMap<TableName, Vec<ForwardRelation>>,
    back_relations: &HashMap<TableName, Vec<BackRelation>>,
    options: PullNamingOptions,
) -> HashMap<TableName, Vec<ManyToManyEnd>> {
    let mut used_fields: HashMap<&TableName, HashSet<String>> = HashMap::new();
    let mut result: HashMap<TableName, Vec<ManyToManyEnd>> = HashMap::new();

    for (table_name, naming) in table_naming {
        let mut used: HashSet<String> = naming.logical_field_order.iter().cloned().collect();
        used.extend(
            slice_for(forward_relations, table_name)
                .iter()
                .map(|relation| relation.field_name.clone()),
        );
        used.extend(
            slice_for(back_relations, table_name)
                .iter()
                .map(|relation| relation.field_name.clone()),
        );
        used_fields.insert(table_name, used);
    }

    for join in joins {
        let Some(a_naming) = table_naming.get(&join.a_table) else {
            continue;
        };
        let Some(b_naming) = table_naming.get(&join.b_table) else {
            continue;
        };

        let default_name = format!("_{}To{}", a_naming.model_name, b_naming.model_name);
        let relation_name =
            (join.name != default_name).then(|| join.name.name.trim_start_matches('_').to_string());

        for (owner, target, target_model) in [
            (&join.a_table, &join.b_table, &b_naming.model_name),
            (&join.b_table, &join.a_table, &a_naming.model_name),
        ] {
            let used = used_fields
                .get_mut(owner)
                .expect("every live table has a naming context");
            let base = apply_derived_field_case(
                &pluralize_name(&to_snake_case_identifier(&singular_name(target_model))),
                options.field_case,
            );
            let field_name = choose_unique_field_name(vec![base], used);
            result
                .entry(owner.clone())
                .or_default()
                .push(ManyToManyEnd {
                    target_table: target.clone(),
                    field_name,
                    relation_name: relation_name.clone(),
                });
        }
    }

    result
}

fn relation_pair_key(left: &TableName, right: &TableName) -> (TableName, TableName) {
    if left <= right {
        (left.clone(), right.clone())
    } else {
        (right.clone(), left.clone())
    }
}

fn needs_explicit_relation_name(
    owning_table: &TableName,
    referenced_table: &TableName,
    relation_pair_counts: &HashMap<(TableName, TableName), usize>,
) -> bool {
    owning_table == referenced_table
        || relation_pair_counts
            .get(&relation_pair_key(owning_table, referenced_table))
            .copied()
            .unwrap_or(0)
            > 1
}

fn is_one_to_one_back_relation(
    live: &LiveSchema,
    owning_table: &TableName,
    fk: &LiveForeignKey,
) -> bool {
    live.tables
        .get(owning_table)
        .is_some_and(|table| columns_form_unique_key(table, &fk.columns))
}

fn columns_form_unique_key(table: &LiveTable, columns: &[String]) -> bool {
    let mut normalized_columns = columns.to_vec();
    normalized_columns.sort();

    let mut primary_key = table.primary_key.clone();
    primary_key.sort();
    if normalized_columns == primary_key {
        return true;
    }

    table.indexes.iter().any(|idx| {
        if !idx.unique {
            return false;
        }
        let mut index_columns = idx.columns.clone();
        index_columns.sort();
        index_columns == normalized_columns
    })
}
