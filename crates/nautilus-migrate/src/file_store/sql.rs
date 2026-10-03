//! Statement boundaries for migration files, without parsing SQL expressions.

enum State<'a> {
    Sql,
    Quote { end: u8, backslash_escape: bool },
    LineComment,
    BlockComment(usize),
    DollarQuote(&'a str),
}

pub(super) fn split_statements(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut state = State::Sql;
    let mut statements = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    let mut has_sql = false;
    while pos < bytes.len() {
        let byte = bytes[pos];
        let next = bytes.get(pos + 1).copied();
        match state {
            State::Sql => match (byte, next) {
                (b'-', Some(b'-')) => {
                    state = State::LineComment;
                    pos += 1;
                }
                (b'/', Some(b'*')) => {
                    state = State::BlockComment(1);
                    pos += 1;
                }
                (b'\'' | b'"' | b'`' | b'[', _) => {
                    let escape_string = byte == b'\''
                        && pos > 0
                        && matches!(bytes[pos - 1], b'e' | b'E')
                        && (pos < 2 || !is_identifier_byte(bytes[pos - 2]));
                    state = State::Quote {
                        end: if byte == b'[' { b']' } else { byte },
                        backslash_escape: escape_string || byte == b'`',
                    };
                    has_sql = true;
                }
                (b'$', _) => {
                    if let Some(tag) = dollar_tag(content, pos) {
                        state = State::DollarQuote(tag);
                        pos += tag.len() - 1;
                    }
                    has_sql = true;
                }
                (b';', _) => {
                    push_statement(&mut statements, &content[start..pos]);
                    start = pos + 1;
                    has_sql = false;
                }
                (b'\n', _) if !has_sql => {
                    let tail = content[pos + 1..].trim_start_matches([' ', '\t']);
                    if tail.starts_with('\n') {
                        push_statement(&mut statements, &content[start..pos]);
                        start = pos + 1;
                    }
                }
                _ => has_sql |= !byte.is_ascii_whitespace(),
            },
            State::Quote {
                end,
                backslash_escape,
            } => {
                if backslash_escape && byte == b'\\' && next.is_some() {
                    pos += 1;
                } else if byte == end {
                    if next == Some(end) {
                        pos += 1;
                    } else {
                        state = State::Sql;
                    }
                }
            }
            State::LineComment => {
                if byte == b'\n' {
                    state = State::Sql;
                    continue;
                }
            }
            State::BlockComment(depth) => match (byte, next) {
                (b'/', Some(b'*')) => {
                    state = State::BlockComment(depth + 1);
                    pos += 1;
                }
                (b'*', Some(b'/')) => {
                    state = if depth == 1 {
                        State::Sql
                    } else {
                        State::BlockComment(depth - 1)
                    };
                    pos += 1;
                }
                _ => {}
            },
            State::DollarQuote(tag) => {
                if bytes[pos..].starts_with(tag.as_bytes()) {
                    pos += tag.len() - 1;
                    state = State::Sql;
                }
            }
        }
        pos += 1;
    }
    push_statement(&mut statements, &content[start..]);
    statements
}

fn push_statement(statements: &mut Vec<String>, content: &str) {
    let trimmed = content.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') || !byte.is_ascii()
}

fn dollar_tag(content: &str, pos: usize) -> Option<&str> {
    let bytes = content.as_bytes();
    if pos > 0 && is_identifier_byte(bytes[pos - 1]) {
        return None;
    }
    let end = pos + 1 + bytes[pos + 1..].iter().position(|byte| *byte == b'$')?;
    let tag = &bytes[pos + 1..end];
    if !tag.is_empty()
        && (!(tag[0].is_ascii_alphabetic() || tag[0] == b'_')
            || !tag
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_'))
    {
        return None;
    }
    Some(&content[pos..=end])
}

#[cfg(test)]
mod tests {
    use super::split_statements;

    #[test]
    fn blank_lines_are_allowed_inside_sql_and_statements_need_no_blank_separator() {
        assert_eq!(
            split_statements("CREATE TABLE t (\n  id INTEGER,\n\n  label TEXT\n);INSERT INTO t VALUES (1, 'a');\nSELECT 1"),
            vec!["CREATE TABLE t (\n  id INTEGER,\n\n  label TEXT\n)", "INSERT INTO t VALUES (1, 'a')", "SELECT 1"]
        );
    }

    #[test]
    fn quoted_semicolons_and_escaped_delimiters_do_not_split_statements() {
        let sql = r#"SELECT 'a;''b', "a;""b", `a;``b`, [a;b], E'a\';b', 'c\';SELECT 'é;日';"#;
        assert_eq!(
            split_statements(sql),
            vec![
                r#"SELECT 'a;''b', "a;""b", `a;``b`, [a;b], E'a\';b', 'c\'"#,
                "SELECT 'é;日'",
            ]
        );
    }

    #[test]
    fn comments_and_dollar_quoted_bodies_keep_internal_semicolons() {
        let sql = "-- leading; comment\nDO $body$ BEGIN\n\n PERFORM ';'; END $body$;\n/* outer; /* inner; */ */ SELECT $$a;b$$;-- trailing; comment";
        assert_eq!(
            split_statements(sql),
            vec![
                "-- leading; comment\nDO $body$ BEGIN\n\n PERFORM ';'; END $body$",
                "/* outer; /* inner; */ */ SELECT $$a;b$$",
                "-- trailing; comment",
            ]
        );
    }
}
