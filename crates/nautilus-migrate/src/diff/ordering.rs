//! Execution order for a computed set of changes.
//!
//! Every flow that applies a diff goes through here: [`order_changes_for_apply`]
//! sequences a full change list, and [`topo_sort_models`] orders table creation
//! so a referenced table exists before the table pointing at it.

use nautilus_schema::ir::{ModelIr, ResolvedFieldType};

use crate::change::Change;
use crate::live::LiveSchema;
use nautilus_core::TableName;

/// Reorder computed changes into a safer execution plan.
///
/// The plan prefers dropping foreign keys before destructive column/table
/// changes, drops tables in reverse live dependency order, and defers foreign
/// key creation until after structural changes complete.
pub fn order_changes_for_apply(changes: &[Change], live: &LiveSchema) -> Vec<Change> {
    use std::collections::HashMap;

    let mut pre_type_changes = Vec::new();
    let mut new_tables = Vec::new();
    let mut added_columns = Vec::new();
    let mut foreign_key_drops = Vec::new();
    let mut main_changes = Vec::new();
    let mut dropped_table_names = Vec::new();
    let mut dropped_tables: HashMap<TableName, Change> = HashMap::new();
    let mut index_adds = Vec::new();
    let mut foreign_key_adds = Vec::new();
    let mut post_type_changes = Vec::new();

    for change in changes {
        match change {
            Change::CreateCompositeType { .. }
            | Change::CreateEnum { .. }
            | Change::CreateSchema { .. }
            | Change::CreateExtension { .. } => {
                pre_type_changes.push(change.clone());
            }
            Change::AlterCompositeType {
                dropped_fields,
                type_changed_fields,
                ..
            } if dropped_fields.is_empty() && type_changed_fields.is_empty() => {
                pre_type_changes.push(change.clone());
            }
            Change::AlterEnum {
                removed_variants, ..
            } if removed_variants.is_empty() => {
                pre_type_changes.push(change.clone());
            }
            Change::NewTable(_) => new_tables.push(change.clone()),
            Change::AddedColumn { .. } => added_columns.push(change.clone()),
            Change::ForeignKeyDropped { .. } => foreign_key_drops.push(change.clone()),
            Change::DroppedTable { name } => {
                dropped_table_names.push(name.clone());
                dropped_tables.insert(name.clone(), change.clone());
            }
            Change::IndexAdded { .. } => index_adds.push(change.clone()),
            Change::ForeignKeyAdded { .. } => foreign_key_adds.push(change.clone()),
            Change::DropCompositeType { .. }
            | Change::DropEnum { .. }
            | Change::DropExtension { .. } => {
                post_type_changes.push(change.clone());
            }
            Change::AlterCompositeType { .. } | Change::AlterEnum { .. } => {
                post_type_changes.push(change.clone());
            }
            _ => main_changes.push(change.clone()),
        }
    }

    let mut ordered = Vec::with_capacity(changes.len());
    ordered.extend(pre_type_changes);
    ordered.extend(new_tables);
    ordered.extend(added_columns);
    ordered.extend(foreign_key_drops);
    ordered.extend(main_changes);

    for name in order_dropped_live_tables(live, &dropped_table_names) {
        if let Some(change) = dropped_tables.remove(&name) {
            ordered.push(change);
        }
    }
    for name in &dropped_table_names {
        if let Some(change) = dropped_tables.remove(name) {
            ordered.push(change);
        }
    }

    ordered.extend(index_adds);
    ordered.extend(foreign_key_adds);
    ordered.extend(post_type_changes);
    ordered
}

fn order_dropped_live_tables(live: &LiveSchema, dropped_tables: &[TableName]) -> Vec<TableName> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let dropped_set: HashSet<&TableName> = dropped_tables.iter().collect();
    let mut names: Vec<&TableName> = dropped_set.iter().copied().collect();
    names.sort_unstable();

    let name_to_idx: HashMap<&TableName, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, i))
        .collect();
    let mut in_degree = vec![0usize; names.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); names.len()];

    for &table_name in &names {
        let Some(table) = live.tables.get(table_name) else {
            continue;
        };
        let table_idx = name_to_idx[table_name];
        let mut seen_refs: HashSet<&TableName> = HashSet::new();

        for fk in &table.foreign_keys {
            let referenced = &fk.referenced_table;
            if referenced == table_name
                || !dropped_set.contains(referenced)
                || !seen_refs.insert(referenced)
            {
                continue;
            }
            let referenced_idx = name_to_idx[referenced];
            dependents[referenced_idx].push(table_idx);
            in_degree[table_idx] += 1;
        }
    }

    let mut queue: VecDeque<usize> = (0..names.len()).filter(|&i| in_degree[i] == 0).collect();
    let mut create_order = Vec::with_capacity(names.len());

    while let Some(idx) = queue.pop_front() {
        create_order.push(names[idx]);
        let mut ready = Vec::new();
        for &dependent_idx in &dependents[idx] {
            in_degree[dependent_idx] -= 1;
            if in_degree[dependent_idx] == 0 {
                ready.push(dependent_idx);
            }
        }
        ready.sort_unstable_by_key(|idx| names[*idx]);
        queue.extend(ready);
    }

    let emitted: HashSet<&TableName> = create_order.iter().copied().collect();
    let mut remaining: Vec<&TableName> = names
        .into_iter()
        .filter(|name| !emitted.contains(name))
        .collect();
    remaining.sort_unstable();
    create_order.extend(remaining);

    create_order.reverse();
    create_order.into_iter().cloned().collect()
}

/// Sort models so that a table is always created *before* any table that holds
/// a foreign-key pointing to it.
pub(crate) fn topo_sort_models<'a>(models: &[&'a ModelIr]) -> Vec<&'a ModelIr> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let name_to_idx: HashMap<&str, usize> = models
        .iter()
        .enumerate()
        .map(|(i, m)| (m.logical_name.as_str(), i))
        .collect();

    let n = models.len();
    let mut in_degree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n]; // dependents[dep] = [models that need dep]

    for (i, model) in models.iter().enumerate() {
        for field in &model.fields {
            if let ResolvedFieldType::Relation(rel) = &field.field_type {
                // Only the FK-owning side carries actual column dependencies
                if rel.fields.is_empty() {
                    continue;
                }
                // Skip self-references (can't reorder away from a cycle)
                if rel.target_model == model.logical_name {
                    continue;
                }
                if let Some(&dep_idx) = name_to_idx.get(rel.target_model.as_str()) {
                    if dep_idx != i {
                        dependents[dep_idx].push(i);
                        in_degree[i] += 1;
                    }
                }
            }
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut result: Vec<&ModelIr> = Vec::with_capacity(n);

    while let Some(idx) = queue.pop_front() {
        result.push(models[idx]);
        for &dep in &dependents[idx] {
            in_degree[dep] -= 1;
            if in_degree[dep] == 0 {
                queue.push_back(dep);
            }
        }
    }

    // Append any remaining (cyclic) models — FK cycles shouldn't exist in a
    // valid schema but we handle them gracefully rather than panicking.
    let emitted: HashSet<*const ModelIr> = result.iter().map(|m| *m as *const _).collect();
    for model in models {
        if !emitted.contains(&(*model as *const _)) {
            result.push(model);
        }
    }

    result
}
