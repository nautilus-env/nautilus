//! Migration file store — reads and writes `up.sql` / `down.sql` pairs.
//!
//! Each migration lives in its own subdirectory under the migrations folder
//! (typically `migrations/` next to the schema file):
//!
//! ```text
//! migrations/
//!   {YYYYmmddHHMMSS}_{label}/
//!     up.sql
//!     down.sql
//! ```
//!
//! Lexicographic order equals chronological order because of the timestamp
//! prefix.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{MigrationError, Result};
use crate::migration::Migration;

mod sql;

/// Manages the `up.sql` / `down.sql` pair of each migration on disk.
pub struct MigrationFileStore {
    dir: PathBuf,
}

impl MigrationFileStore {
    /// Create a store pointing at `dir`.
    ///
    /// The directory is created lazily when the first migration is written.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Return the directory this store uses.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Ensure the migrations directory exists.
    pub fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir).map_err(|e| {
            MigrationError::Other(format!(
                "Cannot create migrations directory {}: {}",
                self.dir.display(),
                e
            ))
        })
    }

    /// Return all migration names in lexicographic order.
    ///
    /// Each name is a `{name}/` subdirectory of the store directory holding
    /// `up.sql` and `down.sql`.
    pub fn list_migration_names(&self) -> Result<Vec<String>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .map_err(|e| {
                MigrationError::Other(format!(
                    "Cannot read migrations directory {}: {}",
                    self.dir.display(),
                    e
                ))
            })?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if entry.file_type().ok()?.is_dir() {
                    entry.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();

        names.sort();
        Ok(names)
    }

    /// Load a migration from disk.
    ///
    /// The [`Migration::checksum`] is always recalculated from the current
    /// file content, so hand-editing a file does not break checksum
    /// verification.
    pub fn load_migration(&self, name: &str) -> Result<Migration> {
        let up_path = self.dir.join(name).join("up.sql");
        let down_path = self.dir.join(name).join("down.sql");

        let up_sql = parse_sql_file(&up_path)?;
        let down_sql = if down_path.exists() {
            parse_sql_file(&down_path)?
        } else {
            Vec::new()
        };

        Ok(Migration::new(name.to_string(), up_sql, down_sql))
    }

    /// Write a migration to disk.
    ///
    /// Creates `{name}/up.sql` and `{name}/down.sql` inside the store
    /// directory, creating the directories if they do not yet exist.  Any
    /// existing files with the same name are overwritten.
    pub fn write_migration(
        &self,
        name: &str,
        up_sql: &[String],
        down_sql: &[String],
    ) -> Result<()> {
        let migration_dir = self.dir.join(name);
        fs::create_dir_all(&migration_dir).map_err(|e| {
            MigrationError::Other(format!(
                "Cannot create migration directory {}: {}",
                migration_dir.display(),
                e
            ))
        })?;

        let up_path = migration_dir.join("up.sql");
        let down_path = migration_dir.join("down.sql");

        fs::write(&up_path, format_sql_file(up_sql)).map_err(|e| {
            MigrationError::Other(format!("Cannot write {}: {}", up_path.display(), e))
        })?;

        fs::write(&down_path, format_sql_file(down_sql)).map_err(|e| {
            MigrationError::Other(format!("Cannot write {}: {}", down_path.display(), e))
        })?;

        Ok(())
    }
}

/// Serialise SQL statements to file content.
///
/// Real SQL statements are terminated with `;`.  Comment-only statements
/// (lines that all start with `--`) are written as-is without a trailing `;`
/// so that round-tripping through `parse_sql_file` does not corrupt them.
fn format_sql_file(stmts: &[String]) -> String {
    if stmts.is_empty() {
        return "-- (empty)\n".to_string();
    }

    let mut parts: Vec<String> = stmts
        .iter()
        .map(|stmt| {
            let is_comment = stmt
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .all(|l| l.starts_with("--"));

            if is_comment {
                stmt.clone()
            } else {
                format!("{stmt};")
            }
        })
        .collect();

    parts.push(String::new());
    parts.join("\n\n")
}

/// Parse a SQL file back into individual statement strings.
///
/// Normalize line endings, then split on unquoted statement terminators.
/// Blank lines inside a statement do not split it. Standalone comment blocks
/// retain their own entries so generated migrations keep their checksums.
fn parse_sql_file(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)
        .map_err(|e| MigrationError::Other(format!("Cannot read {}: {}", path.display(), e)))?;

    let normalized = content.replace("\r\n", "\n");
    let trimmed = normalized.trim();
    if trimmed.is_empty() || trimmed == "-- (empty)" {
        return Ok(Vec::new());
    }

    Ok(sql::split_statements(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_migration() {
        let tmp = TempDir::new().unwrap();
        let store = MigrationFileStore::new(tmp.path());

        let up = vec![
            "CREATE TABLE users (id INTEGER PRIMARY KEY)".to_string(),
            "CREATE TABLE posts (id INTEGER PRIMARY KEY)".to_string(),
        ];
        let down = vec![
            "DROP TABLE IF EXISTS posts".to_string(),
            "DROP TABLE IF EXISTS users".to_string(),
        ];

        store
            .write_migration("20260227_initial", &up, &down)
            .unwrap();

        let names = store.list_migration_names().unwrap();
        assert_eq!(names, vec!["20260227_initial"]);

        let loaded = store.load_migration("20260227_initial").unwrap();
        assert_eq!(loaded.name, "20260227_initial");
        assert_eq!(loaded.up_sql, up);
        assert_eq!(loaded.down_sql, down);
    }

    #[test]
    fn empty_migration_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = MigrationFileStore::new(tmp.path());

        store.write_migration("20260227_empty", &[], &[]).unwrap();

        let loaded = store.load_migration("20260227_empty").unwrap();
        assert!(loaded.up_sql.is_empty());
        assert!(loaded.down_sql.is_empty());
    }

    #[test]
    fn list_names_sorted() {
        let tmp = TempDir::new().unwrap();
        let store = MigrationFileStore::new(tmp.path());

        store
            .write_migration("20260227_b", &["SELECT 1".to_string()], &[])
            .unwrap();
        store
            .write_migration("20260101_a", &["SELECT 1".to_string()], &[])
            .unwrap();
        store
            .write_migration("20260301_c", &["SELECT 1".to_string()], &[])
            .unwrap();

        let names = store.list_migration_names().unwrap();
        assert_eq!(names, vec!["20260101_a", "20260227_b", "20260301_c"]);
    }

    #[test]
    fn comment_only_down_sql_round_trips_without_semicolon() {
        let tmp = TempDir::new().unwrap();
        let store = MigrationFileStore::new(tmp.path());

        let up = vec!["ALTER TABLE \"test\" ADD COLUMN \"tmp\" TEXT".to_string()];
        let down = vec!["-- Cannot auto-reverse ADD COLUMN on SQLite: test.tmp".to_string()];

        store
            .write_migration("20260227_add_col", &up, &down)
            .unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join("20260227_add_col").join("down.sql")).unwrap();
        assert!(
            !content.contains("tmp;"),
            "comment should not have trailing semicolon: {content}"
        );

        let loaded = store.load_migration("20260227_add_col").unwrap();
        assert_eq!(loaded.down_sql, down);
    }

    #[test]
    fn crlf_preserves_statements_and_the_migration_checksum() {
        let tmp = TempDir::new().unwrap();
        let store = MigrationFileStore::new(tmp.path());
        let up = vec![
            "-- first table\nCREATE TABLE users (id INTEGER PRIMARY KEY)".to_string(),
            "-- comment block".to_string(),
            "CREATE TABLE posts (id INTEGER PRIMARY KEY)".to_string(),
        ];
        let down = vec![
            "DROP TABLE posts".to_string(),
            "DROP TABLE users".to_string(),
        ];
        store.write_migration("crlf", &up, &down).unwrap();
        let original = store.load_migration("crlf").unwrap();
        for file in ["up.sql", "down.sql"] {
            let path = tmp.path().join("crlf").join(file);
            let content = fs::read_to_string(&path).unwrap();
            fs::write(path, content.replace('\n', "\r\n")).unwrap();
        }
        let loaded = store.load_migration("crlf").unwrap();
        assert_eq!(loaded.up_sql, up);
        assert_eq!(loaded.down_sql, down);
        assert_eq!(loaded.checksum, original.checksum);
    }

    #[test]
    fn an_empty_sentinel_comment_does_not_hide_following_sql() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("up.sql");
        fs::write(&path, "-- (empty)\nSELECT 1;").unwrap();
        assert_eq!(parse_sql_file(&path).unwrap(), vec!["-- (empty)\nSELECT 1"]);
    }
}
