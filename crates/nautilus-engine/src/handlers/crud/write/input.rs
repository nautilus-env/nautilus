//! The rules that turn one `data` object into columns, values and assignments.
//!
//! Every write path shares them: which keys a model accepts, how an atomic
//! operator becomes an expression over the column's current value, when an
//! explicit `null` means the database's own default should fill the column,
//! and where `updatedAt` takes its value from. Having one owner for them is
//! what keeps a create and the create half of an upsert writing the same row.

use nautilus_core::{Assignment, BinaryOp, ColumnMarker, Expr, Value};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{DefaultValue, FieldIr, ModelIr, ResolvedFieldType, ScalarType};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::conversion::json_to_value_field;
use crate::handlers::field_marker;
use crate::state::EngineState;

pub(super) fn row_field_json<'a>(
    data_obj: &'a JsonMap<String, JsonValue>,
    field: &FieldIr,
) -> Option<&'a JsonValue> {
    data_obj
        .get(&field.logical_name)
        .or_else(|| data_obj.get(&field.db_name))
}

/// Reject a `data` key that matches no field of the model.
///
/// A mistyped field name is otherwise dropped without a word, and the row is
/// written missing that column.
pub(super) fn ensure_known_data_keys(
    model: &ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<(), ProtocolError> {
    for key in data_obj.keys() {
        if !model
            .fields
            .iter()
            .any(|field| &field.logical_name == key || &field.db_name == key)
        {
            return Err(ProtocolError::InvalidParams(format!(
                "Model '{}' has no field '{}'",
                model.logical_name, key
            )));
        }
    }
    Ok(())
}

/// The atomic update operators a scalar field accepts.
///
/// `set` writes the operand as given and exists to disambiguate a payload that
/// would otherwise read as an operator object. The other four derive the new
/// value from the column's current one, which is why they render as an
/// expression instead of a bound parameter — and why they are meaningless on a
/// create, where the row has no current value yet.
const ATOMIC_OPERATORS: [&str; 5] = ["set", "increment", "decrement", "multiply", "divide"];

/// The arithmetic an atomic operator applies to the column's current value.
fn atomic_operator_sql(operator: &str) -> Option<BinaryOp> {
    match operator {
        "increment" => Some(BinaryOp::Add),
        "decrement" => Some(BinaryOp::Sub),
        "multiply" => Some(BinaryOp::Mul),
        "divide" => Some(BinaryOp::Div),
        _ => None,
    }
}

/// Read one field's input as an atomic update operator, if that is what it is.
///
/// A field whose declared type makes an object or array a legitimate value —
/// `Json`, `Bytes`, a composite, any list — is never interpreted this way: a
/// `{"set": …}` there is the value the caller means to store.
fn atomic_operator<'a>(
    json: &'a JsonValue,
    field: &FieldIr,
) -> Result<Option<(&'a str, &'a JsonValue)>, ProtocolError> {
    if crate::conversion::holds_structured_json(&field.field_type, field.is_array) {
        return Ok(None);
    }
    let Some(object) = json.as_object() else {
        return Ok(None);
    };
    let mut found: Option<(&str, &JsonValue)> = None;
    for (key, operand) in object {
        let Some(operator) = ATOMIC_OPERATORS
            .iter()
            .find(|candidate| *candidate == key)
            .copied()
        else {
            return Ok(None);
        };
        if found.is_some() {
            return Err(ProtocolError::InvalidParams(format!(
                "Field '{}' takes one update operator at a time",
                field.logical_name
            )));
        }
        found = Some((operator, operand));
    }
    Ok(found)
}

/// Whether an arithmetic operator can be applied to the field's type.
fn accepts_arithmetic(field: &FieldIr) -> bool {
    !field.is_array
        && matches!(
            &field.field_type,
            ResolvedFieldType::Scalar(
                ScalarType::Int
                    | ScalarType::BigInt
                    | ScalarType::Float
                    | ScalarType::Decimal { .. }
            )
        )
}

/// Resolve one field's update input into the right-hand side of a `SET` entry.
///
/// An arithmetic operator becomes `column = (column <op> $n)`, so the database
/// applies it to whatever the row holds at the moment the statement runs and
/// two concurrent increments both land.
fn field_assignment(
    state: &EngineState,
    model: &ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
    field: &FieldIr,
) -> Result<Option<Assignment>, ProtocolError> {
    let operator = match row_field_json(data_obj, field) {
        Some(json) => atomic_operator(json, field)?,
        None => None,
    };

    let Some((operator, operand)) = operator else {
        return Ok(
            field_input_value(state, data_obj, field, FieldInputMode::Update)?
                .map(Assignment::Value),
        );
    };
    let Some(op) = atomic_operator_sql(operator) else {
        return Ok(
            field_input_value(state, data_obj, field, FieldInputMode::Update)?
                .map(Assignment::Value),
        );
    };

    if !accepts_arithmetic(field) {
        return Err(ProtocolError::InvalidParams(format!(
            "'{}' takes a number, so field '{}' does not support it",
            operator, field.logical_name
        )));
    }
    // The new value is only known once the statement has run, and a backend
    // without RETURNING finds the updated rows by the key it captured before
    // it: a moving key would leave the read-back looking for a row that no
    // longer exists.
    if model
        .primary_key
        .fields()
        .iter()
        .any(|name| *name == field.logical_name || *name == field.db_name)
    {
        return Err(ProtocolError::InvalidParams(format!(
            "'{}' cannot be applied to primary-key field '{}'",
            operator, field.logical_name
        )));
    }
    let decimal_operand = match (&field.field_type, operand.as_str()) {
        (ResolvedFieldType::Scalar(ScalarType::Decimal { .. }), Some(_)) => {
            json_to_value_field(operand, &field.field_type).ok()
        }
        _ => None,
    };
    if !operand.is_number() && decimal_operand.is_none() {
        return Err(ProtocolError::InvalidParams(format!(
            "'{}' on field '{}' takes a number",
            operator, field.logical_name
        )));
    }

    let operand = match decimal_operand {
        Some(value) => value,
        None => json_to_value_field(operand, &field.field_type)?,
    };
    Ok(Some(Assignment::Expr(Expr::Binary {
        left: Box::new(Expr::column(&field.db_name)),
        op,
        right: Box::new(Expr::param(operand)),
    })))
}

fn updated_at_now_value() -> Value {
    Value::DateTime(chrono::Utc::now().naive_utc())
}

/// Whether a field is being written by an insert or by an update, which is
/// what decides where `updatedAt` and a function default take their value.
#[derive(Clone, Copy)]
pub(super) enum FieldInputMode {
    Create,
    Update,
}

pub(super) fn field_input_value(
    state: &EngineState,
    data_obj: &JsonMap<String, JsonValue>,
    field: &FieldIr,
    mode: FieldInputMode,
) -> Result<Option<Value>, ProtocolError> {
    if field.is_updated_at {
        return match row_field_json(data_obj, field) {
            Some(json_val) if !json_val.is_null() => {
                Ok(Some(convert_field_input(state, json_val, field)?))
            }
            // On insert the column's CURRENT_TIMESTAMP default supplies the
            // value, so it comes from the same clock and the same statement as
            // a `@default(now())` sibling. Computing it here instead made
            // `updatedAt` older than `createdAt` on every row.
            _ => match mode {
                FieldInputMode::Create => Ok(None),
                FieldInputMode::Update => Ok(Some(updated_at_now_value())),
            },
        };
    }

    let Some(json_val) = row_field_json(data_obj, field) else {
        return Ok(None);
    };

    let json_val = match atomic_operator(json_val, field)? {
        Some((operator, operand)) if atomic_operator_sql(operator).is_none() => operand,
        Some((operator, _)) => {
            return Err(ProtocolError::InvalidParams(match mode {
                FieldInputMode::Create => format!(
                    "'{}' derives the new value from the current one, and a create has none; write the value directly",
                    operator
                ),
                FieldInputMode::Update => format!(
                    "'{}' on field '{}' cannot be resolved here",
                    operator, field.logical_name
                ),
            }));
        }
        None => json_val,
    };

    if matches!(mode, FieldInputMode::Create)
        && json_val.is_null()
        && matches!(&field.default_value, Some(DefaultValue::Function(_)))
    {
        return Ok(None);
    }

    Ok(Some(convert_field_input(state, json_val, field)?))
}

/// Convert a single field's JSON input into a [`Value`], routing PostgreSQL
/// native composite-type fields through [`json_to_value_composite`] so they bind
/// as a record literal instead of an untyped text/JSON value. On backends that
/// store composites as JSON, the regular [`json_to_value_field`] path is used.
fn convert_field_input(
    state: &EngineState,
    json_val: &JsonValue,
    field: &FieldIr,
) -> Result<Value, ProtocolError> {
    crate::conversion::ensure_scalar_input(
        json_val,
        &field.field_type,
        field.is_array,
        &field.logical_name,
    )?;
    if let ResolvedFieldType::CompositeType { type_name, .. } = &field.field_type {
        if state.uses_native_composite_types() && !json_val.is_null() {
            if let Some(composite) = state.schema.get_composite_type(type_name) {
                return crate::conversion::json_to_value_composite(json_val, composite);
            }
        }
    }
    json_to_value_field(json_val, &field.field_type)
}

/// Whether an explicit `null` should leave the column out of the INSERT so the
/// database's own default fills it.
pub(super) fn should_omit_server_default(json_val: &JsonValue, field: &FieldIr) -> bool {
    json_val.is_null()
        && (field.is_updated_at || matches!(&field.default_value, Some(DefaultValue::Function(_))))
}

/// The columns and bound values of one inserted row.
///
/// A field this leaves out is one the database fills itself, so its column
/// never reaches the statement at all.
pub(super) fn insert_columns(
    state: &EngineState,
    model: &ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<(Vec<ColumnMarker>, Vec<Value>), ProtocolError> {
    let mut columns = Vec::with_capacity(model.fields.len());
    let mut values = Vec::with_capacity(model.fields.len());

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(value) = field_input_value(state, data_obj, field, FieldInputMode::Create)? {
            columns.push(field_marker(model, field));
            values.push(value);
        }
    }

    Ok((columns, values))
}

/// The `SET` entries of one update, including the `updatedAt` the caller did
/// not write.
pub(super) fn update_assignments(
    state: &EngineState,
    model: &ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<Vec<(ColumnMarker, Assignment)>, ProtocolError> {
    let mut assignments = Vec::with_capacity(model.fields.len());

    for field in &model.fields {
        if matches!(field.field_type, ResolvedFieldType::Relation(_)) {
            continue;
        }
        if let Some(assignment) = field_assignment(state, model, data_obj, field)? {
            assignments.push((field_marker(model, field), assignment));
        }
    }

    Ok(assignments)
}
