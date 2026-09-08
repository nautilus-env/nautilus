//! The validated AST turned into the IR the rest of Nautilus reads.
//!
//! This file assembles the whole schema; each part of it is built by the
//! module that owns that concept — [`config`] for the configuration blocks,
//! [`entities`] for the declarations, [`fields`] for what a model holds.

mod config;
mod entities;
mod fields;

use crate::validator::*;

impl SchemaValidator<'_> {
    /// Build the IR from the validated AST.
    pub(in crate::validator) fn build_ir(self) -> Result<SchemaIr> {
        let mut ir = SchemaIr::new();

        if let Some(datasource) = self.schema.datasource() {
            ir.datasource = Some(self.build_datasource_ir(datasource)?);
        }

        if let Some(generator) = self.schema.generator() {
            ir.generator = Some(self.build_generator_ir(generator)?);
        }

        for enum_decl in self.schema.enums() {
            let enum_ir = self.build_enum_ir(enum_decl);
            ir.enums.insert(enum_ir.logical_name.clone(), enum_ir);
        }

        for type_decl in self.schema.types() {
            let composite_ir = self.build_composite_type_ir(type_decl)?;
            ir.composite_types
                .insert(composite_ir.logical_name.clone(), composite_ir);
        }

        for model in self.schema.models() {
            let model_ir = self.build_model_ir(model)?;
            ir.models.insert(model_ir.logical_name.clone(), model_ir);
        }

        crate::validator::many_to_many::link_implicit_many_to_many(&mut ir)?;

        Ok(ir)
    }
}
