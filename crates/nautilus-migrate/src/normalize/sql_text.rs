//! Rewrites at the level of SQL text.
//!
//! Shared by the provider rules, which read an expression out of a database,
//! and the comparison rules, which reduce two expressions to a common form.

use crate::utils::strip_outer_parens;

/// Remove every `::typename` cast suffix.
///
/// PostgreSQL adds them to defaults, CHECK constraints and generated
/// expressions alike; a quoted type name (`::"MyEnum"`) is consumed whole.
pub(crate) fn strip_casts(s: &str) -> String {
    let mut result = s.to_string();
    while let Some(idx) = result.rfind("::") {
        let after = &result[idx + 2..];
        let type_end = if let Some(after_quote) = after.strip_prefix('"') {
            after_quote.find('"').map(|i| i + 2).unwrap_or(after.len())
        } else {
            after
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != ' ')
                .unwrap_or(after.len())
        };
        result = format!("{}{}", &result[..idx], &result[idx + 2 + type_end..]);
    }
    result
}

/// Remove parentheses that wrap a single numeric literal.
pub(crate) fn strip_numeric_paren_literals(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '(' {
            let mut inner = String::new();
            let mut depth = 1i32;
            for c in chars.by_ref() {
                match c {
                    '(' => {
                        depth += 1;
                        inner.push(c);
                    }
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        inner.push(c);
                    }
                    _ => inner.push(c),
                }
            }
            let trimmed = inner.trim();
            let is_numeric = !trimmed.is_empty()
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-');
            if is_numeric {
                result.push_str(trimmed);
            } else {
                result.push('(');
                result.push_str(&strip_numeric_paren_literals(&inner));
                result.push(')');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Convert `col = ANY (ARRAY['A', 'B'])` into the Nautilus bracket form.
pub(crate) fn convert_any_array_to_in(s: &str) -> String {
    let lower = s.to_lowercase();
    let marker = "= any (array[";

    let Some(eq_pos) = lower.find(marker) else {
        return s.to_string();
    };

    let field = s[..eq_pos].trim();
    let bracket_start = eq_pos + marker.len();
    let rest = &s[bracket_start..];

    let mut depth = 1i32;
    let mut bracket_end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    bracket_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let Some(bclose) = bracket_end else {
        return s.to_string();
    };

    let items = &rest[..bclose];
    let after_array = rest[bclose + 1..].trim_start();
    let after_paren = after_array.strip_prefix(')').unwrap_or(after_array);

    if after_paren.is_empty() {
        format!("{} IN [{}]", field, items)
    } else {
        format!("{} IN [{}] {}", field, items, after_paren.trim_start())
    }
}

/// Convert all SQL `col IN (...)` occurrences into Nautilus `col IN [...]`.
pub(crate) fn convert_in_parens_to_brackets(s: &str) -> String {
    let lower = s.to_lowercase();
    let marker = " in (";

    if !lower.contains(marker) {
        return s.to_string();
    }

    let mut result = String::with_capacity(s.len());
    let mut pos = 0usize;

    while let Some(rel) = lower[pos..].find(marker) {
        let abs = pos + rel;
        result.push_str(&s[pos..abs]);
        result.push_str(" IN [");

        let after_open = abs + marker.len();
        let rest = &s[after_open..];

        let mut depth = 1i32;
        let mut close = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        result.push_str(&rest[..close]);
        result.push(']');
        pos = after_open + close + 1;
    }

    result.push_str(&s[pos..]);
    result
}

/// Strip every balanced layer of outer parentheses, not just the outermost one.
pub(crate) fn strip_all_outer_parens(s: &str) -> String {
    let mut s = s.to_string();
    loop {
        let stripped = strip_outer_parens(&s);
        if stripped == s {
            return s;
        }
        s = stripped;
    }
}

/// Collapse every run of whitespace to a single space, and trim the ends.
pub(crate) fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
