//! JavaScript model fields, write inputs, filters and aggregate types.

use crate::extension_types::{ExtensionRegistry, ExtensionType};
use crate::js::type_mapper::{
    get_base_ts_type, get_filter_operators_for_field, get_ts_default_value, scalar_to_ts_type,
};
use crate::model_view::ModelView;
use nautilus_schema::ir::{FieldIr, ResolvedFieldType, ScalarType, SchemaIr};
use serde::Serialize;

use super::types::{
    exact_input_ts_type, exact_output_ts_type, input_base_ts_type, output_base_ts_type,
};

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsFieldContext {
    /// Logical JS field name (camelCase, same as schema logical name).
    pub(super) name: String,
    /// Logical name from the schema IR (may differ from `name` after `@map`).
    pub(super) logical_name: String,
    /// Database column name.
    pub(super) db_name: String,
    /// Full TypeScript type, e.g. `string | null`, `number[]`.
    pub(super) ts_type: String,
    pub(super) input_ts_type: String,
    /// Inner base type without wrappers, e.g. `string`, `number`, `Date`.
    pub(super) base_type: String,
    pub(super) raw_base_type: String,
    pub(super) extension_coercer: String,
    pub(super) extension_input_serializer: String,
    pub(super) is_optional: bool,
    pub(super) is_array: bool,
    pub(super) is_enum: bool,
    pub(super) has_default: bool,
    pub(super) default: String,
    pub(super) is_pk: bool,
    pub(super) doc_comment: String,
    pub(super) index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsFilterOperatorContext {
    suffix: String,
    ts_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsWhereInputFieldContext {
    name: String,
    /// Base TS type used by the template to pick the right filter interface.
    base_type: String,
    ts_type: String,
    where_ts_type: String,
    is_nullable: bool,
    is_vector: bool,
    operators: Vec<JsFilterOperatorContext>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsCreateInputFieldContext {
    name: String,
    ts_type: String,
    is_required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsUpdateInputFieldContext {
    name: String,
    ts_type: String,
    /// The `T | { increment: T, … }` union a numeric column accepts, empty for
    /// a column whose type cannot take arithmetic.
    operator_ts_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsOrderByFieldContext {
    name: String,
    is_dotted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JsAggregateFieldContext {
    name: String,
    ts_type: String,
}

/// The per-field template contexts a model needs, collected in a single pass
/// over the shared [`ModelView`].
#[derive(Default)]
pub(super) struct JsFieldSets {
    pub(super) scalar: Vec<JsFieldContext>,
    pub(super) where_input: Vec<JsWhereInputFieldContext>,
    pub(super) create_input: Vec<JsCreateInputFieldContext>,
    pub(super) update_input: Vec<JsUpdateInputFieldContext>,
    pub(super) order_by: Vec<JsOrderByFieldContext>,
    pub(super) numeric: Vec<JsAggregateFieldContext>,
    pub(super) orderable: Vec<JsAggregateFieldContext>,
}

pub(super) fn build_scalar_fields(
    view: &ModelView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> JsFieldSets {
    let mut sets = JsFieldSets::default();

    for scalar in &view.scalars {
        let field = scalar.field;
        let extension_type = scalar.extension_type;

        let base_type = output_base_ts_type(field, &ir.enums, extensions);
        let ts_type = exact_output_ts_type(field, base_type.clone());
        let input_ts_type = exact_input_ts_type(field, input_base_ts_type(field, extensions));
        let raw_base_type = get_base_ts_type(field, &ir.enums);
        let auto_generated = scalar.is_database_generated();
        let default_val = get_ts_default_value(field);

        sets.scalar.push(JsFieldContext {
            name: field.logical_name.clone(),
            logical_name: field.logical_name.clone(),
            db_name: field.db_name.clone(),
            ts_type: ts_type.clone(),
            input_ts_type: input_ts_type.clone(),
            base_type: base_type.clone(),
            raw_base_type: raw_base_type.clone(),
            extension_coercer: extension_wire_adapter(field, extension_type, WireAdapter::From),
            extension_input_serializer: extension_wire_adapter(
                field,
                extension_type,
                WireAdapter::ToInput,
            ),
            is_optional: !field.is_required,
            is_array: field.is_array,
            is_enum: scalar.is_enum(),
            has_default: default_val.is_some(),
            default: default_val.unwrap_or_default(),
            is_pk: scalar.is_pk,
            doc_comment: scalar.doc_comment.clone(),
            index: scalar.index,
        });

        sets.where_input.push(where_input_field(
            field,
            ir,
            extension_type,
            &raw_base_type,
            &ts_type,
        ));

        if !auto_generated {
            sets.create_input.push(JsCreateInputFieldContext {
                name: field.logical_name.clone(),
                ts_type: input_ts_type.clone(),
                is_required: scalar.requires_create_value(),
            });
        }

        let is_auto_pk = auto_generated
            && scalar.is_pk
            && matches!(
                field.field_type,
                ResolvedFieldType::Scalar(ScalarType::Int | ScalarType::BigInt)
            );
        if !is_auto_pk {
            let operator_ts_type = scalar
                .numeric_scalar()
                .map(|scalar_type| {
                    let operand = scalar_to_ts_type(scalar_type);
                    format!(
                        "{{ set?: {operand}; increment?: {operand}; decrement?: {operand};                          multiply?: {operand}; divide?: {operand} }}"
                    )
                })
                .unwrap_or_default();
            sets.update_input.push(JsUpdateInputFieldContext {
                name: field.logical_name.clone(),
                ts_type: input_ts_type,
                operator_ts_type,
            });
        }

        if let Some(scalar_type) = scalar.numeric_scalar() {
            sets.numeric.push(JsAggregateFieldContext {
                name: field.logical_name.clone(),
                ts_type: scalar_to_ts_type(scalar_type).to_string(),
            });
        }

        if scalar.is_orderable() {
            sets.order_by.push(JsOrderByFieldContext {
                name: field.logical_name.clone(),
                is_dotted: false,
            });
            sets.orderable.push(JsAggregateFieldContext {
                name: field.logical_name.clone(),
                ts_type: base_type,
            });
        }
    }

    sets.order_by.extend(
        view.dotted_order_by
            .iter()
            .map(|dotted| JsOrderByFieldContext {
                name: dotted.path(),
                is_dotted: true,
            }),
    );
    sets
}

enum WireAdapter {
    From,
    ToInput,
}

/// The JavaScript expression that converts a field between its wire form and
/// its extension type, mapping over the elements of an array field.
fn extension_wire_adapter(
    field: &FieldIr,
    extension_type: Option<ExtensionType>,
    adapter: WireAdapter,
) -> String {
    let Some(ty) = extension_type else {
        return String::new();
    };
    let method = match adapter {
        WireAdapter::From => "from",
        WireAdapter::ToInput => "toWireInput",
    };
    if field.is_array {
        format!(
            "(value) => Array.isArray(value) ? value.map(item => {}.{}(item)) : value",
            ty.type_name, method
        )
    } else {
        format!("{}.{}", ty.type_name, method)
    }
}

fn where_input_field(
    field: &FieldIr,
    ir: &SchemaIr,
    extension_type: Option<ExtensionType>,
    raw_base_type: &str,
    ts_type: &str,
) -> JsWhereInputFieldContext {
    let is_nullable = !field.is_required && !field.is_array;
    JsWhereInputFieldContext {
        name: field.logical_name.clone(),
        base_type: raw_base_type.to_string(),
        ts_type: ts_type.to_string(),
        where_ts_type: extension_type
            .map(|ty| {
                let type_expr = ty.ts_filter_input();
                if is_nullable {
                    format!("{type_expr} | null")
                } else {
                    type_expr
                }
            })
            .unwrap_or_default(),
        is_nullable,
        is_vector: field.is_vector(),
        operators: get_filter_operators_for_field(field, &ir.enums)
            .into_iter()
            .map(|op| JsFilterOperatorContext {
                suffix: op.suffix,
                ts_type: op.type_name,
            })
            .collect(),
    }
}
