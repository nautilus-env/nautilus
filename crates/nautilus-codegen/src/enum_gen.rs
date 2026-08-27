//! Enum code generator.

use anyhow::Result;
use nautilus_schema::ir::EnumIr;
use std::collections::HashMap;
use tera::Context;

use crate::generator::TEMPLATES;

/// Generate code for an enum type.
pub fn generate_enum(enum_ir: &EnumIr) -> Result<String> {
    let mut context = Context::new();
    context.insert("enum_name", &enum_ir.logical_name);
    context.insert("variants", &enum_ir.variants);

    crate::template::render(&TEMPLATES, "enum.tera", &context)
}

/// Generate code for all enums in the schema.
pub fn generate_all_enums(enums: &HashMap<String, EnumIr>) -> Result<String> {
    let mut output = String::new();

    output.push_str("//! Generated enum types.\n\n");

    for enum_ir in enums.values() {
        output.push_str(&generate_enum(enum_ir)?);
        output.push('\n');
    }

    Ok(output)
}
