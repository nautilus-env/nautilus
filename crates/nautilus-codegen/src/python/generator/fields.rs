//! Python model fields, write inputs, filters and aggregate types.

use crate::extension_types::{ExtensionRegistry, ExtensionType, ExtensionWireKind};
use crate::model_view::{FieldView, ModelView};
use crate::python::type_mapper::{
    get_base_python_type, get_default_value, get_filter_operators_for_field,
};
use heck::ToSnakeCase;
use nautilus_schema::ir::{ResolvedFieldType, ScalarType, SchemaIr};
use serde::Serialize;

use super::types::{
    add_none_to_python_union, exact_input_python_type, exact_output_python_type,
    input_base_python_type, output_base_python_type,
};

/// Python field types, model defaults and wire conversion expressions.
#[derive(Debug, Clone, Serialize)]
pub(super) struct PythonFieldContext {
    pub(super) name: String,
    pub(super) logical_name: String,
    pub(super) db_name: String,
    pub(super) python_type: String,
    pub(super) input_python_type: String,
    pub(super) model_python_type: String,
    pub(super) base_type: String,
    pub(super) raw_base_type: String,
    pub(super) extension_coercer: String,
    pub(super) extension_input_serializer: String,
    pub(super) is_optional: bool,
    pub(super) is_array: bool,
    pub(super) is_enum: bool,
    pub(super) has_default: bool,
    pub(super) default: String,
    pub(super) model_has_default: bool,
    pub(super) model_default: String,
    pub(super) is_pk: bool,
    pub(super) doc_comment: String,
    pub(super) index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FilterOperatorContext {
    suffix: String,
    python_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WhereInputFieldContext {
    name: String,
    python_type: String,
    where_python_type: String,
    is_nullable: bool,
    is_vector: bool,
    operators: Vec<FilterOperatorContext>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CreateInputFieldContext {
    name: String,
    python_type: String,
    is_required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct UpdateInputFieldContext {
    name: String,
    python_type: String,
    /// Name of the operator `TypedDict` a numeric column also accepts, empty
    /// for a column whose type cannot take arithmetic.
    operator_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OrderByFieldContext {
    name: String,
    is_dotted: bool,
}

/// Context for a scalar field used in aggregate input types (avg/sum/min/max).
#[derive(Debug, Clone, Serialize)]
pub(super) struct AggregateFieldContext {
    name: String,
    python_type: String,
}

/// The per-field template contexts and import flags a model needs, collected in
/// a single pass over its scalar fields.
#[derive(Default)]
pub(super) struct PythonFieldSets {
    pub(super) scalar: Vec<PythonFieldContext>,
    pub(super) create: Vec<PythonFieldContext>,
    pub(super) where_input: Vec<WhereInputFieldContext>,
    pub(super) create_input: Vec<CreateInputFieldContext>,
    pub(super) update_input: Vec<UpdateInputFieldContext>,
    pub(super) order_by: Vec<OrderByFieldContext>,
    pub(super) numeric: Vec<AggregateFieldContext>,
    pub(super) orderable: Vec<AggregateFieldContext>,
    pub(super) has_datetime: bool,
    pub(super) has_uuid: bool,
    pub(super) has_decimal: bool,
    pub(super) has_dict: bool,
}

/// The operator `TypedDict` that accompanies a numeric column's own type.
///
/// One per operand type rather than a generic alias: a generic `TypedDict`
/// needs Python 3.11, and the generated clients target older interpreters too.
fn numeric_operator_type_name(base_python_type: &str) -> String {
    match base_python_type {
        "int" => "IntFieldUpdate".to_string(),
        "float" => "FloatFieldUpdate".to_string(),
        "Decimal" => "DecimalFieldUpdate".to_string(),
        _ => String::new(),
    }
}

pub(super) fn build_scalar_fields(
    view: &ModelView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> PythonFieldSets {
    let mut sets = PythonFieldSets::default();

    for scalar in &view.scalars {
        let field = scalar.field;
        let extension_type = scalar.extension_type;

        if let ResolvedFieldType::Scalar(scalar_type) = &field.field_type {
            match scalar_type {
                ScalarType::DateTime => sets.has_datetime = true,
                ScalarType::Uuid => sets.has_uuid = true,
                ScalarType::Decimal { .. } => sets.has_decimal = true,
                ScalarType::Json | ScalarType::Jsonb | ScalarType::Hstore => sets.has_dict = true,
                _ => {}
            }
        }
        if extension_type.is_some_and(|ty| ty.wire_kind == ExtensionWireKind::Hstore) {
            sets.has_dict = true;
        }

        let input_python_type =
            exact_input_python_type(field, input_base_python_type(field, &ir.enums, extensions));
        let base_python_type = get_base_python_type(field, &ir.enums);

        let field_ctx = scalar_field_context(scalar, ir, extensions);
        sets.create.push(field_ctx.clone());
        sets.scalar.push(field_ctx);

        sets.where_input.push(where_input_field(
            field,
            ir,
            extension_type,
            &base_python_type,
        ));

        sets.create_input.push(CreateInputFieldContext {
            name: field.logical_name.clone(),
            python_type: input_python_type.clone(),
            is_required: scalar.requires_create_value(),
        });

        let is_auto_pk = scalar.is_database_generated() && scalar.is_pk;
        if !is_auto_pk {
            let operator_type = match scalar.numeric_scalar() {
                Some(_) => numeric_operator_type_name(&base_python_type),
                None => String::new(),
            };
            sets.update_input.push(UpdateInputFieldContext {
                name: field.logical_name.clone(),
                python_type: input_python_type,
                operator_type,
            });
        }

        if scalar.numeric_scalar().is_some() {
            sets.numeric.push(AggregateFieldContext {
                name: field.logical_name.clone(),
                python_type: base_python_type.clone(),
            });
        }

        if scalar.is_orderable() {
            sets.order_by.push(OrderByFieldContext {
                name: field.logical_name.clone(),
                is_dotted: false,
            });
            sets.orderable.push(AggregateFieldContext {
                name: field.logical_name.clone(),
                python_type: base_python_type,
            });
        }
    }

    sets.order_by.extend(
        view.dotted_order_by
            .iter()
            .map(|dotted| OrderByFieldContext {
                name: dotted.path(),
                is_dotted: true,
            }),
    );
    sets
}

fn scalar_field_context(
    scalar: &FieldView<'_>,
    ir: &SchemaIr,
    extensions: &ExtensionRegistry,
) -> PythonFieldContext {
    let field = scalar.field;
    let extension_type = scalar.extension_type;
    let output_base_type = output_base_python_type(field, &ir.enums, extensions);
    let python_type = exact_output_python_type(field, output_base_type.clone());
    let input_python_type =
        exact_input_python_type(field, input_base_python_type(field, &ir.enums, extensions));
    let raw_base_type = match &field.field_type {
        ResolvedFieldType::Scalar(s) => {
            crate::python::type_mapper::scalar_to_python_type(s).to_string()
        }
        ResolvedFieldType::Enum { enum_name, .. } => enum_name.clone(),
        _ => "Any".to_string(),
    };

    let mut default_val = get_default_value(field);
    if let Some(ref def) = default_val {
        if let ResolvedFieldType::Enum { enum_name, .. } = &field.field_type {
            if !def.contains('.') && !def.contains('(') && def != "None" {
                default_val = Some(format!("{}.{}", enum_name, def));
            }
        }
    }

    let (model_has_default, model_default) = if field.is_array {
        (true, "Field(default_factory=list)".to_string())
    } else if !field.is_required {
        (true, "None".to_string())
    } else {
        (false, String::new())
    };

    PythonFieldContext {
        name: field.logical_name.to_snake_case(),
        logical_name: field.logical_name.clone(),
        db_name: field.db_name.clone(),
        model_python_type: python_type.clone(),
        python_type,
        input_python_type,
        base_type: output_base_type,
        raw_base_type,
        extension_coercer: extension_wire_adapter(field, extension_type, "from_wire"),
        extension_input_serializer: extension_wire_adapter(field, extension_type, "to_wire_input"),
        is_optional: !field.is_required,
        is_array: field.is_array,
        is_enum: scalar.is_enum(),
        model_has_default,
        model_default,
        is_pk: scalar.is_pk,
        doc_comment: scalar.doc_comment.clone(),
        has_default: default_val.is_some(),
        default: default_val.unwrap_or_default(),
        index: scalar.index,
    }
}

/// The Python expression that converts a field between its wire form and its
/// extension type, mapping over the elements of an array field.
fn extension_wire_adapter(
    field: &nautilus_schema::ir::FieldIr,
    extension_type: Option<ExtensionType>,
    method: &str,
) -> String {
    extension_type
        .map(|ty| {
            if field.is_array {
                format!(
                    "lambda v: [{}.{}(item) for item in v] if isinstance(v, list) else v",
                    ty.type_name, method
                )
            } else {
                format!("{}.{}", ty.type_name, method)
            }
        })
        .unwrap_or_default()
}

fn where_input_field(
    field: &nautilus_schema::ir::FieldIr,
    ir: &SchemaIr,
    extension_type: Option<ExtensionType>,
    base_python_type: &str,
) -> WhereInputFieldContext {
    let is_nullable = !field.is_required && !field.is_array;
    WhereInputFieldContext {
        name: field.logical_name.clone(),
        python_type: base_python_type.to_string(),
        where_python_type: extension_type
            .map(|ty| {
                let type_expr = ty.python_filter_input();
                if is_nullable {
                    add_none_to_python_union(type_expr)
                } else {
                    type_expr
                }
            })
            .unwrap_or_default(),
        is_nullable,
        is_vector: field.is_vector(),
        operators: get_filter_operators_for_field(field, &ir.enums)
            .into_iter()
            .map(|op| FilterOperatorContext {
                suffix: op.suffix,
                python_type: op.type_name,
            })
            .collect(),
    }
}
