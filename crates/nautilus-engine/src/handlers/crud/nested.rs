//! Nested writes: relation-shaped entries inside the `data` of `query.create`
//! and `query.update`.
//!
//! A relation field named in `data` carries an object of operations rather than
//! a column value — `{ "posts": { "create": [...] } }`. Which operations are
//! legal depends on where the foreign key lives:
//!
//! - **Owning side** — the written model holds the foreign key. The related row
//!   has to exist before the parent statement runs, so these operations resolve
//!   first and contribute the foreign-key columns to the parent's own data.
//! - **Inverse side** — the related model holds the foreign key. The parent row
//!   has to exist before its children can point at it, so these operations run
//!   after the parent statement, scoped to the key it produced.
//! - **Many-to-many** — neither model holds one. The links live in a join table
//!   Nautilus owns, so these operations also run after the parent statement and
//!   add or remove rows there instead of writing a foreign key anywhere.
//!
//! Every operation reaches the database through the handler a top-level request
//! would use, on the caller's transaction, so value conversion, defaults,
//! `RETURNING` handling and one more level of nesting are shared with the flat
//! paths instead of reimplemented here.
use nautilus_connector::Row;
use nautilus_core::Value;
use nautilus_protocol::{
    CreateParams, DeleteParams, ProtocolError, UpdateParams, PROTOCOL_VERSION,
};
use nautilus_schema::ir::{FieldIr, ManyToManyJoinIr, ModelIr, RelationIr, ResolvedFieldType};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::mutations::{execute_create, execute_delete, execute_update};
use crate::state::EngineState;

/// Which model carries the foreign-key columns of a relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationSide {
    /// The written model holds them; the related row must exist first.
    Owning,
    /// The related model holds them; the parent row must exist first.
    Inverse,
    /// Neither does: the links live in a join table. See [`ManyToManyJoinIr`].
    ManyToMany,
}

/// The two field lists a nested write needs: the columns that hold the foreign
/// key and the columns it points at, both as logical names.
#[derive(Debug, Clone)]
struct RelationBinding {
    target_model: String,
    side: RelationSide,
    /// Logical field names on the model that carries the foreign key. Empty on
    /// a many-to-many, where no model carries one.
    foreign_keys: Vec<String>,
    /// Logical field names on the model the foreign key points at. On a
    /// many-to-many this is the written model's own key, which the join table
    /// stores.
    referenced: Vec<String>,
    /// The join table, on a many-to-many.
    via: Option<ManyToManyJoinIr>,
}

/// One relation field of a `data` payload together with its parsed operations.
pub(super) struct NestedWrite<'a> {
    field_name: &'a str,
    binding: RelationBinding,
    operations: Vec<(&'static str, &'a JsonValue)>,
}

/// A `data` payload split into the columns of the written model and the nested
/// writes that run around the statement for it.
pub(super) struct NestedPlan<'a> {
    scalar_data: JsonMap<String, JsonValue>,
    owning: Vec<NestedWrite<'a>>,
    inverse: Vec<NestedWrite<'a>>,
}

impl NestedPlan<'_> {
    /// Whether the payload was plain column data, with no relation entries.
    pub(super) fn is_empty(&self) -> bool {
        self.owning.is_empty() && self.inverse.is_empty()
    }

    /// Whether any nested write needs the written row's key.
    pub(super) fn writes_children(&self) -> bool {
        !self.inverse.is_empty()
    }

    /// The column entries plus the foreign keys the owning-side writes resolved.
    fn scalar_data_with(&self, assignments: JsonMap<String, JsonValue>) -> JsonValue {
        let mut data = self.scalar_data.clone();
        for (key, value) in assignments {
            data.insert(key, value);
        }
        JsonValue::Object(data)
    }
}

const CREATE_OPERATIONS: &[&str] = &["create", "createMany", "connect", "connectOrCreate"];
const UPDATE_ONLY_OPERATIONS: &[&str] = &[
    "disconnect",
    "set",
    "update",
    "updateMany",
    "delete",
    "deleteMany",
];

/// Accept an operation name in either the wire spelling or the snake_case one a
/// Python or Rust caller would write, and answer with the wire spelling.
fn canonical_operation(name: &str) -> Option<&'static str> {
    let flattened: String = name
        .chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    CREATE_OPERATIONS
        .iter()
        .chain(UPDATE_ONLY_OPERATIONS)
        .find(|candidate| {
            candidate
                .chars()
                .flat_map(char::to_lowercase)
                .eq(flattened.chars())
        })
        .copied()
}

fn supported_operations(allow_update_operations: bool) -> String {
    let names: Vec<&str> = if allow_update_operations {
        CREATE_OPERATIONS
            .iter()
            .chain(UPDATE_ONLY_OPERATIONS)
            .copied()
            .collect()
    } else {
        CREATE_OPERATIONS.to_vec()
    };
    names.join(", ")
}

/// Order the operations of one relation so that removals run before additions.
///
/// `set` replaces the members of a relation wholesale, so it has to clear the
/// old ones before `connect` or `create` adds the new ones.
fn operation_order(name: &str) -> u8 {
    match name {
        "set" => 0,
        "disconnect" => 1,
        "delete" | "deleteMany" => 2,
        "update" | "updateMany" => 3,
        _ => 4,
    }
}

fn relation_field<'a>(model: &'a ModelIr, key: &str) -> Option<(&'a FieldIr, &'a RelationIr)> {
    model.fields.iter().find_map(|field| {
        let ResolvedFieldType::Relation(relation) = &field.field_type else {
            return None;
        };
        (field.logical_name == key
            || field.db_name == key
            || crate::conversion::to_snake_case(&field.logical_name) == key)
            .then_some((field, relation))
    })
}

/// Find the relation on `target` that is the other end of `relation`.
///
/// The inverse is the side that names foreign-key fields; a relation name
/// disambiguates when two relations connect the same pair of models.
fn inverse_relation<'a>(
    model: &ModelIr,
    field: &FieldIr,
    relation: &RelationIr,
    target: &'a ModelIr,
) -> Result<&'a RelationIr, ProtocolError> {
    let mut matches = target.relation_fields().filter_map(|candidate| {
        let ResolvedFieldType::Relation(inverse) = &candidate.field_type else {
            return None;
        };
        if inverse.target_model != model.logical_name || inverse.fields.is_empty() {
            return None;
        }
        match relation.name.as_deref() {
            Some(name) if inverse.name.as_deref() != Some(name) => None,
            _ => Some(inverse),
        }
    });

    let first = matches.next().ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Nested write on '{}.{}' needs the foreign-key side of the relation on model '{}', and none was found",
            model.logical_name, field.logical_name, target.logical_name
        ))
    })?;

    if matches.next().is_some() {
        return Err(ProtocolError::QueryPlanning(format!(
            "Nested write on '{}.{}' is ambiguous: model '{}' declares more than one relation back to '{}'",
            model.logical_name, field.logical_name, target.logical_name, model.logical_name
        )));
    }

    Ok(first)
}

fn binding_for(
    state: &EngineState,
    model: &ModelIr,
    field: &FieldIr,
    relation: &RelationIr,
) -> Result<RelationBinding, ProtocolError> {
    let target = state.models().get(&relation.target_model).ok_or_else(|| {
        ProtocolError::InvalidModel(format!(
            "Relation '{}.{}' targets unknown model '{}'",
            model.logical_name, field.logical_name, relation.target_model
        ))
    })?;

    if let Some(join) = &relation.join {
        return Ok(RelationBinding {
            target_model: target.logical_name.clone(),
            side: RelationSide::ManyToMany,
            foreign_keys: Vec::new(),
            referenced: vec![join.self_reference.clone()],
            via: Some(join.clone()),
        });
    }

    if relation.fields.is_empty() {
        let inverse = inverse_relation(model, field, relation, target)?;
        Ok(RelationBinding {
            target_model: target.logical_name.clone(),
            side: RelationSide::Inverse,
            foreign_keys: inverse.fields.clone(),
            referenced: inverse.references.clone(),
            via: None,
        })
    } else {
        Ok(RelationBinding {
            target_model: target.logical_name.clone(),
            side: RelationSide::Owning,
            foreign_keys: relation.fields.clone(),
            referenced: relation.references.clone(),
            via: None,
        })
    }
}

/// Split a `data` payload into its column entries and its nested writes.
///
/// `allow_update_operations` admits the operations that only make sense against
/// a row that already exists; a `create` rejects them.
pub(super) fn split<'a>(
    state: &EngineState,
    model: &'a ModelIr,
    data: &'a JsonValue,
    allow_update_operations: bool,
) -> Result<NestedPlan<'a>, ProtocolError> {
    let data_obj = data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;

    let mut plan = NestedPlan {
        scalar_data: JsonMap::new(),
        owning: Vec::new(),
        inverse: Vec::new(),
    };

    for (key, value) in data_obj {
        let Some((field, relation)) = relation_field(model, key) else {
            plan.scalar_data.insert(key.clone(), value.clone());
            continue;
        };

        let operations = parse_operations(model, field, value, allow_update_operations)?;
        if operations.is_empty() {
            continue;
        }

        let write = NestedWrite {
            field_name: field.logical_name.as_str(),
            binding: binding_for(state, model, field, relation)?,
            operations,
        };

        match write.binding.side {
            RelationSide::Owning => plan.owning.push(write),
            RelationSide::Inverse | RelationSide::ManyToMany => plan.inverse.push(write),
        }
    }

    Ok(plan)
}

fn parse_operations<'a>(
    model: &ModelIr,
    field: &FieldIr,
    value: &'a JsonValue,
    allow_update_operations: bool,
) -> Result<Vec<(&'static str, &'a JsonValue)>, ProtocolError> {
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::InvalidParams(format!(
            "'{}.{}' is a relation, so its data entry must be an object of nested-write operations",
            model.logical_name, field.logical_name
        ))
    })?;

    let mut operations = Vec::with_capacity(object.len());
    for (name, payload) in object {
        let canonical = canonical_operation(name).ok_or_else(|| {
            ProtocolError::InvalidParams(format!(
                "Unknown nested-write operation '{}' on '{}.{}'; supported operations are {}",
                name,
                model.logical_name,
                field.logical_name,
                supported_operations(allow_update_operations)
            ))
        })?;

        if !allow_update_operations && UPDATE_ONLY_OPERATIONS.contains(&canonical) {
            return Err(ProtocolError::InvalidParams(format!(
                "Nested '{}' on '{}.{}' needs an existing row, so it is only available on update",
                canonical, model.logical_name, field.logical_name
            )));
        }

        operations.push((canonical, payload));
    }

    operations.sort_by_key(|(name, _)| operation_order(name));
    Ok(operations)
}

/// Read the columns named by `fields` out of a row of `model`.
fn row_key_values(
    state: &EngineState,
    model: &ModelIr,
    row: &Row,
    fields: &[String],
) -> Result<Vec<Value>, ProtocolError> {
    fields
        .iter()
        .map(|logical_name| {
            let db_name = state
                .model_metadata(model)
                .logical_to_db()
                .get(logical_name)
                .ok_or_else(|| {
                    ProtocolError::QueryPlanning(format!(
                        "Relation on model '{}' references unknown field '{}'",
                        model.logical_name, logical_name
                    ))
                })?;
            row.get(&format!("{}__{}", model.db_name, db_name))
                .cloned()
                .ok_or_else(|| {
                    ProtocolError::Internal(format!(
                        "Nested write could not read '{}.{}' back from the written row",
                        model.logical_name, logical_name
                    ))
                })
        })
        .collect()
}

fn key_filter(fields: &[String], values: &[Value]) -> JsonValue {
    let mut filter = JsonMap::with_capacity(fields.len());
    for (name, value) in fields.iter().zip(values) {
        filter.insert(name.clone(), value.to_json_plain());
    }
    JsonValue::Object(filter)
}

fn null_key_data(fields: &[String]) -> JsonValue {
    let mut data = JsonMap::with_capacity(fields.len());
    for name in fields {
        data.insert(name.clone(), JsonValue::Null);
    }
    JsonValue::Object(data)
}

fn scoped_filter(scope: JsonValue, extra: Option<&JsonValue>) -> JsonValue {
    match extra {
        Some(extra) if extra.as_object().is_some_and(|obj| !obj.is_empty()) => {
            serde_json::json!({ "AND": [scope, unwrap_where(extra)] })
        }
        _ => scope,
    }
}

/// Accept a filter written either bare or wrapped in `{ "where": ... }`.
fn unwrap_where(value: &JsonValue) -> JsonValue {
    value
        .as_object()
        .filter(|obj| obj.len() == 1)
        .and_then(|obj| obj.get("where"))
        .cloned()
        .unwrap_or_else(|| value.clone())
}

fn payload_items(payload: &JsonValue) -> Vec<&JsonValue> {
    match payload {
        JsonValue::Array(items) => items.iter().collect(),
        other => vec![other],
    }
}

fn require_object<'a>(
    value: &'a JsonValue,
    context: &str,
) -> Result<&'a JsonMap<String, JsonValue>, ProtocolError> {
    value
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams(format!("{context} must be an object")))
}

fn require_member<'a>(
    object: &'a JsonMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<&'a JsonValue, ProtocolError> {
    object
        .get(key)
        .ok_or_else(|| ProtocolError::InvalidParams(format!("{context} needs a '{key}' entry")))
}

async fn create_related(
    state: &EngineState,
    target_model: &str,
    data: JsonValue,
    tx: &str,
) -> Result<Vec<Row>, ProtocolError> {
    let params = CreateParams {
        protocol_version: PROTOCOL_VERSION,
        model: target_model.to_string(),
        data,
        transaction_id: Some(tx.to_string()),
        return_data: true,
    };
    Box::pin(execute_create(state, params))
        .await?
        .into_rows("nested create")
}

async fn update_related(
    state: &EngineState,
    target_model: &str,
    filter: JsonValue,
    data: JsonValue,
    tx: &str,
) -> Result<usize, ProtocolError> {
    let params = UpdateParams {
        protocol_version: PROTOCOL_VERSION,
        model: target_model.to_string(),
        filter,
        data,
        transaction_id: Some(tx.to_string()),
        return_data: false,
    };
    Ok(Box::pin(execute_update(state, params)).await?.into_count())
}

async fn delete_related(
    state: &EngineState,
    target_model: &str,
    filter: JsonValue,
    tx: &str,
) -> Result<usize, ProtocolError> {
    let params = DeleteParams {
        protocol_version: PROTOCOL_VERSION,
        model: target_model.to_string(),
        filter,
        transaction_id: Some(tx.to_string()),
        return_data: false,
    };
    Ok(Box::pin(execute_delete(state, params)).await?.into_count())
}

async fn find_related(
    state: &EngineState,
    target_model: &str,
    filter: &JsonValue,
    tx: &str,
) -> Result<Option<Row>, ProtocolError> {
    let target = state
        .models()
        .get(target_model)
        .ok_or_else(|| ProtocolError::InvalidModel(target_model.to_string()))?;
    super::read::find_one_row(state, target, &unwrap_where(filter), Some(tx)).await
}

/// Resolve the operations whose related row has to exist before the parent
/// statement runs, and answer with the foreign-key columns they set on it.
///
/// `current` is the row being updated, and is `None` on a create; the
/// operations that act on an already-connected row need it to find that row.
async fn resolve_owning_writes(
    state: &EngineState,
    model: &ModelIr,
    writes: &[NestedWrite<'_>],
    current: Option<&Row>,
    tx: &str,
) -> Result<(JsonMap<String, JsonValue>, Vec<DeferredDelete>), ProtocolError> {
    let mut assignments = JsonMap::new();
    let mut deferred = Vec::new();

    for write in writes {
        let binding = &write.binding;
        for (operation, payload) in &write.operations {
            match *operation {
                "create" => {
                    let rows =
                        create_related(state, &binding.target_model, (*payload).clone(), tx).await?;
                    let row = first_row(&rows, write.field_name)?;
                    assign_foreign_key(state, binding, row, &mut assignments)?;
                }
                "connect" => {
                    let row = find_related(state, &binding.target_model, payload, tx)
                        .await?
                        .ok_or_else(|| not_found("connect", write.field_name, binding))?;
                    assign_foreign_key(state, binding, &row, &mut assignments)?;
                }
                "connectOrCreate" => {
                    let object = require_object(payload, "connectOrCreate")?;
                    let filter = require_member(object, "where", "connectOrCreate")?;
                    match find_related(state, &binding.target_model, filter, tx).await? {
                        Some(row) => assign_foreign_key(state, binding, &row, &mut assignments)?,
                        None => {
                            let data = require_member(object, "create", "connectOrCreate")?;
                            let rows =
                                create_related(state, &binding.target_model, data.clone(), tx)
                                    .await?;
                            let row = first_row(&rows, write.field_name)?;
                            assign_foreign_key(state, binding, row, &mut assignments)?;
                        }
                    }
                }
                "disconnect" => clear_foreign_key(binding, &mut assignments),
                "update" => {
                    let row = require_current(current, write.field_name, operation)?;
                    let filter = connected_filter(state, model, binding, row)?;
                    update_related(state, &binding.target_model, filter, (*payload).clone(), tx)
                        .await?;
                }
                "delete" => {
                    let row = require_current(current, write.field_name, operation)?;
                    let filter = connected_filter(state, model, binding, row)?;
                    clear_foreign_key(binding, &mut assignments);
                    deferred.push(DeferredDelete {
                        model: binding.target_model.clone(),
                        filter,
                    });
                }
                other => {
                    return Err(ProtocolError::UnsupportedOperation(format!(
                        "Nested '{}' is not available on '{}.{}', which is the side holding the foreign key",
                        other, model.logical_name, write.field_name
                    )))
                }
            }
        }
    }

    Ok((assignments, deferred))
}

/// A delete held back until the parent no longer references the row.
struct DeferredDelete {
    model: String,
    filter: JsonValue,
}

fn not_found(operation: &str, field_name: &str, binding: &RelationBinding) -> ProtocolError {
    ProtocolError::RecordNotFound(format!(
        "Nested {} on '{}' matched no '{}' record",
        operation, field_name, binding.target_model
    ))
}

fn require_current<'a>(
    current: Option<&'a Row>,
    field_name: &str,
    operation: &str,
) -> Result<&'a Row, ProtocolError> {
    current.ok_or_else(|| {
        ProtocolError::InvalidParams(format!(
            "Nested '{}' on '{}' needs an existing row, so it is only available on update",
            operation, field_name
        ))
    })
}

/// Filter matching the row the parent's foreign key currently points at.
fn connected_filter(
    state: &EngineState,
    model: &ModelIr,
    binding: &RelationBinding,
    current: &Row,
) -> Result<JsonValue, ProtocolError> {
    let values = row_key_values(state, model, current, &binding.foreign_keys)?;
    Ok(key_filter(&binding.referenced, &values))
}

fn first_row<'a>(rows: &'a [Row], field_name: &str) -> Result<&'a Row, ProtocolError> {
    rows.first().ok_or_else(|| {
        ProtocolError::Internal(format!(
            "Nested create on '{}' returned no row to link the parent to",
            field_name
        ))
    })
}

fn assign_foreign_key(
    state: &EngineState,
    binding: &RelationBinding,
    row: &Row,
    assignments: &mut JsonMap<String, JsonValue>,
) -> Result<(), ProtocolError> {
    let target = state
        .models()
        .get(&binding.target_model)
        .ok_or_else(|| ProtocolError::InvalidModel(binding.target_model.clone()))?;
    let values = row_key_values(state, target, row, &binding.referenced)?;
    for (name, value) in binding.foreign_keys.iter().zip(values) {
        assignments.insert(name.clone(), value.to_json_plain());
    }
    Ok(())
}

fn clear_foreign_key(binding: &RelationBinding, assignments: &mut JsonMap<String, JsonValue>) {
    for name in &binding.foreign_keys {
        assignments.insert(name.clone(), JsonValue::Null);
    }
}

/// Run the operations whose rows point at the parent, now that it exists.
///
/// Every one of them is scoped to the parent's key, so a filter supplied by the
/// caller can only narrow the rows reached through the relation, never widen
/// them to rows belonging to another parent.
async fn apply_inverse_writes(
    state: &EngineState,
    model: &ModelIr,
    writes: &[NestedWrite<'_>],
    parent: &Row,
    tx: &str,
) -> Result<(), ProtocolError> {
    for write in writes {
        let binding = &write.binding;
        if binding.side == RelationSide::ManyToMany {
            apply_many_to_many_writes(state, model, write, parent, tx).await?;
            continue;
        }

        let parent_key = row_key_values(state, model, parent, &binding.referenced)?;
        let link = key_filter(&binding.foreign_keys, &parent_key);

        for (operation, payload) in &write.operations {
            match *operation {
                "create" => {
                    for item in payload_items(payload) {
                        let data = merge_link(item, &link, write.field_name)?;
                        create_related(state, &binding.target_model, data, tx).await?;
                    }
                }
                "createMany" => {
                    let object = require_object(payload, "createMany")?;
                    let rows = require_member(object, "data", "createMany")?;
                    for item in payload_items(rows) {
                        let data = merge_link(item, &link, write.field_name)?;
                        create_related(state, &binding.target_model, data, tx).await?;
                    }
                }
                "connect" => {
                    for item in payload_items(payload) {
                        connect_child(state, binding, write.field_name, item, &link, tx).await?;
                    }
                }
                "connectOrCreate" => {
                    for item in payload_items(payload) {
                        let object = require_object(item, "connectOrCreate")?;
                        let filter = require_member(object, "where", "connectOrCreate")?;
                        if find_related(state, &binding.target_model, filter, tx)
                            .await?
                            .is_some()
                        {
                            connect_child(state, binding, write.field_name, filter, &link, tx)
                                .await?;
                        } else {
                            let create = require_member(object, "create", "connectOrCreate")?;
                            let data = merge_link(create, &link, write.field_name)?;
                            create_related(state, &binding.target_model, data, tx).await?;
                        }
                    }
                }
                "disconnect" => {
                    let nulls = null_key_data(&binding.foreign_keys);
                    for filter in child_filters(payload, &link) {
                        update_related(state, &binding.target_model, filter, nulls.clone(), tx)
                            .await?;
                    }
                }
                "set" => {
                    let nulls = null_key_data(&binding.foreign_keys);
                    update_related(state, &binding.target_model, link.clone(), nulls, tx).await?;
                    for item in payload_items(payload) {
                        connect_child(state, binding, write.field_name, item, &link, tx).await?;
                    }
                }
                "update" | "updateMany" => {
                    for item in payload_items(payload) {
                        let object = require_object(item, operation)?;
                        let data = require_member(object, "data", operation)?;
                        let filter = scoped_filter(link.clone(), object.get("where"));
                        let affected =
                            update_related(state, &binding.target_model, filter, data.clone(), tx)
                                .await?;
                        if *operation == "update" && affected == 0 {
                            return Err(not_found("update", write.field_name, binding));
                        }
                    }
                }
                "delete" | "deleteMany" => {
                    for filter in child_filters(payload, &link) {
                        let affected =
                            delete_related(state, &binding.target_model, filter, tx).await?;
                        if *operation == "delete" && affected == 0 {
                            return Err(not_found("delete", write.field_name, binding));
                        }
                    }
                }
                other => {
                    return Err(ProtocolError::UnsupportedOperation(format!(
                        "Nested '{}' is not available on '{}.{}'",
                        other, model.logical_name, write.field_name
                    )))
                }
            }
        }
    }

    Ok(())
}

/// Run the operations of one many-to-many relation.
///
/// The parent row exists by the time this runs, so every operation is at most
/// two writes: the child row itself, and the link in the join table. The
/// operations that reach existing children resolve the relation's current
/// members first and narrow to them, which is what keeps a caller-supplied
/// `where` from reaching a row linked to a different parent.
async fn apply_many_to_many_writes(
    state: &EngineState,
    model: &ModelIr,
    write: &NestedWrite<'_>,
    parent: &Row,
    tx: &str,
) -> Result<(), ProtocolError> {
    let binding = &write.binding;
    let join = binding
        .via
        .as_ref()
        .expect("a many-to-many binding always carries its join table");
    let parent_key = row_key_values(state, model, parent, &binding.referenced)?
        .into_iter()
        .next()
        .expect("a many-to-many binding references exactly one key field");

    for (operation, payload) in &write.operations {
        match *operation {
            "create" => {
                for item in payload_items(payload) {
                    let child =
                        create_one(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "createMany" => {
                let object = require_object(payload, "createMany")?;
                let items = require_member(object, "data", "createMany")?;
                for item in payload_items(items) {
                    let child =
                        create_one(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "connect" => {
                for item in payload_items(payload) {
                    let child =
                        connected_key(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "connectOrCreate" => {
                for item in payload_items(payload) {
                    let object = require_object(item, "connectOrCreate")?;
                    let filter = require_member(object, "where", "connectOrCreate")?;
                    let child = match find_related(state, &binding.target_model, filter, tx).await?
                    {
                        Some(child) => child_key_value(state, binding, join, &child)?,
                        None => {
                            let data = require_member(object, "create", "connectOrCreate")?;
                            create_one(state, binding, join, write.field_name, data, tx).await?
                        }
                    };
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "disconnect" => {
                let scope = member_filter(join, &linked_keys(state, join, &parent_key, tx).await?);
                for filter in child_filters(payload, &scope) {
                    let matched =
                        matching_children(state, &binding.target_model, &filter, tx).await?;
                    unlink_children(state, binding, join, &parent_key, &matched, tx).await?;
                }
            }
            "set" => {
                unlink_all(state, join, &parent_key, tx).await?;
                for item in payload_items(payload) {
                    let child =
                        connected_key(state, binding, join, write.field_name, item, tx).await?;
                    link_child(state, join, &parent_key, &child, tx).await?;
                }
            }
            "update" | "updateMany" => {
                let scope = member_filter(join, &linked_keys(state, join, &parent_key, tx).await?);
                for item in payload_items(payload) {
                    let object = require_object(item, operation)?;
                    let data = require_member(object, "data", operation)?;
                    let filter = scoped_filter(scope.clone(), object.get("where"));
                    let affected =
                        update_related(state, &binding.target_model, filter, data.clone(), tx)
                            .await?;
                    if *operation == "update" && affected == 0 {
                        return Err(not_found("update", write.field_name, binding));
                    }
                }
            }
            "delete" | "deleteMany" => {
                let scope = member_filter(join, &linked_keys(state, join, &parent_key, tx).await?);
                for filter in child_filters(payload, &scope) {
                    let affected = delete_related(state, &binding.target_model, filter, tx).await?;
                    if *operation == "delete" && affected == 0 {
                        return Err(not_found("delete", write.field_name, binding));
                    }
                }
            }
            other => {
                return Err(ProtocolError::UnsupportedOperation(format!(
                    "Nested '{}' is not available on '{}.{}'",
                    other, model.logical_name, write.field_name
                )))
            }
        }
    }

    Ok(())
}

/// Create one child of the relation and answer with the key the join table
/// stores for it.
async fn create_one(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    field_name: &str,
    data: &JsonValue,
    tx: &str,
) -> Result<Value, ProtocolError> {
    let rows = create_related(state, &binding.target_model, data.clone(), tx).await?;
    let child = first_row(&rows, field_name)?;
    child_key_value(state, binding, join, child)
}

/// Resolve an existing child by the filter a `connect` or `set` names.
async fn connected_key(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    field_name: &str,
    filter: &JsonValue,
    tx: &str,
) -> Result<Value, ProtocolError> {
    let child = find_related(state, &binding.target_model, filter, tx)
        .await?
        .ok_or_else(|| not_found("connect", field_name, binding))?;
    child_key_value(state, binding, join, &child)
}

/// The model Nautilus synthesised for the links of this relation.
fn join_model<'a>(
    state: &'a EngineState,
    join: &ManyToManyJoinIr,
) -> Result<&'a ModelIr, ProtocolError> {
    state.models().get(&join.table).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!("Join table '{}' not found", join.table))
    })
}

/// Read one column out of a row of the join table.
fn join_column(join: &ManyToManyJoinIr, row: &Row, column: &str) -> Option<Value> {
    row.get(&format!("{}__{}", join.table, column)).cloned()
}

/// The keys of every child currently linked to `parent_key`.
async fn linked_keys(
    state: &EngineState,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    tx: &str,
) -> Result<Vec<JsonValue>, ProtocolError> {
    let model = join_model(state, join)?;
    let filter = serde_json::json!({ &join.self_column: parent_key.to_json_plain() });
    let rows = super::read::find_all_rows_by_filter(state, model, &filter, Some(tx)).await?;

    Ok(rows
        .iter()
        .filter_map(|row| join_column(join, row, &join.target_column))
        .map(|value| value.to_json_plain())
        .collect())
}

/// A filter matching exactly the children the relation currently holds.
///
/// An empty member list stays expressible on purpose: narrowing to nothing is
/// the right answer for an operation aimed at an empty relation, and it is what
/// makes a `where` supplied by the caller unable to widen the reach.
fn member_filter(join: &ManyToManyJoinIr, members: &[JsonValue]) -> JsonValue {
    serde_json::json!({
        &join.target_reference: { "in": JsonValue::Array(members.to_vec()) }
    })
}

/// The rows of the target model that `filter` selects.
async fn matching_children(
    state: &EngineState,
    target_model: &str,
    filter: &JsonValue,
    tx: &str,
) -> Result<Vec<Row>, ProtocolError> {
    let target = state
        .models()
        .get(target_model)
        .ok_or_else(|| ProtocolError::InvalidModel(target_model.to_string()))?;
    super::read::find_all_rows_by_filter(state, target, filter, Some(tx)).await
}

/// Link `child` to the parent, unless the two are linked already.
///
/// Connecting twice is not an error the caller can act on — the relation ends
/// up the same either way — so the second link is skipped rather than left to
/// violate the join table's primary key.
async fn link_child(
    state: &EngineState,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    child_key: &Value,
    tx: &str,
) -> Result<(), ProtocolError> {
    let model = join_model(state, join)?;
    let link = serde_json::json!({
        &join.self_column: parent_key.to_json_plain(),
        &join.target_column: child_key.to_json_plain(),
    });

    if super::read::find_one_row(state, model, &link, Some(tx))
        .await?
        .is_some()
    {
        return Ok(());
    }

    create_related(state, &model.logical_name, link, tx).await?;
    Ok(())
}

/// Drop the links between the parent and each of `children`.
async fn unlink_children(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    children: &[Row],
    tx: &str,
) -> Result<(), ProtocolError> {
    if children.is_empty() {
        return Ok(());
    }

    let keys: Vec<JsonValue> = children
        .iter()
        .map(|child| child_key_value(state, binding, join, child).map(|key| key.to_json_plain()))
        .collect::<Result<_, _>>()?;

    let filter = serde_json::json!({
        &join.self_column: parent_key.to_json_plain(),
        &join.target_column: { "in": JsonValue::Array(keys) },
    });
    delete_related(state, &join.table, filter, tx).await?;
    Ok(())
}

/// Drop every link the parent holds through this relation.
async fn unlink_all(
    state: &EngineState,
    join: &ManyToManyJoinIr,
    parent_key: &Value,
    tx: &str,
) -> Result<(), ProtocolError> {
    let filter = serde_json::json!({ &join.self_column: parent_key.to_json_plain() });
    delete_related(state, &join.table, filter, tx).await?;
    Ok(())
}

/// Read the key the join table stores for a child row.
fn child_key_value(
    state: &EngineState,
    binding: &RelationBinding,
    join: &ManyToManyJoinIr,
    child: &Row,
) -> Result<Value, ProtocolError> {
    let target = state
        .models()
        .get(&binding.target_model)
        .ok_or_else(|| ProtocolError::InvalidModel(binding.target_model.clone()))?;
    row_key_values(
        state,
        target,
        child,
        std::slice::from_ref(&join.target_reference),
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        ProtocolError::Internal("Many-to-many link could not read the child's key back".to_string())
    })
}

/// Filters for the operations that address children by an optional `where`:
/// `true` (or no payload at all) means every child of this parent.
fn child_filters(payload: &JsonValue, link: &JsonValue) -> Vec<JsonValue> {
    match payload {
        JsonValue::Bool(true) | JsonValue::Null => vec![link.clone()],
        JsonValue::Array(items) => items
            .iter()
            .map(|item| scoped_filter(link.clone(), Some(item)))
            .collect(),
        other => vec![scoped_filter(link.clone(), Some(other))],
    }
}

async fn connect_child(
    state: &EngineState,
    binding: &RelationBinding,
    field_name: &str,
    filter: &JsonValue,
    link: &JsonValue,
    tx: &str,
) -> Result<(), ProtocolError> {
    let affected = update_related(
        state,
        &binding.target_model,
        unwrap_where(filter),
        link.clone(),
        tx,
    )
    .await?;

    if affected == 0 {
        return Err(not_found("connect", field_name, binding));
    }
    Ok(())
}

fn merge_link(
    item: &JsonValue,
    link: &JsonValue,
    field_name: &str,
) -> Result<JsonValue, ProtocolError> {
    let mut data = item
        .as_object()
        .ok_or_else(|| {
            ProtocolError::InvalidParams(format!(
                "Nested create on '{}' must be an object or an array of objects",
                field_name
            ))
        })?
        .clone();

    if let Some(link) = link.as_object() {
        for (key, value) in link {
            data.insert(key.clone(), value.clone());
        }
    }
    Ok(JsonValue::Object(data))
}

/// Resolve the owning-side writes of `plan` and answer with the payload the
/// parent statement should write, plus the deletes it defers.
pub(super) async fn prepare_parent_data(
    state: &EngineState,
    model: &ModelIr,
    plan: &NestedPlan<'_>,
    current: Option<&Row>,
    tx: &str,
) -> Result<(JsonValue, DeferredDeletes), ProtocolError> {
    let (assignments, deferred) =
        resolve_owning_writes(state, model, &plan.owning, current, tx).await?;
    Ok((
        plan.scalar_data_with(assignments),
        DeferredDeletes(deferred),
    ))
}

/// The deletes a nested write postponed until the parent stopped referencing
/// their rows.
pub(super) struct DeferredDeletes(Vec<DeferredDelete>);

impl DeferredDeletes {
    /// Run the postponed deletes.
    pub(super) async fn run(self, state: &EngineState, tx: &str) -> Result<(), ProtocolError> {
        for delete in self.0 {
            delete_related(state, &delete.model, delete.filter, tx).await?;
        }
        Ok(())
    }
}

/// Run the inverse-side writes of `plan` against the parent row just written.
pub(super) async fn apply_children(
    state: &EngineState,
    model: &ModelIr,
    plan: &NestedPlan<'_>,
    parent: &Row,
    tx: &str,
) -> Result<(), ProtocolError> {
    apply_inverse_writes(state, model, &plan.inverse, parent, tx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_names_are_accepted_in_both_spellings() {
        assert_eq!(
            canonical_operation("connectOrCreate"),
            Some("connectOrCreate")
        );
        assert_eq!(
            canonical_operation("connect_or_create"),
            Some("connectOrCreate")
        );
        assert_eq!(canonical_operation("deleteMany"), Some("deleteMany"));
        assert_eq!(canonical_operation("nope"), None);
    }

    #[test]
    fn removals_are_ordered_before_additions() {
        let mut operations = vec!["create", "set", "connect", "disconnect", "deleteMany"];
        operations.sort_by_key(|name| operation_order(name));
        assert_eq!(
            operations,
            vec!["set", "disconnect", "deleteMany", "create", "connect"]
        );
    }

    #[test]
    fn a_bare_filter_and_a_wrapped_one_unwrap_alike() {
        let bare = serde_json::json!({ "id": 1 });
        let wrapped = serde_json::json!({ "where": { "id": 1 } });
        assert_eq!(unwrap_where(&bare), unwrap_where(&wrapped));
    }

    #[test]
    fn a_child_filter_narrows_to_the_parent_link() {
        let link = serde_json::json!({ "authorId": 7 });
        assert_eq!(
            child_filters(&JsonValue::Bool(true), &link),
            vec![link.clone()]
        );
        assert_eq!(
            child_filters(&serde_json::json!({ "id": 3 }), &link),
            vec![serde_json::json!({ "AND": [{ "authorId": 7 }, { "id": 3 }] })]
        );
    }
}
