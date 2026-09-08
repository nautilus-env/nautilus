//! The configuration blocks of a schema.
//!
//! A schema declares at most one `datasource` and one `generator`; each is
//! validated by its own module, and the rule that refuses the extra blocks is
//! shared by both.

mod datasource;
mod generator;

use crate::validator::*;

impl SchemaValidator<'_> {
    fn reject_extra_blocks(&mut self, kind: &str, blocks: &[(String, Span)]) {
        let Some((first, _)) = blocks.first() else {
            return;
        };

        for (name, span) in &blocks[1..] {
            self.errors.push_back(SchemaError::Validation(
                format!(
                    "Duplicate {} '{}': a schema has exactly one {} block, already declared as '{}'",
                    kind, name, kind, first
                ),
                *span,
            ));
        }
    }
}
