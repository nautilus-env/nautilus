//! Finding and reading the schema a command works on.
//!
//! The path comes from the flag or from the current directory, the `.env`
//! beside it is loaded before any URL is read, and the file — or the directory
//! of files — is parsed and validated into the IR every command starts from.

use anyhow::Context;
use nautilus_schema::{discover_schema_paths, ir::SchemaIr, SchemaSet};
use std::path::{Path, PathBuf};

use crate::context::environment::CommandEnv;
use crate::tui;

/// Locate the `.nautilus` schema file.
///
/// Priority: explicit `--schema` argument -> first `.nautilus` file in the
/// directory the command was invoked from. Returns an error if neither is
/// available.
pub fn resolve_schema_path(
    schema_arg: Option<String>,
    env: &CommandEnv,
) -> anyhow::Result<PathBuf> {
    maybe_resolve_schema_path(schema_arg.as_deref(), env)?.context(
        "Schema file not found. Pass --schema <path> or create a .nautilus \
         file in the current directory.",
    )
}

/// Resolve the schema path when it is optional for the caller.
pub(crate) fn maybe_resolve_schema_path(
    schema_arg: Option<&str>,
    env: &CommandEnv,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = schema_arg {
        return Ok(Some(PathBuf::from(path)));
    }

    let nautilus_files = discover_schema_paths(env.current_dir())
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

/// Load a `.env` file and make its entries visible to the command.
///
/// Search order (first file found wins):
///   1. Directory containing the schema file.
///   2. The directory the command was invoked from.
///
/// Already-set variables are never overwritten (shell exports take priority).
pub(crate) fn load_dotenv_for_schema(schema_path: &Path, env: &mut CommandEnv) {
    let Some(path) = find_dotenv(schema_path, env.current_dir()) else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };

    for (key, value) in parse_dotenv(&contents) {
        env.set_var_if_absent(key, value);
    }
}

/// The `.env` to read: the one beside the schema, else the one where the
/// command was invoked.
fn find_dotenv(schema_path: &Path, current_dir: &Path) -> Option<PathBuf> {
    let schema_dir = if schema_path.is_dir() {
        schema_path.to_path_buf()
    } else {
        schema_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };

    [schema_dir, current_dir.to_path_buf()]
        .into_iter()
        .map(|dir| dir.join(".env"))
        .find(|candidate| candidate.is_file())
}

/// The `KEY=VALUE` pairs a `.env` file declares.
///
/// Supports `KEY=VALUE` and `KEY="VALUE"` / `KEY='VALUE'`; `#` comments; blank
/// lines. No variable-expansion is performed.
fn parse_dotenv(contents: &str) -> Vec<(&str, &str)> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), unquote(value.trim())))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

/// Strip one matching pair of surrounding quotes, if there is one.
fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{load_dotenv_for_schema, parse_dotenv, resolve_schema_path};
    use crate::context::environment::CommandEnv;
    use tempfile::TempDir;

    #[test]
    fn resolve_schema_path_auto_detects_first_nautilus_file() {
        let temp_dir = TempDir::new().expect("temp dir");
        let env = CommandEnv::fixed(temp_dir.path());

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

        let resolved = resolve_schema_path(None, &env).expect("schema should auto-resolve");
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("alpha.nautilus")
        );
    }

    #[test]
    fn resolve_schema_path_errors_when_no_nautilus_files_exist() {
        let temp_dir = TempDir::new().expect("temp dir");
        let env = CommandEnv::fixed(temp_dir.path());

        let err = resolve_schema_path(None, &env).expect_err("missing schema should fail");
        assert!(err
            .to_string()
            .contains("Schema file not found. Pass --schema <path> or create a .nautilus file"));
    }

    #[test]
    fn parse_dotenv_reads_quoted_values_and_skips_comments() {
        let entries = parse_dotenv(
            "# comment\n\nPLAIN=one\nDOUBLE=\"two\"\nSINGLE='three'\n  SPACED = four \nEMPTY=\n=novalue\nNO_EQUALS\n",
        );

        assert_eq!(
            entries,
            vec![
                ("PLAIN", "one"),
                ("DOUBLE", "two"),
                ("SINGLE", "three"),
                ("SPACED", "four"),
                ("EMPTY", ""),
            ]
        );
    }

    #[test]
    fn load_dotenv_prefers_the_schema_directory_over_the_working_directory() {
        let schema_dir = TempDir::new().expect("schema temp dir");
        let working_dir = TempDir::new().expect("working temp dir");
        let schema_path = schema_dir.path().join("schema.nautilus");
        std::fs::write(&schema_path, "model User { id Int @id }\n")
            .expect("failed to write schema");
        std::fs::write(schema_dir.path().join(".env"), "NAUTILUS_TEST_URL=beside\n")
            .expect("failed to write schema dotenv");
        std::fs::write(working_dir.path().join(".env"), "NAUTILUS_TEST_URL=cwd\n")
            .expect("failed to write cwd dotenv");

        let mut env = CommandEnv::fixed(working_dir.path());
        load_dotenv_for_schema(&schema_path, &mut env);

        assert_eq!(env.var("NAUTILUS_TEST_URL").as_deref(), Some("beside"));
    }

    #[test]
    fn load_dotenv_falls_back_to_the_working_directory() {
        let schema_dir = TempDir::new().expect("schema temp dir");
        let working_dir = TempDir::new().expect("working temp dir");
        let schema_path = schema_dir.path().join("schema.nautilus");
        std::fs::write(&schema_path, "model User { id Int @id }\n")
            .expect("failed to write schema");
        std::fs::write(working_dir.path().join(".env"), "NAUTILUS_TEST_URL=cwd\n")
            .expect("failed to write cwd dotenv");

        let mut env = CommandEnv::fixed(working_dir.path());
        load_dotenv_for_schema(&schema_path, &mut env);

        assert_eq!(env.var("NAUTILUS_TEST_URL").as_deref(), Some("cwd"));
    }

    #[test]
    fn load_dotenv_never_replaces_an_existing_value() {
        let working_dir = TempDir::new().expect("working temp dir");
        let schema_path = working_dir.path().join("schema.nautilus");
        std::fs::write(&schema_path, "model User { id Int @id }\n")
            .expect("failed to write schema");
        std::fs::write(
            working_dir.path().join(".env"),
            "NAUTILUS_TEST_URL=from-file\n",
        )
        .expect("failed to write dotenv");

        let mut env = CommandEnv::fixed(working_dir.path());
        env.set_var_if_absent("NAUTILUS_TEST_URL", "exported");
        load_dotenv_for_schema(&schema_path, &mut env);

        assert_eq!(env.var("NAUTILUS_TEST_URL").as_deref(), Some("exported"));
    }
}
