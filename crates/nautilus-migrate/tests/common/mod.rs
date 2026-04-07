#![allow(dead_code)]

use nautilus_migrate::live::{LiveSchema, LiveTable};
use nautilus_migrate::{MigrationError, Result};
use nautilus_schema::{validate_schema, Lexer, Parser};

/// Parse a `.nautilus` source string into a validated [`SchemaIr`].
pub fn parse(source: &str) -> Result<nautilus_schema::ir::SchemaIr> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token().map_err(MigrationError::Schema)?;
        let is_eof = matches!(token.kind, nautilus_schema::TokenKind::Eof);
        tokens.push(token);
        if is_eof {
            break;
        }
    }
    let ast = Parser::new(&tokens, source)
        .parse_schema()
        .map_err(MigrationError::Schema)?;
    validate_schema(ast).map_err(MigrationError::Schema)
}

/// Build a [`LiveSchema`] from a list of [`LiveTable`] values.
pub fn make_live_schema(tables: Vec<LiveTable>) -> LiveSchema {
    let mut ls = LiveSchema::default();
    for t in tables {
        ls.tables.insert(t.name.clone(), t);
    }
    ls
}
