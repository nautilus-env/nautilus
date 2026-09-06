//! Getting from a schema path to the IR a client is generated from.

use anyhow::{Context, Result};
use std::path::PathBuf;

use nautilus_schema::ir::{ResolvedFieldType, SchemaIr};
use nautilus_schema::{parse_schema_source, SchemaSet};

use crate::report;

/// Auto-detect the first `.nautilus` file in the current directory, or return
/// `schema` as-is if explicitly provided.
pub fn resolve_schema_path(schema: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = schema {
        return Ok(path);
    }

    let nautilus_files = nautilus_schema::discover_schema_paths_in_current_dir()
        .context("Failed to inspect current directory for .nautilus schema files")?;

    if nautilus_files.is_empty() {
        return Err(anyhow::anyhow!(
            "No .nautilus schema file found in current directory.\n\n\
            Hint: Create a schema file (e.g. 'schema.nautilus') or specify the path:\n\
            nautilus generate --schema path/to/schema.nautilus"
        ));
    }

    let schema_file = &nautilus_files[0];

    if nautilus_files.len() > 1 {
        report::warning(&format!(
            "multiple .nautilus files found, using: {}",
            schema_file.display()
        ));
    }

    Ok(schema_file.clone())
}

pub fn parse_schema(source: &str) -> Result<nautilus_schema::ast::Schema> {
    parse_schema_source(source).map_err(|e| anyhow::anyhow!("{}", e))
}

/// Validate `schema` and prune the IR down to what a client can express.
pub(crate) fn generation_ir(schema: &SchemaSet, verbose: bool) -> Result<SchemaIr> {
    let validated = schema
        .validate()
        .map_err(|e| anyhow::anyhow!("Validation failed:\n{}", schema.format_error(&e)))?;
    let nautilus_schema::ValidatedSchema { ast, ir } = validated;

    if verbose {
        println!("parsed {} declarations", ast.declarations.len());
    }

    validate_ir_references(&ir)?;

    // Generation runs on the pruned schema: an `@@ignore`d model or `@ignore`d
    // field names something the client has no faithful type for and no way to
    // write, and the join table of an implicit many-to-many is reached through
    // the two relation fields rather than as a model of its own.
    let ir = ir.without_ignored().without_join_tables();

    if verbose {
        println!("{:#?}", ir);
    }

    Ok(ir)
}

/// Verify that all type references in the IR resolve to known definitions.
///
/// The schema validator already checks these, but this acts as a defense-in-depth
/// guard so codegen never silently produces broken output from a malformed IR.
fn validate_ir_references(ir: &SchemaIr) -> Result<()> {
    for (model_name, model) in &ir.models {
        for field in &model.fields {
            match &field.field_type {
                ResolvedFieldType::Enum { enum_name, .. } => {
                    if !ir.enums.contains_key(enum_name) {
                        return Err(anyhow::anyhow!(
                            "Model '{}' field '{}' references unknown enum '{}'",
                            model_name,
                            field.logical_name,
                            enum_name
                        ));
                    }
                }
                ResolvedFieldType::Relation(rel) => {
                    if !ir.models.contains_key(&rel.target_model) {
                        return Err(anyhow::anyhow!(
                            "Model '{}' field '{}' references unknown model '{}'",
                            model_name,
                            field.logical_name,
                            rel.target_model
                        ));
                    }
                }
                ResolvedFieldType::CompositeType { type_name, .. } => {
                    if !ir.composite_types.contains_key(type_name) {
                        return Err(anyhow::anyhow!(
                            "Model '{}' field '{}' references unknown composite type '{}'",
                            model_name,
                            field.logical_name,
                            type_name
                        ));
                    }
                }
                ResolvedFieldType::Scalar(_) => {}
            }
        }
    }
    Ok(())
}
