//! Java expressions that decode database rows and composite values.

use crate::extension_types::ExtensionRegistry;
use nautilus_schema::ir::{CompositeFieldIr, FieldIr, ModelIr, ResolvedFieldType, ScalarType};

use super::config::JavaConfig;

pub(super) fn generate_model_field_read(
    config: &JavaConfig,
    model: &ModelIr,
    field: &FieldIr,
) -> String {
    match &field.field_type {
        ResolvedFieldType::Relation(rel) => {
            if field.is_array {
                format!(
                    "        List<{target}> {name} = JsonSupport.asList(JsonSupport.firstPresent(row, \"{logical}_json\"), {target}::fromJsonNode);\n",
                    target = rel.target_model,
                    name = field.logical_name,
                    logical = field.logical_name,
                )
            } else {
                format!(
                    "        {target} {name} = JsonSupport.asObject(JsonSupport.firstPresent(row, \"{logical}_json\"), {target}::fromJsonNode);\n",
                    target = rel.target_model,
                    name = field.logical_name,
                    logical = field.logical_name,
                )
            }
        }
        _ => generate_regular_field_read(
            config,
            &field.logical_name,
            &field.db_name,
            Some(&model.db_name),
            &field.field_type,
            field.is_array,
        ),
    }
}

pub(super) fn generate_composite_field_read(
    config: &JavaConfig,
    field: &CompositeFieldIr,
) -> String {
    generate_regular_field_read(
        config,
        &field.logical_name,
        &field.db_name,
        None,
        &field.field_type,
        field.is_array,
    )
}

fn generate_regular_field_read(
    config: &JavaConfig,
    logical_name: &str,
    db_name: &str,
    table_name: Option<&str>,
    field_type: &ResolvedFieldType,
    is_array: bool,
) -> String {
    let source = match table_name {
        Some(table) => format!(
            "JsonSupport.firstPresent(row, \"{}__{}\", \"{}\")",
            table, db_name, logical_name
        ),
        None => format!(
            "JsonSupport.firstPresent(node, \"{}\", \"{}\")",
            db_name, logical_name
        ),
    };

    if is_array {
        match field_type {
            ResolvedFieldType::Scalar(scalar) => {
                if let Some(ext) = config.extensions.type_for_scalar(scalar) {
                    return format!(
                        "        List<{ty}> {name} = JsonSupport.asList({source}, {ty}::fromJsonNode);\n",
                        ty = ext.type_name,
                        name = logical_name,
                    );
                }
                let reader = array_reader_for_scalar(scalar);
                format!(
                    "        List<{ty}> {name} = JsonSupport.asList({source}, {reader});\n",
                    ty = base_java_type_name(field_type, &config.extensions),
                    name = logical_name,
                )
            }
            ResolvedFieldType::Enum { enum_name, .. } => format!(
                "        List<{enum_name}> {name} = JsonSupport.asList({source}, value -> JsonSupport.asEnum(value, {enum_name}.class));\n",
                name = logical_name,
            ),
            ResolvedFieldType::CompositeType { type_name, .. } => format!(
                "        List<{type_name}> {name} = JsonSupport.asList({source}, {type_name}::fromJsonNode);\n",
                name = logical_name,
            ),
            ResolvedFieldType::Relation(_) => unreachable!(),
        }
    } else {
        match field_type {
            ResolvedFieldType::Scalar(scalar) => {
                if let Some(ext) = config.extensions.type_for_scalar(scalar) {
                    return format!(
                        "        {ty} {name} = {ty}.fromJsonNode({source});\n",
                        ty = ext.type_name,
                        name = logical_name,
                    );
                }
                let reader = scalar_reader_for_type(scalar);
                format!(
                    "        {ty} {name} = JsonSupport.{reader}({source});\n",
                    ty = base_java_type_name(field_type, &config.extensions),
                    name = logical_name,
                )
            }
            ResolvedFieldType::Enum { enum_name, .. } => format!(
                "        {enum_name} {name} = JsonSupport.asEnum({source}, {enum_name}.class);\n",
                name = logical_name,
            ),
            ResolvedFieldType::CompositeType { type_name, .. } => format!(
                "        {type_name} {name} = JsonSupport.asObject({source}, {type_name}::fromJsonNode);\n",
                name = logical_name,
            ),
            ResolvedFieldType::Relation(_) => unreachable!(),
        }
    }
}

fn base_java_type_name(
    field_type: &ResolvedFieldType,
    extensions: &ExtensionRegistry,
) -> &'static str {
    if let ResolvedFieldType::Scalar(scalar) = field_type {
        if let Some(ext) = extensions.type_for_scalar(scalar) {
            return ext.type_name;
        }
    }

    match field_type {
        ResolvedFieldType::Scalar(ScalarType::String)
        | ResolvedFieldType::Scalar(ScalarType::Citext)
        | ResolvedFieldType::Scalar(ScalarType::Ltree)
        | ResolvedFieldType::Scalar(ScalarType::Geometry)
        | ResolvedFieldType::Scalar(ScalarType::Geography)
        | ResolvedFieldType::Scalar(ScalarType::Xml)
        | ResolvedFieldType::Scalar(ScalarType::Char { .. })
        | ResolvedFieldType::Scalar(ScalarType::VarChar { .. }) => "String",
        ResolvedFieldType::Scalar(ScalarType::Hstore) => "JsonSupport.Hstore",
        ResolvedFieldType::Scalar(ScalarType::Vector { .. }) => "List<Float>",
        ResolvedFieldType::Scalar(ScalarType::Boolean) => "Boolean",
        ResolvedFieldType::Scalar(ScalarType::Int) => "Integer",
        ResolvedFieldType::Scalar(ScalarType::BigInt) => "Long",
        ResolvedFieldType::Scalar(ScalarType::Float) => "Double",
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => "BigDecimal",
        ResolvedFieldType::Scalar(ScalarType::DateTime) => "OffsetDateTime",
        ResolvedFieldType::Scalar(ScalarType::Bytes) => "byte[]",
        ResolvedFieldType::Scalar(ScalarType::Json)
        | ResolvedFieldType::Scalar(ScalarType::Jsonb) => "JsonNode",
        ResolvedFieldType::Scalar(ScalarType::Uuid) => "UUID",
        _ => "Object",
    }
}

pub(super) fn scalar_reader_for_type(scalar: &ScalarType) -> &'static str {
    match scalar {
        ScalarType::String
        | ScalarType::Citext
        | ScalarType::Ltree
        | ScalarType::Geometry
        | ScalarType::Geography
        | ScalarType::Xml
        | ScalarType::Char { .. }
        | ScalarType::VarChar { .. } => "asString",
        ScalarType::Hstore => "asHstore",
        ScalarType::Vector { .. } => "asFloatList",
        ScalarType::Boolean => "asBoolean",
        ScalarType::Int => "asInteger",
        ScalarType::BigInt => "asLong",
        ScalarType::Float => "asDouble",
        ScalarType::Decimal { .. } => "asBigDecimal",
        ScalarType::DateTime => "asOffsetDateTime",
        ScalarType::Bytes => "asBytes",
        ScalarType::Json | ScalarType::Jsonb => "asJsonNode",
        ScalarType::Uuid => "asUuid",
    }
}

pub(super) fn array_reader_for_scalar(scalar: &ScalarType) -> &'static str {
    match scalar {
        ScalarType::String
        | ScalarType::Citext
        | ScalarType::Ltree
        | ScalarType::Geometry
        | ScalarType::Geography
        | ScalarType::Xml
        | ScalarType::Char { .. }
        | ScalarType::VarChar { .. } => "JsonSupport::asString",
        ScalarType::Hstore => "JsonSupport::asHstore",
        ScalarType::Vector { .. } => "JsonSupport::asFloatList",
        ScalarType::Boolean => "JsonSupport::asBoolean",
        ScalarType::Int => "JsonSupport::asInteger",
        ScalarType::BigInt => "JsonSupport::asLong",
        ScalarType::Float => "JsonSupport::asDouble",
        ScalarType::Decimal { .. } => "JsonSupport::asBigDecimal",
        ScalarType::DateTime => "JsonSupport::asOffsetDateTime",
        ScalarType::Bytes => "JsonSupport::asBytes",
        ScalarType::Json | ScalarType::Jsonb => "JsonSupport::asJsonNode",
        ScalarType::Uuid => "JsonSupport::asUuid",
    }
}
