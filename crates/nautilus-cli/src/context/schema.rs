//! Finding and reading the schema a command works on.
//!
//! The path comes from the flag or from the current directory, the `.env`
//! beside it is loaded before any URL is read, and the file — or the directory
//! of files — is parsed and validated into the IR every command starts from.

use anyhow::Context;
use nautilus_schema::{discover_schema_paths_in_current_dir, ir::SchemaIr, SchemaSet};
use std::path::{Path, PathBuf};

use crate::tui;

/// Locate the `.nautilus` schema file.
///
/// Priority: explicit `--schema` argument -> first `.nautilus` file in the
/// current directory. Returns an error if neither is available.
pub fn resolve_schema_path(schema_arg: Option<String>) -> anyhow::Result<PathBuf> {
    maybe_resolve_schema_path(schema_arg.as_deref())?.context(
        "Schema file not found. Pass --schema <path> or create a .nautilus \
         file in the current directory.",
    )
}

/// Resolve the schema path when it is optional for the caller.
pub(crate) fn maybe_resolve_schema_path(
    schema_arg: Option<&str>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = schema_arg {
        return Ok(Some(PathBuf::from(path)));
    }

    let nautilus_files = discover_schema_paths_in_current_dir()
        .context("Failed to inspect current directory for .nautilus schema files")?;
    let schema_path = nautilus_files.first().cloned();

    if let Some(path) = &schema_path {
        if nautilus_files.len() > 1 {
            tui::eprint_warning(&format!(
                "multiple .nautilus files found, using: {}",
                path.display()
            ));
        }
    }

    Ok(schema_path)
}

/// Lex, parse, and validate a schema, returning the [`SchemaIr`].
///
/// `path` may name a single `.nautilus` file or a directory holding several, in
/// which case they are assembled into one schema.
pub fn parse_and_validate_schema(path: &std::path::Path) -> anyhow::Result<SchemaIr> {
    let set = SchemaSet::load_path(path)
        .with_context(|| format!("Cannot read schema: {}", path.display()))?;

    set.validate()
        .map(|validated| validated.ir)
        .map_err(|e| anyhow::anyhow!("{}", set.format_error(&e)))
}

/// Load a `.env` file and inject its entries into the process environment.
///
/// Search order (first file found wins):
///   1. Directory containing the schema file.
///   2. Current working directory.
///
/// Already-set variables are never overwritten (shell exports take priority).
/// Supports `KEY=VALUE` and `KEY="VALUE"` / `KEY='VALUE'`; `#` comments; blank
/// lines. No variable-expansion is performed.
pub(crate) fn load_dotenv_for_schema(schema_path: &Path) {
    let schema_dir = if schema_path.is_dir() {
        schema_path.to_path_buf()
    } else {
        schema_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let search_dirs = [
        schema_dir,
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    ];

    for dir in search_dirs {
        let candidate = dir.join(".env");
        if !candidate.is_file() {
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(&candidate) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let mut value = value.trim();
                    if value.len() >= 2 {
                        let (first, last) =
                            (value.as_bytes()[0], value.as_bytes()[value.len() - 1]);
                        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                            value = &value[1..value.len() - 1];
                        }
                    }
                    if !key.is_empty() && std::env::var(key).is_err() {
                        // SAFETY: single-threaded context (before async spawn)
                        #[allow(clippy::disallowed_methods)]
                        std::env::set_var(key, value);
                    }
                }
            }
        }
        return; // first file found wins
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_schema_path;
    use crate::test_support::{lock_working_dir, CurrentDirGuard};
    use tempfile::TempDir;

    #[test]
    fn resolve_schema_path_auto_detects_first_nautilus_file() {
        let _cwd_lock = lock_working_dir();
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());

        std::fs::write(
            temp_dir.path().join("custom.nautilus"),
            "model User { id Int @id }\n",
        )
        .expect("failed to write custom schema");
        std::fs::write(
            temp_dir.path().join("alpha.nautilus"),
            "model Post { id Int @id }\n",
        )
        .expect("failed to write alpha schema");

        let resolved = resolve_schema_path(None).expect("schema should auto-resolve");
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("alpha.nautilus")
        );
    }

    #[test]
    fn resolve_schema_path_errors_when_no_nautilus_files_exist() {
        let _cwd_lock = lock_working_dir();
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());

        let err = resolve_schema_path(None).expect_err("missing schema should fail");
        assert!(err
            .to_string()
            .contains("Schema file not found. Pass --schema <path> or create a .nautilus file"));
    }
}
