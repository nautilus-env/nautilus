//! Private Python modules with the existing per-model import facade.

use anyhow::Result;
use heck::ToSnakeCase;
use nautilus_schema::ir::SchemaIr;

use super::{model_context, templates::render};
use crate::{extension_types::ExtensionRegistry, GeneratedFile};

pub(crate) struct ModelFiles {
    pub(crate) facades: Vec<GeneratedFile>,
    pub(crate) parts: Vec<GeneratedFile>,
}

pub(crate) fn generate_python_model_files(
    ir: &SchemaIr,
    is_async: bool,
    recursive_type_depth: usize,
    extensions: &ExtensionRegistry,
) -> Result<ModelFiles> {
    let mut files = ModelFiles {
        facades: Vec::new(),
        parts: Vec::new(),
    };
    for model in ir.models.values() {
        let mut context = model_context(model, ir, is_async, recursive_type_depth, extensions);
        let snake_name = model.logical_name.to_snake_case();
        let base = if model.is_view { "Read" } else { "Write" };
        context.insert(
            "delegate_bases",
            &format!(
                "(_{}{base}, _{}Aggregate)",
                model.logical_name, model.logical_name
            ),
        );
        files.facades.push((
            format!("{snake_name}.py"),
            render("files/facade.py.tera", &context)?,
        ));
        for part in ["inputs", "events", "codec", "read", "write", "aggregate"] {
            if part == "write" && model.is_view {
                continue;
            }
            files.parts.push((
                format!("_{snake_name}_{part}.py"),
                render(&format!("files/{part}.py.tera"), &context)?,
            ));
        }
    }
    files.facades.sort_by(|left, right| left.0.cmp(&right.0));
    files.parts.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}
