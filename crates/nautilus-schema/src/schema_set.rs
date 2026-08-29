//! Loading a schema that is spread across more than one `.nautilus` file.
//!
//! A [`SchemaSet`] concatenates the files into a single source and remembers
//! where each one starts, so the existing single-source lexer, parser and
//! validator are used unchanged while diagnostics still point at the file the
//! developer wrote.

use std::io;
use std::path::{Path, PathBuf};

use crate::error::SchemaError;
use crate::{discover_schema_paths, ValidatedSchema};

/// One `.nautilus` file inside a [`SchemaSet`].
#[derive(Debug, Clone)]
struct SchemaFile {
    path: PathBuf,
    /// Byte offset of this file's first character in the concatenated source.
    start: usize,
    /// Byte offset one past this file's last character.
    end: usize,
}

/// A schema assembled from one or more `.nautilus` files.
///
/// Files are concatenated in the order they are given — [`SchemaSet::load_dir`]
/// sorts them lexicographically — which makes the assembled source stable
/// across runs, so a schema split across files diffs and errors the same way
/// every time.
///
/// Declaration order does not affect meaning: the validator resolves every
/// reference by name across the whole set, so a model in `post.nautilus` may
/// reference an enum declared in `enums.nautilus` regardless of file order.
#[derive(Debug, Clone)]
pub struct SchemaSet {
    source: String,
    files: Vec<SchemaFile>,
}

impl SchemaSet {
    /// Load every `.nautilus` file directly inside `dir`, in lexicographic
    /// order.
    pub fn load_dir(dir: &Path) -> io::Result<SchemaSet> {
        let paths = discover_schema_paths(dir)?;
        if paths.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no .nautilus file found in {}", dir.display()),
            ));
        }
        SchemaSet::load(&paths)
    }

    /// Load `path` as a single-file set, or as a directory set when it names a
    /// directory.
    pub fn load_path(path: &Path) -> io::Result<SchemaSet> {
        if path.is_dir() {
            SchemaSet::load_dir(path)
        } else {
            SchemaSet::load(std::slice::from_ref(&path.to_path_buf()))
        }
    }

    /// Load the given files, in the order given.
    pub fn load(paths: &[PathBuf]) -> io::Result<SchemaSet> {
        let mut source = String::new();
        let mut files = Vec::with_capacity(paths.len());

        for path in paths {
            let contents = std::fs::read_to_string(path)?;
            let start = source.len();
            source.push_str(&contents);
            // A file that does not end in a newline would otherwise glue its
            // last declaration onto the next file's first one.
            if !source.ends_with('\n') {
                source.push('\n');
            }
            let end = source.len();
            files.push(SchemaFile {
                path: path.clone(),
                start,
                end,
            });
        }

        Ok(SchemaSet { source, files })
    }

    /// The concatenated source, which is what the lexer and parser consume.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The files this set was assembled from, in concatenation order.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    /// The path reported to the user when the whole set has to be named as one
    /// thing: the single file when there is only one, otherwise its directory.
    pub fn display_path(&self) -> PathBuf {
        match self.files.as_slice() {
            [only] => only.path.clone(),
            _ => self
                .files
                .first()
                .and_then(|file| file.path.parent())
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        }
    }

    /// Parse and validate the assembled source.
    pub fn validate(&self) -> crate::Result<ValidatedSchema> {
        crate::validate_schema_source(&self.source)
    }

    /// Render `error` as `path:line:column: message`, resolved against the file
    /// the offending span actually came from.
    pub fn format_error(&self, error: &SchemaError) -> String {
        let Some(span) = error.span() else {
            return error.to_string();
        };
        let Some(file) = self
            .files
            .iter()
            .find(|file| span.start >= file.start && span.start < file.end)
        else {
            return error.format_with_file(&self.display_path().to_string_lossy(), &self.source);
        };

        error.shifted_back(file.start).format_with_file(
            &file.path.to_string_lossy(),
            &self.source[file.start..file.end],
        )
    }
}
