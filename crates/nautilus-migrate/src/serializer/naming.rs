//! Logical names for an introspected schema.
//!
//! A live database has table and column names; a schema has model and field
//! names. This module owns that translation and the collision handling it
//! needs, so a name is chosen once and every renderer reads it back from the
//! same [`TableNamingContext`].

use std::collections::{HashMap, HashSet};

use super::PullNameCase;
use super::PullNamingOptions;
use crate::live::LiveSchema;
use nautilus_core::TableName;
use nautilus_schema::TokenKind;

#[derive(Debug, Clone)]
pub(super) struct TableNamingContext {
    pub(super) model_name: String,
    pub(super) db_to_logical_field: HashMap<String, String>,
    pub(super) logical_field_order: Vec<String>,
}

pub(super) fn build_table_naming_contexts(
    live: &LiveSchema,
    table_names: &[&TableName],
    view_names: &[&TableName],
    options: PullNamingOptions,
) -> HashMap<TableName, TableNamingContext> {
    let mut contexts = HashMap::new();
    let mut used_model_names = HashSet::new();

    for &table_name in table_names.iter().chain(view_names.iter()) {
        let table = live
            .tables
            .get(table_name)
            .unwrap_or_else(|| &live.views[table_name]);
        let model_name = choose_unique_field_name(
            vec![apply_model_case(&table_name.name, options.model_case)],
            &mut used_model_names,
        );

        let mut used_field_names = HashSet::new();
        let mut db_to_logical_field = HashMap::new();
        let mut logical_field_order = Vec::new();

        for column in &table.columns {
            let logical_name = choose_unique_field_name(
                vec![apply_scalar_field_case(&column.name, options.field_case)],
                &mut used_field_names,
            );
            db_to_logical_field.insert(column.name.clone(), logical_name.clone());
            logical_field_order.push(logical_name);
        }

        contexts.insert(
            table_name.clone(),
            TableNamingContext {
                model_name,
                db_to_logical_field,
                logical_field_order,
            },
        );
    }

    contexts
}

/// Infer a logical relation field name from FK columns and the referenced table.
///
/// Examples:
/// - columns = `["user_id"]`  -> `"user"`   (strip `_id` suffix)
/// - columns = `["author_id"]` -> `"author"`
/// - columns = `["a_id", "b_id"]` -> singular form of `referenced_table`
fn infer_relation_field_name(fk_cols: &[String], ref_table: &str) -> String {
    if fk_cols.len() == 1 {
        let col = &fk_cols[0];
        if let Some(name) = col.strip_suffix("_id") {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    singular_name(ref_table)
}

pub(super) fn relation_field_name_base(fk_cols: &[String], ref_table: &str) -> String {
    let raw = infer_relation_field_name(fk_cols, ref_table);
    let normalized = to_snake_case_identifier(&raw);
    if normalized.is_empty() {
        "relation".to_string()
    } else {
        normalized
    }
}

pub(super) fn default_back_relation_field_name(
    owning_table: &str,
    is_one_to_one: bool,
    options: PullNamingOptions,
) -> String {
    let singular = to_snake_case_identifier(&singular_name(owning_table));
    if is_one_to_one {
        apply_derived_field_case(&singular, options.field_case)
    } else {
        apply_derived_field_case(&pluralize_name(&singular), options.field_case)
    }
}

pub(super) fn qualify_back_relation_field_name(
    default_name: &str,
    forward_field_name: &str,
    options: PullNamingOptions,
) -> String {
    apply_derived_field_case(
        &format!(
            "{}_{}",
            to_snake_case_identifier(default_name),
            to_snake_case_identifier(forward_field_name)
        ),
        options.field_case,
    )
}

pub(super) fn choose_unique_field_name(
    candidates: Vec<String>,
    used_fields: &mut HashSet<String>,
) -> String {
    let mut first_candidate = None;
    for candidate in candidates {
        let candidate = sanitize_logical_identifier(&candidate);
        if candidate.is_empty() {
            continue;
        }
        if first_candidate.is_none() {
            first_candidate = Some(candidate.clone());
        }
        if used_fields.insert(candidate.clone()) {
            return candidate;
        }
    }

    let base = first_candidate.unwrap_or_else(|| "relation".to_string());
    let mut suffix = 2usize;
    loop {
        let candidate = sanitize_logical_identifier(&format!("{}_{}", base, suffix));
        if used_fields.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

pub(super) fn logical_field_name(naming: &TableNamingContext, db_column_name: &str) -> String {
    naming
        .db_to_logical_field
        .get(db_column_name)
        .cloned()
        .unwrap_or_else(|| db_column_name.to_string())
}

pub(super) fn join_logical_fields(naming: &TableNamingContext, columns: &[String]) -> String {
    columns
        .iter()
        .map(|column| logical_field_name(naming, column))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn default_index_name(table_name: &str, columns: &[String]) -> String {
    let mut sorted_columns = columns.to_vec();
    sorted_columns.sort();
    format!("idx_{}_{}", table_name, sorted_columns.join("_"))
}

fn sanitize_logical_identifier(name: &str) -> String {
    let mut candidate = name
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if candidate.is_empty() {
        candidate.push('_');
    }

    if candidate
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_alphabetic() && ch != '_')
    {
        candidate.insert(0, '_');
    }

    if TokenKind::from_ident(&candidate).is_keyword() {
        candidate.push('_');
    }

    candidate
}

fn apply_model_case(name: &str, case: PullNameCase) -> String {
    match case {
        PullNameCase::Auto => to_pascal_case(name),
        PullNameCase::Snake => to_snake_case_identifier(name),
        PullNameCase::Pascal => normalized_pascal_case(name),
    }
}

pub(super) fn apply_scalar_field_case(name: &str, case: PullNameCase) -> String {
    match case {
        PullNameCase::Auto => name.to_string(),
        PullNameCase::Snake => to_snake_case_identifier(name),
        PullNameCase::Pascal => normalized_pascal_case(name),
    }
}

pub(super) fn apply_derived_field_case(name: &str, case: PullNameCase) -> String {
    match case {
        PullNameCase::Auto | PullNameCase::Snake => to_snake_case_identifier(name),
        PullNameCase::Pascal => normalized_pascal_case(name),
    }
}

fn normalized_pascal_case(name: &str) -> String {
    let snake = to_snake_case_identifier(name);
    if snake.is_empty() {
        String::new()
    } else {
        to_pascal_case(&snake)
    }
}

pub(super) fn pluralize_name(name: &str) -> String {
    if name.ends_with('y')
        && !matches!(name.chars().rev().nth(1), Some('a' | 'e' | 'i' | 'o' | 'u'))
    {
        format!("{}ies", &name[..name.len() - 1])
    } else if matches!(name.chars().last(), Some('s' | 'x' | 'z'))
        || name.ends_with("ch")
        || name.ends_with("sh")
    {
        format!("{name}es")
    } else {
        format!("{name}s")
    }
}

/// Very simple singularisation: strip a trailing `s` (handles the common
/// plural pattern; no full inflection library is needed here).
pub(super) fn singular_name(name: &str) -> String {
    if name.ends_with("ies") && name.len() > 3 {
        format!("{}y", &name[..name.len() - 3])
    } else if name.ends_with('s') && name.len() > 1 {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

pub(super) fn to_snake_case_identifier(s: &str) -> String {
    let chars: Vec<char> = s
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    let mut out = String::new();

    for (idx, ch) in chars.iter().copied().enumerate() {
        if ch == '_' {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            continue;
        }

        let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(idx + 1).copied();
        let is_upper = ch.is_ascii_uppercase();
        let prev_is_lower_or_digit =
            prev.is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit());
        let prev_is_upper = prev.is_some_and(|prev| prev.is_ascii_uppercase());
        let next_is_lower = next.is_some_and(|next| next.is_ascii_lowercase());

        if is_upper
            && !out.is_empty()
            && (prev_is_lower_or_digit || (prev_is_upper && next_is_lower))
            && !out.ends_with('_')
        {
            out.push('_');
        }

        out.push(ch.to_ascii_lowercase());
    }

    out.trim_matches('_').to_string()
}

/// Convert a snake_case table name to PascalCase (for example `blog_posts` -> `BlogPosts`).
pub(super) fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::live::{LiveColumn, LiveTable};

    fn column(name: &str) -> LiveColumn {
        LiveColumn {
            name: name.to_string(),
            col_type: "text".into(),
            nullable: false,
            default_value: None,
            generated_expr: None,
            computed_kind: None,
            check_expr: None,
            auto_increment: false,
            self_updating: false,
        }
    }

    fn live_with(table: &str, columns: &[&str]) -> LiveSchema {
        let name = TableName::new(table);
        let mut live = LiveSchema::default();
        live.tables.insert(
            name.clone(),
            LiveTable {
                name,
                columns: columns.iter().map(|c| column(c)).collect(),
                primary_key: Vec::new(),
                indexes: Vec::new(),
                check_constraints: Vec::new(),
                foreign_keys: Vec::new(),
            },
        );
        live
    }

    fn context_for(
        live: &LiveSchema,
        table: &str,
        options: PullNamingOptions,
    ) -> TableNamingContext {
        let name = TableName::new(table);
        build_table_naming_contexts(live, &[&name], &[], options)
            .remove(&name)
            .expect("a context for the table")
    }

    #[test]
    fn naming_options_choose_model_and_field_spelling() {
        let live = live_with("blog_posts", &["id", "author_id"]);

        let auto = context_for(&live, "blog_posts", PullNamingOptions::default());
        assert_eq!(auto.model_name, "BlogPosts");
        assert_eq!(auto.db_to_logical_field["author_id"], "author_id");

        let pascal = context_for(
            &live,
            "blog_posts",
            PullNamingOptions {
                model_case: PullNameCase::Pascal,
                field_case: PullNameCase::Pascal,
            },
        );
        assert_eq!(pascal.model_name, "BlogPosts");
        assert_eq!(pascal.db_to_logical_field["author_id"], "AuthorId");

        let snake = context_for(
            &live,
            "blog_posts",
            PullNamingOptions {
                model_case: PullNameCase::Snake,
                field_case: PullNameCase::Snake,
            },
        );
        assert_eq!(snake.model_name, "blog_posts");
        assert_eq!(snake.db_to_logical_field["author_id"], "author_id");
    }

    #[test]
    fn colliding_columns_get_distinct_logical_fields() {
        let live = live_with("t", &["user id", "user_id"]);
        let context = context_for(&live, "t", PullNamingOptions::default());

        let first = &context.db_to_logical_field["user id"];
        let second = &context.db_to_logical_field["user_id"];
        assert_ne!(first, second);
        assert_eq!(
            context.logical_field_order,
            vec![first.clone(), second.clone()]
        );
    }

    #[test]
    fn pascal_case_snake() {
        assert_eq!(to_pascal_case("blog_posts"), "BlogPosts");
    }

    #[test]
    fn pascal_case_single() {
        assert_eq!(to_pascal_case("users"), "Users");
    }

    #[test]
    fn pascal_case_already() {
        assert_eq!(to_pascal_case("User"), "User");
    }
}
