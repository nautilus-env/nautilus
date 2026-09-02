use super::*;

pub(super) enum MutationResultData {
    Count(usize),
    Rows(Vec<Row>),
}

impl MutationResultData {
    /// The first returned row, or `None` when the statement answered with a
    /// count alone.
    pub(super) fn first_row(&self) -> Option<&Row> {
        match self {
            MutationResultData::Rows(rows) => rows.first(),
            MutationResultData::Count(_) => None,
        }
    }

    /// The returned rows, for a caller that asked for them.
    pub(super) fn into_rows(self, context: &str) -> Result<Vec<Row>, ProtocolError> {
        match self {
            MutationResultData::Rows(rows) => Ok(rows),
            MutationResultData::Count(_) => Err(ProtocolError::Internal(format!(
                "{context} path expected returned rows"
            ))),
        }
    }

    /// How many rows the statement affected, whichever shape it answered in.
    pub(super) fn into_count(self) -> usize {
        match self {
            MutationResultData::Count(count) => count,
            MutationResultData::Rows(rows) => rows.len(),
        }
    }
}

/// The primary key or unique constraint whose columns are exactly `keys`, or
/// `None` when no constraint matches.
///
/// A partial or mixed key would either fail in the database or silently match a
/// different index than the caller meant, so the match has to be exact.
pub(super) fn matching_unique_constraint<'a>(
    model: &'a ModelIr,
    keys: &[&str],
) -> Option<Vec<&'a str>> {
    let names_field = |name: &str| {
        model
            .scalar_fields()
            .find(|field| field.logical_name == name || field.db_name == name)
    };

    std::iter::once(model.primary_key.fields())
        .chain(
            model
                .unique_constraints
                .iter()
                .map(|constraint| constraint.fields.iter().map(String::as_str).collect()),
        )
        .find(|candidate: &Vec<&str>| {
            candidate.len() == keys.len()
                && candidate.iter().all(|name| {
                    keys.iter().any(|key| {
                        names_field(key).is_some_and(|field| {
                            field.logical_name == *name || field.db_name == *name
                        })
                    })
                })
        })
}

/// Reject an empty filter on a single-record `update` or `delete`.
///
/// Those operate on one row, so a filter that matches everything is a mistake
/// rather than an instruction; the `*Many` variants are where "every row" is
/// spelled out deliberately.
pub(super) fn ensure_single_record_filter(
    operation: &str,
    filter: &JsonValue,
) -> Result<(), ProtocolError> {
    let JsonValue::Object(filter_obj) = protocol_filter_body(filter) else {
        return Err(ProtocolError::InvalidFilter(format!(
            "{} where must be an object",
            operation
        )));
    };

    if filter_obj.is_empty() {
        return Err(ProtocolError::InvalidFilter(format!(
            "{} needs a where filter; use {}Many to change every row",
            operation, operation
        )));
    }

    Ok(())
}

/// Reject a `findUnique` filter that cannot identify at most one row.
///
/// Without this, a filter on an ordinary column is accepted and the first
/// matching row of however many is returned, which makes the answer arbitrary.
pub(super) fn ensure_unique_filter(
    model: &ModelIr,
    filter: &JsonValue,
) -> Result<(), ProtocolError> {
    let JsonValue::Object(filter_obj) = protocol_filter_body(filter) else {
        return Err(ProtocolError::InvalidFilter(
            "findUnique where must be an object".to_string(),
        ));
    };

    let keys: Vec<&str> = filter_obj.keys().map(String::as_str).collect();
    if matching_unique_constraint(model, &keys).is_some() {
        return Ok(());
    }

    let mut names = keys;
    names.sort_unstable();
    Err(ProtocolError::InvalidFilter(format!(
        "findUnique where [{}] does not match the primary key or any unique constraint of model '{}'",
        names.join(", "),
        model.logical_name
    )))
}

pub(super) fn qualify_model_filter(
    model: &ModelIr,
    logical_to_db: &std::collections::HashMap<String, String>,
    filter: Option<Expr>,
) -> Option<Expr> {
    filter.map(|expr| qualify_filter_columns(expr, &model.db_name, logical_to_db))
}

/// Unwrap a `{ "where": { ... } }` envelope down to the filter body.
///
/// Clients send the unique filter either bare or wrapped; both reach the
/// handlers through this.
pub(super) fn protocol_filter_body(filter: &JsonValue) -> &JsonValue {
    filter
        .as_object()
        .and_then(|obj| (obj.len() == 1).then_some(obj))
        .and_then(|obj| obj.get("where"))
        .unwrap_or(filter)
}

pub(super) fn parse_optional_model_filter(
    model: &ModelIr,
    filter: &JsonValue,
    field_types: &crate::filter::FieldTypeMap,
    logical_to_db: &std::collections::HashMap<String, String>,
) -> Result<Option<Expr>, ProtocolError> {
    let filter = protocol_filter_body(filter);
    let JsonValue::Object(filter_obj) = filter else {
        return Err(ProtocolError::InvalidFilter(
            "where must be an object".to_string(),
        ));
    };

    if filter_obj.is_empty() {
        return Ok(None);
    }

    let parsed = crate::filter::parse_where_filter(
        filter,
        &crate::filter::RelationMap::new(),
        field_types,
        crate::filter::SchemaContext::none(),
    )?;
    Ok(Some(qualify_filter_columns(
        parsed,
        &model.db_name,
        logical_to_db,
    )))
}

pub(super) fn parse_and_qualify_model_filter(
    model: &ModelIr,
    filter: &JsonValue,
    field_types: &crate::filter::FieldTypeMap,
    logical_to_db: &std::collections::HashMap<String, String>,
) -> Result<Expr, ProtocolError> {
    parse_optional_model_filter(model, filter, field_types, logical_to_db)?
        .ok_or_else(|| ProtocolError::InvalidFilter("where cannot be empty".to_string()))
}

pub(super) fn wrap_result(
    result_str: String,
    context: &str,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    serde_json::value::RawValue::from_string(result_str)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap {}: {}", context, e)))
}

pub(super) fn wrap_data_result(
    rows: &[Row],
    context: &str,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let data_raw = rows_to_raw_json(rows)?;
    let data_str = data_raw.get();
    let mut buf = String::with_capacity(data_str.len() + 9);
    buf.push_str("{\"data\":");
    buf.push_str(data_str);
    buf.push('}');
    wrap_result(buf, context)
}

pub(super) fn wrap_count_result(
    count: impl std::fmt::Display,
    context: &str,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    use std::fmt::Write;
    let mut buf = String::with_capacity(32);
    buf.push_str("{\"count\":");
    let _ = write!(buf, "{}", count);
    buf.push('}');
    wrap_result(buf, context)
}

pub(super) fn wrap_mutation_result(
    rows: &[Row],
    context: &str,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    use std::fmt::Write;
    let data_raw = rows_to_raw_json(rows)?;
    let data_str = data_raw.get();
    let mut buf = String::with_capacity(data_str.len() + 32);
    buf.push_str("{\"count\":");
    let _ = write!(buf, "{}", rows.len());
    buf.push_str(",\"data\":");
    buf.push_str(data_str);
    buf.push('}');
    wrap_result(buf, context)
}

pub(super) async fn execute_mutation_result(
    state: &EngineState,
    sql: &Sql,
    exec_tag: &'static str,
    tx_id: Option<&str>,
    scalar_hints: &[Option<ValueHint>],
    return_data: bool,
) -> Result<MutationResultData, ProtocolError> {
    if return_data {
        let rows = normalize_rows_with_hints(
            state.execute_query_on(sql, exec_tag, tx_id).await?,
            scalar_hints,
        )?;
        Ok(MutationResultData::Rows(rows))
    } else {
        let count = state.execute_affected_on(sql, exec_tag, tx_id).await?;
        Ok(MutationResultData::Count(count))
    }
}
