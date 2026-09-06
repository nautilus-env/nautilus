//! `query.upsert`: one `INSERT ... ON CONFLICT DO UPDATE`.
//!
//! The conflict target comes from the `where` filter, which therefore has to
//! name a whole unique constraint and nothing else. The create half and the
//! update half go through the same input rules as a plain create and update,
//! so the row a conflict writes matches the one an insert would have.

use nautilus_connector::Row;
use nautilus_core::{Insert, InsertCapacity, OnConflict};
use nautilus_protocol::{ProtocolError, RpcRequest, UpsertParams};
use nautilus_schema::ir::{FieldIr, ModelIr};
use serde_json::Value as JsonValue;

use super::input::{insert_columns, update_assignments};
use super::read_back::read_back_upserted_row;
use crate::conversion::check_protocol_version;
use crate::handlers::crud::common::{
    execute_mutation_result, matching_unique_constraint, protocol_filter_body, wrap_count_result,
    wrap_mutation_result, MutationResultData,
};
use crate::handlers::{field_marker, get_writable_model_or_error, parse_params};
use crate::state::EngineState;

/// Resolve the conflict target of an upsert from its unique `where` filter.
///
/// `INSERT ... ON CONFLICT` needs the exact column list of one unique index, so
/// the filter has to name a whole constraint and nothing else — a partial or
/// mixed key would either fail in the database or silently match a different
/// index than the caller meant.
fn unique_conflict_target<'a>(
    model: &'a ModelIr,
    filter: &JsonValue,
) -> Result<Vec<&'a FieldIr>, ProtocolError> {
    let JsonValue::Object(filter_obj) = protocol_filter_body(filter) else {
        return Err(ProtocolError::InvalidFilter(
            "upsert where must be an object".to_string(),
        ));
    };

    if filter_obj.is_empty() {
        return Err(ProtocolError::InvalidFilter(
            "upsert where cannot be empty".to_string(),
        ));
    }

    let mut filter_fields = Vec::with_capacity(filter_obj.len());
    for key in filter_obj.keys() {
        let field = model
            .scalar_fields()
            .find(|field| field.logical_name == *key || field.db_name == *key)
            .ok_or_else(|| {
                ProtocolError::InvalidFilter(format!(
                    "Unknown field '{}' in upsert where on model '{}'",
                    key, model.logical_name
                ))
            })?;
        filter_fields.push(field);
    }

    let keys: Vec<&str> = filter_obj.keys().map(String::as_str).collect();

    let Some(candidate) = matching_unique_constraint(model, &keys) else {
        let mut names: Vec<&str> = filter_obj.keys().map(String::as_str).collect();
        names.sort_unstable();
        return Err(ProtocolError::InvalidFilter(format!(
            "upsert where [{}] does not match the primary key or any unique constraint of model '{}'",
            names.join(", "),
            model.logical_name
        )));
    };

    Ok(candidate
        .iter()
        .filter_map(|name| {
            filter_fields
                .iter()
                .copied()
                .find(|field| field.logical_name == *name || field.db_name == *name)
        })
        .collect())
}

async fn execute_upsert(
    state: &EngineState,
    params: UpsertParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_writable_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);

    let create_obj = params
        .create
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("create must be an object".to_string()))?;
    let update_obj = params
        .update
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("update must be an object".to_string()))?;

    let target_fields = unique_conflict_target(model, &params.filter)?;

    let (columns, values) = insert_columns(state, model, create_obj)?;

    for target in &target_fields {
        if !columns.iter().any(|column| column.name == target.db_name) {
            return Err(ProtocolError::InvalidParams(format!(
                "upsert create data must set '{}' because it is part of the conflict target",
                target.logical_name
            )));
        }
    }

    // An empty `update` means "insert or leave alone", so the conflict clause
    // assigns nothing — not even the `updatedAt` a real update would refresh.
    let assignments = if update_obj.is_empty() {
        Vec::new()
    } else {
        update_assignments(state, model, update_obj)?
    };

    let returns_inline = params.return_data && state.dialect.supports_returning();

    let mut builder = Insert::into_table(crate::metadata::model_table(model))
        .with_capacity(InsertCapacity {
            columns: columns.len(),
            rows: 1,
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        })
        .columns(columns)
        .values(values)
        .on_conflict(OnConflict::do_update(
            target_fields
                .iter()
                .map(|field| field_marker(model, field))
                .collect(),
            assignments,
        ));
    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build upsert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert_owned(insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if params.return_data && !returns_inline {
        state
            .execute_affected_on(&sql, "Insert", tx_id.as_deref())
            .await?;
        return read_back_upserted_row(state, model, &params.filter, tx_id.as_deref()).await;
    }

    execute_mutation_result(
        state,
        &sql,
        "Insert",
        tx_id.as_deref(),
        metadata.scalar_hints(),
        returns_inline,
    )
    .await
}

/// Handle `query.upsert`.
pub(in crate::handlers) async fn handle_upsert(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpsertParams = parse_params(&request, "upsert")?;

    match execute_upsert(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "upsert result"),
        MutationResultData::Count(count) => wrap_count_result(count, "upsert result"),
    }
}

pub(in crate::handlers) async fn handle_upsert_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: UpsertParams = parse_params(&request, "upsert")?;
    execute_upsert(state, params).await?.into_rows("upsert")
}

pub(in crate::handlers) async fn handle_upsert_typed(
    state: &EngineState,
    params: UpsertParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_upsert(state, params).await?.into_rows("upsert")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_schema::validate_schema_source;

    fn model_of(source: &str, name: &str) -> ModelIr {
        validate_schema_source(source)
            .expect("schema should validate")
            .ir
            .models
            .into_values()
            .find(|model| model.logical_name == name)
            .expect("model missing")
    }

    fn user_model() -> ModelIr {
        model_of(
            r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  team  String
  slot  Int
  name  String

  @@unique([team, slot])
}
"#,
            "User",
        )
    }

    #[test]
    fn conflict_target_accepts_a_single_column_unique_constraint() {
        let model = user_model();
        let filter = serde_json::json!({ "where": { "email": "alice@example.com" } });

        let target = unique_conflict_target(&model, &filter).expect("email is unique");

        assert_eq!(
            target
                .iter()
                .map(|field| field.logical_name.as_str())
                .collect::<Vec<_>>(),
            vec!["email"]
        );
    }

    #[test]
    fn conflict_target_accepts_the_primary_key() {
        let model = user_model();
        let filter = serde_json::json!({ "id": 7 });

        let target = unique_conflict_target(&model, &filter).expect("id is the primary key");

        assert_eq!(target.len(), 1);
        assert_eq!(target[0].logical_name, "id");
    }

    #[test]
    fn conflict_target_orders_columns_as_the_constraint_declares_them() {
        let model = user_model();
        let filter = serde_json::json!({ "slot": 3, "team": "blue" });

        let target = unique_conflict_target(&model, &filter).expect("(team, slot) is unique");

        assert_eq!(
            target
                .iter()
                .map(|field| field.logical_name.as_str())
                .collect::<Vec<_>>(),
            vec!["team", "slot"]
        );
    }

    #[test]
    fn conflict_target_rejects_a_partial_composite_key() {
        let model = user_model();
        let filter = serde_json::json!({ "team": "blue" });

        let error = unique_conflict_target(&model, &filter)
            .expect_err("half of a composite unique key is not a conflict target");

        assert!(
            matches!(&error, ProtocolError::InvalidFilter(message) if message.contains("unique constraint")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn conflict_target_rejects_a_non_unique_column() {
        let model = user_model();
        let filter = serde_json::json!({ "name": "Alice" });

        let error =
            unique_conflict_target(&model, &filter).expect_err("name carries no unique constraint");

        assert!(matches!(error, ProtocolError::InvalidFilter(_)));
    }

    #[test]
    fn conflict_target_rejects_an_unknown_field() {
        let model = user_model();
        let filter = serde_json::json!({ "nickname": "Ali" });

        let error =
            unique_conflict_target(&model, &filter).expect_err("nickname is not a model field");

        assert!(
            matches!(&error, ProtocolError::InvalidFilter(message) if message.contains("Unknown field")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn conflict_target_rejects_an_empty_filter() {
        let model = user_model();
        let filter = serde_json::json!({});

        assert!(unique_conflict_target(&model, &filter).is_err());
    }
}
