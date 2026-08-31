//! Filesystem-backed completion for schema import paths.

use std::path::{Path, PathBuf};

use nautilus_schema::LineIndex;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Range, TextEdit,
};

use crate::convert::offset_to_position_with_index;

struct ImportPathContext<'a> {
    typed: &'a str,
    content_start: usize,
    content_end: usize,
}

/// Returns directory and `.nautilus` file completions when `offset` is inside
/// the quoted path of an `import` declaration.
pub fn import_path_completions(
    source: &str,
    line_index: &LineIndex,
    offset: usize,
    document_path: &Path,
) -> Option<Vec<CompletionItem>> {
    let context = import_path_context(source, offset)?;
    let (directory_prefix, partial) = split_path_prefix(context.typed);
    let directory = completion_directory(document_path, directory_prefix)?;
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return Some(Vec::new()),
    };
    let range = Range {
        start: offset_to_position_with_index(source, line_index, context.content_start),
        end: offset_to_position_with_index(source, line_index, context.content_end),
    };
    let partial_lower = partial.to_lowercase();
    let separator = path_separator(directory_prefix);
    let mut items = Vec::new();

    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !name.to_lowercase().starts_with(&partial_lower) {
            continue;
        }

        let path = entry.path();
        let is_directory = path.is_dir();
        let is_schema = path.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("nautilus");
        if !is_directory && !is_schema {
            continue;
        }

        let suffix = if is_directory {
            separator.to_string()
        } else {
            String::new()
        };
        let label = format!("{name}{suffix}");
        let new_text = format!("{directory_prefix}{name}{suffix}");
        items.push(CompletionItem {
            label: label.clone(),
            kind: Some(if is_directory {
                CompletionItemKind::FOLDER
            } else {
                CompletionItemKind::FILE
            }),
            detail: Some(if is_directory {
                "Directory".to_string()
            } else {
                "Nautilus schema".to_string()
            }),
            filter_text: Some(new_text.clone()),
            sort_text: Some(format!(
                "{}-{}",
                if is_directory { '0' } else { '1' },
                name.to_lowercase()
            )),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
            ..Default::default()
        });
    }

    items.sort_by(|left, right| left.sort_text.cmp(&right.sort_text));
    Some(items)
}

fn import_path_context(source: &str, offset: usize) -> Option<ImportPathContext<'_>> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }

    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let before_cursor = &source[line_start..offset];
    let indentation = before_cursor.len() - before_cursor.trim_start_matches([' ', '\t']).len();
    let declaration = &before_cursor[indentation..];
    let after_keyword = declaration.strip_prefix("import")?;
    let first = after_keyword.chars().next()?;
    if !first.is_whitespace() {
        return None;
    }

    let whitespace = after_keyword.len() - after_keyword.trim_start_matches([' ', '\t']).len();
    let after_whitespace = &after_keyword[whitespace..];
    let typed = after_whitespace.strip_prefix('"')?;
    if typed.contains('"') {
        return None;
    }

    let content_start = offset - typed.len();
    let line_end = source[offset..]
        .find('\n')
        .map_or(source.len(), |relative| offset + relative);
    let content_end = source[offset..line_end]
        .find('"')
        .map_or(offset, |relative| offset + relative);

    Some(ImportPathContext {
        typed,
        content_start,
        content_end,
    })
}

fn split_path_prefix(path: &str) -> (&str, &str) {
    match path
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '/' | '\\'))
    {
        Some((index, character)) => path.split_at(index + character.len_utf8()),
        None => ("", path),
    }
}

fn completion_directory(document_path: &Path, prefix: &str) -> Option<PathBuf> {
    let parent = document_path.parent()?;
    let prefix_path = Path::new(prefix);
    Some(if prefix_path.is_absolute() {
        prefix_path.to_path_buf()
    } else {
        parent.join(prefix_path)
    })
}

fn path_separator(prefix: &str) -> char {
    if prefix.ends_with('\\') {
        '\\'
    } else {
        '/'
    }
}

#[cfg(test)]
mod tests {
    use super::import_path_completions;
    use nautilus_schema::LineIndex;
    use tower_lsp::lsp_types::{CompletionItemKind, CompletionTextEdit};

    fn schema_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nautilus-lsp-import-completion-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn import_completion_lists_directories_and_schema_files_only() {
        let dir = schema_dir("root");
        std::fs::create_dir(dir.join("domain")).expect("create domain");
        std::fs::create_dir(dir.join("empty")).expect("create empty");
        std::fs::write(dir.join("models.nautilus"), "").expect("write schema");
        std::fs::write(dir.join("notes.txt"), "").expect("write notes");
        let document = dir.join("schema.nautilus");
        let source = "import \"\"";
        let items = import_path_completions(
            source,
            &LineIndex::new(source),
            "import \"".len(),
            &document,
        )
        .expect("import path context");

        let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, vec!["domain/", "empty/", "models.nautilus"]);
        assert_eq!(items[0].kind, Some(CompletionItemKind::FOLDER));
        assert_eq!(items[2].kind, Some(CompletionItemKind::FILE));
    }

    #[test]
    fn nested_completion_replaces_the_quoted_path_and_filters_the_leaf() {
        let dir = schema_dir("nested");
        let domain = dir.join("domain");
        std::fs::create_dir(&domain).expect("create domain");
        std::fs::write(domain.join("post.nautilus"), "").expect("write post schema");
        std::fs::write(domain.join("user.nautilus"), "").expect("write user schema");
        let document = dir.join("schema.nautilus");
        let source = "import \"./domain/po\"";
        let offset = source.find("po\"").expect("partial path") + 2;
        let items = import_path_completions(source, &LineIndex::new(source), offset, &document)
            .expect("import path context");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "post.nautilus");
        assert_eq!(
            items[0].filter_text.as_deref(),
            Some("./domain/post.nautilus")
        );
        let Some(CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
            panic!("expected text edit");
        };
        assert_eq!(edit.new_text, "./domain/post.nautilus");
        assert_eq!(edit.range.start.character, 8);
        assert_eq!(edit.range.end.character, 19);
    }

    #[test]
    fn completion_outside_an_import_path_is_not_handled() {
        let dir = schema_dir("outside");
        let source = "model User {}";
        assert!(import_path_completions(
            source,
            &LineIndex::new(source),
            source.len(),
            &dir.join("schema.nautilus")
        )
        .is_none());
    }
}
