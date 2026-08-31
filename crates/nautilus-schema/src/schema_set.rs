//! Loading a schema that is spread across more than one `.nautilus` file.
//!
//! A [`SchemaSet`] concatenates the files into a single source and remembers
//! where each one starts, so the existing single-source lexer, parser and
//! validator are used unchanged while diagnostics still point at the file the
//! developer wrote.
//!
//! Files enter a set two ways: by being listed (a directory of schema files, or
//! an explicit list) and by being named in an [`import`](crate::ast::ImportDecl)
//! statement of a file that is already in the set.  Imports are followed
//! transitively, so opening the root file of a schema pulls in the whole thing.

use std::collections::HashSet;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::analysis::{analyze, AnalysisResult};
use crate::diagnostic::Diagnostic;
use crate::error::SchemaError;
use crate::span::Span;
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

/// A file inside a [`SchemaSet`], with its place in the assembled source.
#[derive(Debug, Clone, Copy)]
pub struct SchemaSetFile<'a> {
    /// Path this file was read from.
    pub path: &'a Path,
    /// Byte offset of the file's first character in the assembled source.
    pub start: usize,
    /// The file's own text, as it appears in the assembled source.
    pub source: &'a str,
}

/// A schema assembled from one or more `.nautilus` files.
///
/// Files are concatenated in the order they are given — [`SchemaSet::load_dir`]
/// sorts them lexicographically — which makes the assembled source stable
/// across runs, so a schema split across files diffs and errors the same way
/// every time.  A file's imports follow it immediately, depth-first, and a file
/// reached twice is concatenated once.
///
/// Declaration order does not affect meaning: the validator resolves every
/// reference by name across the whole set, so a model in `post.nautilus` may
/// reference an enum declared in `enums.nautilus` regardless of file order.
#[derive(Debug, Clone)]
pub struct SchemaSet {
    source: String,
    files: Vec<SchemaFile>,
    import_errors: Vec<SchemaError>,
}

impl SchemaSet {
    /// Load every `.nautilus` file directly inside `dir`, in lexicographic
    /// order, plus everything those files import.
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
        SchemaSet::load_path_with(path, &|_| None)
    }

    /// Load the given files, in the order given, plus everything they import.
    pub fn load(paths: &[PathBuf]) -> io::Result<SchemaSet> {
        SchemaSet::load_with(paths, &|_| None)
    }

    /// Like [`SchemaSet::load_path`], reading each file through `overlay`
    /// first.
    ///
    /// An editor passes the unsaved buffer of every open document, so the set
    /// reflects what the developer is looking at rather than what was last
    /// written to disk.  `overlay` returning `None` falls back to the file on
    /// disk.
    pub fn load_path_with(
        path: &Path,
        overlay: &dyn Fn(&Path) -> Option<String>,
    ) -> io::Result<SchemaSet> {
        if path.is_dir() {
            let paths = discover_schema_paths(path)?;
            if paths.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no .nautilus file found in {}", path.display()),
                ));
            }
            SchemaSet::load_with(&paths, overlay)
        } else {
            SchemaSet::load_with(std::slice::from_ref(&path.to_path_buf()), overlay)
        }
    }

    /// Like [`SchemaSet::load`], reading each file through `overlay` first.
    pub fn load_with(
        paths: &[PathBuf],
        overlay: &dyn Fn(&Path) -> Option<String>,
    ) -> io::Result<SchemaSet> {
        let mut assembler = Assembler {
            overlay,
            visited: HashSet::new(),
            set: SchemaSet {
                source: String::new(),
                files: Vec::new(),
                import_errors: Vec::new(),
            },
        };

        for path in paths {
            assembler.add_root(path)?;
        }

        Ok(assembler.set)
    }

    /// The concatenated source, which is what the lexer and parser consume.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The files this set was assembled from, in concatenation order.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    /// The files this set was assembled from, each with its place in the
    /// assembled source.
    pub fn files(&self) -> impl Iterator<Item = SchemaSetFile<'_>> {
        self.files.iter().map(|file| self.view(file))
    }

    /// The file the assembled-source `offset` falls in.
    pub fn file_at(&self, offset: usize) -> Option<SchemaSetFile<'_>> {
        self.files
            .iter()
            .find(|file| offset >= file.start && offset < file.end)
            .map(|file| self.view(file))
    }

    /// Imports that could not be resolved, with spans into the assembled
    /// source.
    ///
    /// Resolution failures are collected rather than fatal so that an editor
    /// can still analyse the files it did reach and report the broken import
    /// where it is written.
    pub fn import_errors(&self) -> &[SchemaError] {
        &self.import_errors
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
        if let Some(error) = self.import_errors.first() {
            return Err(error.clone());
        }
        crate::validate_schema_source(&self.source)
    }

    /// Analyze the assembled source, reporting unresolved imports alongside the
    /// lex, parse and validation diagnostics.
    pub fn analyze(&self) -> AnalysisResult {
        let mut result = analyze(&self.source);
        result
            .diagnostics
            .extend(self.import_errors.iter().map(Diagnostic::from));
        result
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

    fn view<'a>(&'a self, file: &'a SchemaFile) -> SchemaSetFile<'a> {
        SchemaSetFile {
            path: &file.path,
            start: file.start,
            source: &self.source[file.start..file.end],
        }
    }
}

/// Assembles a [`SchemaSet`] by walking files and the imports they declare.
struct Assembler<'a> {
    overlay: &'a dyn Fn(&Path) -> Option<String>,
    visited: HashSet<PathBuf>,
    set: SchemaSet,
}

impl Assembler<'_> {
    /// Add a file the caller named, whose absence is an error the caller has to
    /// see.
    fn add_root(&mut self, path: &Path) -> io::Result<()> {
        if !self.visited.insert(identity(path)) {
            return Ok(());
        }
        let source = self.read(path)?;
        let index = self.push(path, source);
        self.follow_imports(path, index);
        Ok(())
    }

    /// Add a file another file imported, whose absence is a diagnostic on the
    /// `import` statement rather than a failure to load anything at all.
    fn add_import(&mut self, path: &Path, span: Span) {
        if !self.visited.insert(identity(path)) {
            return;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("nautilus") {
            self.set.import_errors.push(SchemaError::Validation(
                format!(
                    "Cannot import schema {}: imported files must use the .nautilus extension",
                    path.display()
                ),
                span,
            ));
            return;
        }
        match self.read(path) {
            Ok(source) => {
                let index = self.push(path, source);
                self.follow_imports(path, index);
            }
            Err(error) => self.set.import_errors.push(SchemaError::Validation(
                format!("Cannot read imported schema {}: {}", path.display(), error),
                span,
            )),
        }
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        match (self.overlay)(path) {
            Some(source) => Ok(source),
            None => std::fs::read_to_string(path),
        }
    }

    /// Append `source` to the assembled text and return the index of the file
    /// it became.
    fn push(&mut self, path: &Path, source: String) -> usize {
        let start = self.set.source.len();
        self.set.source.push_str(&source);
        // A file that does not end in a newline would otherwise glue its last
        // declaration onto the next file's first one.
        if !self.set.source.ends_with('\n') {
            self.set.source.push('\n');
        }
        let end = self.set.source.len();
        self.set.files.push(SchemaFile {
            path: path.to_path_buf(),
            start,
            end,
        });
        self.set.files.len() - 1
    }

    /// Resolve and add everything the file at `index` imports.
    fn follow_imports(&mut self, path: &Path, index: usize) {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        for (target, span) in self.imports_of(index) {
            // Cleaned up lexically so that an import of `./domain` names the
            // file as `schema/domain/post.nautilus` in diagnostics rather than
            // as `schema/./domain/post.nautilus`.
            let resolved = normalize(&dir.join(&target));
            if resolved.is_dir() {
                match discover_schema_paths(&resolved) {
                    Ok(paths) if paths.is_empty() => {
                        self.set.import_errors.push(SchemaError::Validation(
                            format!(
                                "Imported directory {} holds no .nautilus file",
                                resolved.display()
                            ),
                            span,
                        ));
                    }
                    Ok(paths) => {
                        for path in paths {
                            self.add_import(&path, span);
                        }
                    }
                    Err(error) => self.set.import_errors.push(SchemaError::Validation(
                        format!(
                            "Cannot read imported directory {}: {}",
                            resolved.display(),
                            error
                        ),
                        span,
                    )),
                }
            } else {
                self.add_import(&resolved, span);
            }
        }
    }

    /// The import statements of the file at `index`, with spans rebased onto
    /// the assembled source.
    ///
    /// A file that does not parse contributes no imports; its syntax errors are
    /// reported by the analysis pass over the assembled source.
    fn imports_of(&self, index: usize) -> Vec<(String, Span)> {
        let file = &self.set.files[index];
        let start = file.start;
        let file_source = &self.set.source[start..file.end];
        let Ok(parsed) = crate::parse_schema_source_with_recovery(file_source) else {
            return Vec::new();
        };
        parsed
            .ast
            .imports()
            .map(|import| {
                (
                    import.path.clone(),
                    Span::new(import.span.start + start, import.span.end + start),
                )
            })
            .collect()
    }
}

/// A key that is equal for two paths naming the same file.
///
/// `canonicalize` is the truth when the file exists — it resolves symlinks and
/// `..` — and a lexical cleanup stands in for the files that do not, so that a
/// missing import is still reported once rather than once per path spelling.
fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize(path))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
