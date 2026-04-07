use nautilus_schema::{ast::Schema, parser::Parser, Lexer, Result, Token, TokenKind};

pub fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let is_eof = matches!(token.kind, TokenKind::Eof);
        tokens.push(token);
        if is_eof {
            break;
        }
    }
    Ok(tokens)
}

pub fn parse_schema(source: &str) -> Result<Schema> {
    let tokens = tokenize(source)?;
    Parser::new(&tokens, source).parse_schema()
}
