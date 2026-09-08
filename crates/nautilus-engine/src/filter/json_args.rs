//! The JSON `args` object of a read request parsed into [`QueryArgs`].
//!
//! Every key is read here and handed to the module that understands it —
//! `where` to the filter parser, `orderBy` to the ordering parser, `include`
//! and `select` to the projection parser — so an argument the engine does not
//! act on is refused instead of silently dropped.

use std::collections::{HashMap, HashSet};

use serde_json::Value as JsonValue;

use nautilus_core::{Value, VectorMetric};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{ResolvedFieldType, ScalarType};

use super::context::SchemaContext;
use super::include::{parse_include, parse_select};
use super::ordering::{parse_int, parse_order_by, parse_signed_int};
use super::types::{FieldTypeMap, QueryArgs, RelationMap, VectorNearestQuery};
use super::validate::{ensure_nearest_supported, ensure_select_include_exclusive};
use super::where_filter::parse_where_filter;
use crate::conversion::{json_to_value, json_to_value_field};

/// Reject a key in an `args` object that the engine does not act on.
///
/// `args` is a free-form JSON value, so an unrecognised key — a typo, or the
/// Prisma spelling of an argument this protocol names differently — used to be
/// dropped in silence and the caller got a result that answered a different
/// question than the one they asked.
pub(crate) fn ensure_known_arg_keys(
    args: &serde_json::Map<String, JsonValue>,
    method: &str,
    allowed: &[&str],
) -> Result<(), ProtocolError> {
    let Some(unknown) = args.keys().find(|key| !allowed.contains(&key.as_str())) else {
        return Ok(());
    };
    let hint = allowed
        .iter()
        .find(|candidate| {
            unknown
                .trim_start_matches('_')
                .eq_ignore_ascii_case(candidate)
        })
        .map(|candidate| format!(" (did you mean '{}'?)", candidate))
        .unwrap_or_default();
    Err(ProtocolError::InvalidParams(format!(
        "unknown argument '{}' in {} args{}; supported arguments are: {}",
        unknown,
        method,
        hint,
        allowed.join(", ")
    )))
}

/// The `args` keys a read query accepts.
const FIND_ARG_KEYS: [&str; 9] = [
    "where", "orderBy", "take", "skip", "cursor", "include", "select", "distinct", "nearest",
];

impl QueryArgs {
    /// Parse with no relation or field-type context (backward-compatible).
    pub fn parse(args: Option<JsonValue>) -> Result<Self, ProtocolError> {
        Self::parse_with_context(
            args,
            &RelationMap::new(),
            &FieldTypeMap::new(),
            SchemaContext::none(),
        )
    }

    /// Parse without relation context but with field-type context.
    ///
    /// Used by `update` / `delete` / `findUnique` handlers where the model is
    /// known but there are no eager-loaded relations.
    pub fn parse_typed(
        args: Option<JsonValue>,
        field_types: &FieldTypeMap,
    ) -> Result<Self, ProtocolError> {
        Self::parse_with_context(
            args,
            &RelationMap::new(),
            field_types,
            SchemaContext::none(),
        )
    }

    /// Parse with relation context so that `some` / `none` / `every` predicates
    /// and structured `include` objects can be understood.
    pub fn parse_with_relations(
        args: Option<JsonValue>,
        relations: &RelationMap,
        field_types: &FieldTypeMap,
    ) -> Result<Self, ProtocolError> {
        Self::parse_with_context(args, relations, field_types, SchemaContext::none())
    }

    /// Parse with relation context and full schema access so nested include
    /// payloads can reuse the child model's field mappings.
    pub(crate) fn parse_with_context(
        args: Option<JsonValue>,
        relations: &RelationMap,
        field_types: &FieldTypeMap,
        schema_context: SchemaContext<'_>,
    ) -> Result<Self, ProtocolError> {
        let args = match args {
            Some(JsonValue::Object(map)) => map,
            Some(_) => {
                return Err(ProtocolError::InvalidParams(
                    "args must be an object".to_string(),
                ));
            }
            None => {
                return Ok(QueryArgs {
                    filter: None,
                    order_by: vec![],
                    take: None,
                    skip: None,
                    include: HashMap::new(),
                    select: HashSet::new(),
                    cursor: None,
                    backward: false,
                    distinct: vec![],
                    nearest: None,
                    partition: None,
                    join: None,
                });
            }
        };

        ensure_known_arg_keys(&args, "query", &FIND_ARG_KEYS)?;

        let filter = if let Some(where_value) = args.get("where") {
            Some(parse_where_filter(
                where_value,
                relations,
                field_types,
                schema_context,
            )?)
        } else {
            None
        };

        let order_by = if let Some(order_value) = args.get("orderBy") {
            parse_order_by(order_value, Some(field_types))?
        } else {
            vec![]
        };

        let (take, backward) = if let Some(take_value) = args.get("take") {
            let n = parse_signed_int(take_value, "take")?;
            let magnitude = i32::try_from(n.unsigned_abs()).map_err(|_| {
                ProtocolError::InvalidParams(format!(
                    "take must fit in a 32-bit integer, got {}",
                    n
                ))
            })?;
            (Some(magnitude), n < 0)
        } else {
            (None, false)
        };

        let skip = if let Some(skip_value) = args.get("skip") {
            Some(parse_int(skip_value, "skip")?)
        } else {
            None
        };

        let cursor = if let Some(cursor_value) = args.get("cursor") {
            let obj = cursor_value.as_object().ok_or_else(|| {
                ProtocolError::InvalidParams("cursor must be an object".to_string())
            })?;
            let mut map = HashMap::new();
            for (k, v) in obj {
                map.insert(k.clone(), json_to_value(v)?);
            }
            Some(map)
        } else {
            None
        };

        let include = if let Some(include_value) = args.get("include") {
            parse_include(include_value, relations, schema_context)?
        } else {
            HashMap::new()
        };

        let select = if let Some(select_value) = args.get("select") {
            parse_select(select_value, field_types)?
        } else {
            HashSet::new()
        };

        ensure_select_include_exclusive(&select, &include)?;

        let distinct = if let Some(distinct_value) = args.get("distinct") {
            let arr = distinct_value.as_array().ok_or_else(|| {
                ProtocolError::InvalidParams(
                    "'distinct' must be an array of field names".to_string(),
                )
            })?;
            arr.iter()
                .map(|v| {
                    v.as_str()
                        .ok_or_else(|| {
                            ProtocolError::InvalidParams(
                                "each entry in 'distinct' must be a string field name".to_string(),
                            )
                        })
                        .map(str::to_string)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![]
        };

        let nearest = if let Some(nearest_value) = args.get("nearest") {
            Some(parse_nearest_query(nearest_value, field_types)?)
        } else {
            None
        };

        ensure_nearest_supported(
            nearest.as_ref(),
            take,
            backward,
            cursor.is_some(),
            &distinct,
        )?;

        Ok(QueryArgs {
            filter,
            order_by,
            take,
            skip,
            include,
            select,
            cursor,
            backward,
            distinct,
            nearest,
            partition: None,
            join: None,
        })
    }
}

fn parse_nearest_query(
    value: &JsonValue,
    field_types: &FieldTypeMap,
) -> Result<VectorNearestQuery, ProtocolError> {
    let obj = value
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("'nearest' must be an object".to_string()))?;

    let field = obj
        .get("field")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            ProtocolError::InvalidParams("'nearest.field' must be a string".to_string())
        })?
        .to_string();

    let field_type = field_types.get(&field).ok_or_else(|| {
        ProtocolError::InvalidParams(format!(
            "'nearest.field' references unknown field '{}'",
            field
        ))
    })?;

    let ResolvedFieldType::Scalar(ScalarType::Vector { .. }) = field_type else {
        return Err(ProtocolError::InvalidParams(format!(
            "'nearest.field' must reference a Vector field, got '{}'",
            field
        )));
    };

    let query_json = obj
        .get("query")
        .ok_or_else(|| ProtocolError::InvalidParams("'nearest.query' is required".to_string()))?;
    let query = match json_to_value_field(query_json, field_type)? {
        Value::Vector(values) => values,
        _ => {
            return Err(ProtocolError::InvalidParams(
                "'nearest.query' must be a vector".to_string(),
            ));
        }
    };

    let metric = match obj.get("metric").and_then(JsonValue::as_str) {
        Some("l2") => VectorMetric::L2,
        Some("innerProduct") => VectorMetric::InnerProduct,
        Some("cosine") => VectorMetric::Cosine,
        Some(other) => {
            return Err(ProtocolError::InvalidParams(format!(
                "Unsupported nearest metric '{}'; expected one of: l2, innerProduct, cosine",
                other
            )));
        }
        None => {
            return Err(ProtocolError::InvalidParams(
                "'nearest.metric' is required".to_string(),
            ));
        }
    };

    Ok(VectorNearestQuery {
        field,
        query,
        metric,
    })
}
