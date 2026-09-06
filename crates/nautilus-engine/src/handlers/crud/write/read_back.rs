//! Finding again the rows a statement just wrote, on a backend whose dialect
//! has no `RETURNING`.
//!
//! There the write and the read are two statements, so the second one needs a
//! predicate naming exactly the rows the first one touched: the key the caller
//! supplied, the one the server generated, or the primary-key values captured
//! before an update moved them. Both statements run on the same connection,
//! which is why the callers wrap them in a transaction.

use nautilus_connector::Row;
use nautilus_core::Expr;
use nautilus_migrate::DatabaseProvider;
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{DefaultValue, FieldIr, ModelIr};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::input::{field_input_value, FieldInputMode};
use crate::conversion::normalize_rows_with_hints;
use crate::handlers::crud::common::{parse_and_qualify_model_filter, MutationResultData};
use crate::handlers::crud::read::build_find_unique_sql;
use crate::state::EngineState;

/// Whether the database supplies this field's value on insert without being
/// told it, in a way the engine can recover afterwards.
fn is_autoincrement(field: &FieldIr) -> bool {
    matches!(
        &field.default_value,
        Some(DefaultValue::Function(function)) if function.name == "autoincrement"
    )
}

fn unresolvable_key(model: &ModelIr, field: &FieldIr) -> ProtocolError {
    ProtocolError::UnsupportedOperation(format!(
        "Returning the written '{}' row needs its '{}' value, and this backend cannot report a generated one; supply it in the data or set returnData to false",
        model.logical_name, field.logical_name
    ))
}

fn primary_key_field<'a>(model: &'a ModelIr, name: &str) -> Result<&'a FieldIr, ProtocolError> {
    model.find_field(name).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Model '{}' names unknown field '{}' in its primary key",
            model.logical_name, name
        ))
    })
}

fn qualified_column(model: &ModelIr, field: &FieldIr) -> Expr {
    Expr::column(format!("{}__{}", model.db_name, field.db_name))
}

/// Predicate identifying the row an `INSERT` just wrote, for a backend that
/// cannot return it inline.
///
/// A key the caller supplied is known outright. The only generated key that can
/// be recovered afterwards is MySQL's single `AUTO_INCREMENT` column, whose
/// value the server reports as `LAST_INSERT_ID()` — on the same connection,
/// which is why the read-back runs inside a transaction.
pub(super) fn inserted_row_filter(
    state: &EngineState,
    model: &ModelIr,
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<Expr, ProtocolError> {
    let mut conditions: Vec<Expr> = Vec::new();
    let mut generated: Option<&FieldIr> = None;

    for name in model.primary_key.fields() {
        let field = primary_key_field(model, name)?;
        let column = qualified_column(model, field);

        match field_input_value(state, data_obj, field, FieldInputMode::Create)? {
            Some(value) => conditions.push(column.eq(Expr::param(value))),
            None if generated.is_none() => {
                if state.provider() != DatabaseProvider::Mysql || !is_autoincrement(field) {
                    return Err(unresolvable_key(model, field));
                }
                generated = Some(field);
                conditions.push(column.eq(Expr::FunctionCall {
                    name: "LAST_INSERT_ID".to_string(),
                    args: vec![],
                }));
            }
            None => return Err(unresolvable_key(model, field)),
        }
    }

    conditions.into_iter().reduce(Expr::and).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Model '{}' has no primary key to read a written row back by",
            model.logical_name
        ))
    })
}

/// Predicate matching the rows an `UPDATE` touched, keyed on the primary key
/// values captured before it ran.
///
/// An update that assigns a primary-key column moves the row, so the assigned
/// value is what the read-back looks for.
pub(super) fn updated_rows_filter(
    state: &EngineState,
    model: &ModelIr,
    rows: &[Row],
    data_obj: &JsonMap<String, JsonValue>,
) -> Result<Option<Expr>, ProtocolError> {
    let mut per_row = Vec::with_capacity(rows.len());

    for row in rows {
        let mut conditions: Vec<Expr> = Vec::new();
        for name in model.primary_key.fields() {
            let field = primary_key_field(model, name)?;
            let value = match field_input_value(state, data_obj, field, FieldInputMode::Update)? {
                Some(assigned) => assigned,
                None => row
                    .get(&format!("{}__{}", model.db_name, field.db_name))
                    .cloned()
                    .ok_or_else(|| unresolvable_key(model, field))?,
            };
            conditions.push(qualified_column(model, field).eq(Expr::param(value)));
        }
        if let Some(predicate) = conditions.into_iter().reduce(Expr::and) {
            per_row.push(predicate);
        }
    }

    Ok(per_row.into_iter().reduce(Expr::or))
}

/// Fetch the upserted row on dialects without `RETURNING` (MySQL).
///
/// The write itself stays atomic; only the read is a second round-trip, so a
/// concurrent writer can still change the row between the two statements unless
/// the caller wraps the upsert in a transaction.
pub(super) async fn read_back_upserted_row(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    tx_id: Option<&str>,
) -> Result<MutationResultData, ProtocolError> {
    let metadata = state.model_metadata(model);
    let qualified_filter = parse_and_qualify_model_filter(
        model,
        filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    let (sql, row_hints) = build_find_unique_sql(
        state,
        model,
        qualified_filter,
        &std::collections::HashSet::new(),
    )?;

    let rows = normalize_rows_with_hints(
        state.execute_query_on(&sql, "Query", tx_id).await?,
        &row_hints,
    )?;
    Ok(MutationResultData::Rows(rows))
}
