//! The schema an open document belongs to.
//!
//! A `.nautilus` file that declares `import "…"` is one piece of a larger
//! schema, and analysing it on its own turns every cross-file reference into an
//! error the developer did not make.  A [`Workspace`] assembles the open file
//! with everything it imports — reading unsaved buffers instead of disk where
//! the editor has one — analyses the assembled source once, and maps the
//! resulting spans back to the file and range each came from.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nautilus_schema::{analysis::AnalysisResult, LineIndex, SchemaSet, Span};
use tower_lsp::lsp_types::{Diagnostic, Range, Url};

use crate::convert::{nautilus_diagnostic_to_lsp_with_index, span_to_range_with_index};

/// One file of an assembled schema, with what is needed to talk about it in LSP
/// terms.
struct WorkspaceFile {
    path: PathBuf,
    uri: Url,
    start: usize,
    end: usize,
    source: String,
    line_index: LineIndex,
}

/// An open document assembled with the files it imports, analysed as one
/// schema.
pub struct Workspace {
    analysis: AnalysisResult,
    source: String,
    files: Vec<WorkspaceFile>,
}

impl Workspace {
    /// Assemble and analyse the schema rooted at `root`, reading each file from
    /// `open` when the editor holds a buffer for it.
    ///
    /// Returns `None` when the file cannot be read at all — an unsaved
    /// `untitled:` buffer has no path, and a document the client sent for a
    /// deleted file has nothing to assemble.
    #[cfg(test)]
    pub fn load(root: &Path, open: &HashMap<PathBuf, String>) -> Option<Workspace> {
        Self::load_inner(root, None, open)
    }

    /// Assemble a workspace while preserving the root URI exactly as the
    /// editor sent it. VS Code percent-encodes the Windows drive colon, and
    /// diagnostics must use the same URI spelling to attach to the document.
    pub fn load_for_uri(
        root: &Path,
        root_uri: &Url,
        open: &HashMap<PathBuf, String>,
    ) -> Option<Workspace> {
        Self::load_inner(root, Some(root_uri), open)
    }

    fn load_inner(
        root: &Path,
        root_uri: Option<&Url>,
        open: &HashMap<PathBuf, String>,
    ) -> Option<Workspace> {
        let set =
            SchemaSet::load_path_with(root, &|path| open.get(&canonical(path)).cloned()).ok()?;
        let root = canonical(root);

        let files = set
            .files()
            .filter_map(|file| {
                let path = canonical(file.path);
                let uri = root_uri
                    .filter(|_| path == root)
                    .cloned()
                    .or_else(|| Url::from_file_path(&path).ok())?;
                Some(WorkspaceFile {
                    path,
                    uri,
                    start: file.start,
                    end: file.start + file.source.len(),
                    source: file.source.to_string(),
                    line_index: LineIndex::new(file.source),
                })
            })
            .collect();

        Some(Workspace {
            analysis: set.analyze(),
            source: set.source().to_string(),
            files,
        })
    }

    /// The assembled source every offset in this workspace refers to.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The analysis of the assembled source.
    pub fn analysis(&self) -> &AnalysisResult {
        &self.analysis
    }

    /// Byte offset at which `uri`'s text starts in the assembled source.
    pub fn base_of(&self, uri: &Url) -> Option<usize> {
        self.file_for_uri(uri).map(|file| file.start)
    }

    /// Whether `uri` is one of the files this workspace was assembled from.
    pub fn contains(&self, uri: &Url) -> bool {
        self.file_for_uri(uri).is_some()
    }

    /// The files this workspace was assembled from.
    pub fn uris(&self) -> impl Iterator<Item = &Url> {
        self.files.iter().map(|file| &file.uri)
    }

    /// Diagnostics grouped by the file they belong to.
    ///
    /// Every file of the workspace gets an entry, empty ones included, so that
    /// a problem fixed in one file clears the squiggle it left in another.
    pub fn diagnostics(&self) -> Vec<(Url, Vec<Diagnostic>)> {
        let mut grouped: Vec<(Url, Vec<Diagnostic>)> = self
            .files
            .iter()
            .map(|file| (file.uri.clone(), Vec::new()))
            .collect();

        for diagnostic in &self.analysis.diagnostics {
            let Some(index) = self.index_at(diagnostic.span.start) else {
                continue;
            };
            let file = &self.files[index];
            let local = nautilus_schema::diagnostic::Diagnostic {
                severity: diagnostic.severity,
                message: diagnostic.message.clone(),
                span: self.local_span(file, diagnostic.span),
            };
            grouped[index].1.push(nautilus_diagnostic_to_lsp_with_index(
                &file.source,
                &file.line_index,
                &local,
            ));
        }

        grouped
    }

    /// The file and range an assembled-source span points at.
    pub fn locate(&self, span: Span) -> Option<(Url, Range)> {
        let index = self.index_at(span.start)?;
        let file = &self.files[index];
        let local = self.local_span(file, span);
        Some((
            file.uri.clone(),
            span_to_range_with_index(&file.source, &file.line_index, &local),
        ))
    }

    fn index_at(&self, offset: usize) -> Option<usize> {
        self.files
            .iter()
            .position(|file| offset >= file.start && offset < file.end)
    }

    fn file_for_uri(&self, uri: &Url) -> Option<&WorkspaceFile> {
        if let Some(file) = self.files.iter().find(|file| &file.uri == uri) {
            return Some(file);
        }
        let path = canonical(&file_path_from_uri(uri)?);
        self.files.iter().find(|file| file.path == path)
    }

    /// Rebase `span` from the assembled source onto `file`, clamped to it: a
    /// span that runs past the end of one file would otherwise highlight into
    /// the next.
    fn local_span(&self, file: &WorkspaceFile, span: Span) -> Span {
        let start = span.start.saturating_sub(file.start);
        let end = span.end.min(file.end).saturating_sub(file.start);
        Span::new(start, end.max(start))
    }
}

/// The path under which an open document is looked up, so that the same file
/// reached through a symlink or a `..` is recognised as the one the editor
/// holds.
pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Convert a client document URI into a local path.
///
/// VS Code serializes Windows drive letters as `file:///c%3A/...`, while the
/// `url` crate's standard file-path conversion expects `file:///c:/...`.
pub fn file_path_from_uri(uri: &Url) -> Option<PathBuf> {
    if let Ok(path) = uri.to_file_path() {
        return Some(path);
    }

    #[cfg(windows)]
    {
        let path = uri.path();
        let encoded_drive = path.get(1..5)?;
        if uri.scheme() != "file"
            || !encoded_drive[1..].eq_ignore_ascii_case("%3a")
            || !encoded_drive.as_bytes()[0].is_ascii_alphabetic()
        {
            return None;
        }

        let decoded_drive = format!("{}:", &encoded_drive[..1]);
        let normalized = uri.as_str().replacen(encoded_drive, &decoded_drive, 1);
        Url::parse(&normalized).ok()?.to_file_path().ok()
    }

    #[cfg(not(windows))]
    None
}

#[cfg(test)]
mod tests {
    use super::{canonical, file_path_from_uri, Workspace};
    use std::collections::HashMap;
    use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

    fn write(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("write schema file");
        path
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nautilus-lsp-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[cfg(windows)]
    #[test]
    fn vscode_percent_encoded_windows_drive_uri_resolves_to_a_path() {
        let root = write(
            &temp_dir("encoded-uri"),
            "schema.nautilus",
            "model User { id Int @id }",
        );
        let expected = canonical(&root);
        let mut serialized = Url::from_file_path(&expected).unwrap().to_string();
        let path_start = serialized.find(":///").unwrap() + 4;
        let drive_colon = path_start + serialized[path_start..].find(':').unwrap();
        serialized.replace_range(drive_colon..=drive_colon, "%3A");
        let uri = Url::parse(&serialized).unwrap();

        assert_eq!(
            file_path_from_uri(&uri).map(|path| canonical(&path)),
            Some(expected)
        );
    }

    #[test]
    fn imported_declarations_resolve_and_leave_no_diagnostics() {
        let dir = temp_dir("resolves");
        write(&dir, "enums.nautilus", "enum Role {\n  USER\n}\n");
        let root = write(
            &dir,
            "user.nautilus",
            "import \"./enums.nautilus\"\n\nmodel User {\n  id   Int  @id\n  role Role\n}\n",
        );

        let workspace = Workspace::load(&root, &HashMap::new()).expect("workspace");
        for (uri, diagnostics) in workspace.diagnostics() {
            assert!(
                diagnostics.is_empty(),
                "unexpected diagnostics for {}: {:?}",
                uri,
                diagnostics
            );
        }
        assert_eq!(workspace.diagnostics().len(), 2, "both files are analysed");
    }

    #[test]
    fn a_file_without_imports_still_sees_only_itself() {
        let dir = temp_dir("isolated");
        write(&dir, "enums.nautilus", "enum Role {\n  USER\n}\n");
        let root = write(
            &dir,
            "user.nautilus",
            "model User {\n  id   Int  @id\n  role Role\n}\n",
        );

        let workspace = Workspace::load(&root, &HashMap::new()).expect("workspace");
        assert_eq!(
            workspace.diagnostics().len(),
            1,
            "a file that imports nothing is a schema of one file"
        );
        let diagnostics = workspace.diagnostics();
        assert!(diagnostics
            .iter()
            .any(|(_, diags)| diags.iter().any(|d| d.message.contains("Role"))));
    }

    #[test]
    fn diagnostics_are_reported_against_the_file_that_holds_them() {
        let dir = temp_dir("per-file");
        write(
            &dir,
            "post.nautilus",
            "model Post {\n  id   Int  @id\n  tag  Nope\n}\n",
        );
        let root = write(
            &dir,
            "user.nautilus",
            "import \"./post.nautilus\"\n\nmodel User {\n  id Int @id\n}\n",
        );

        let workspace = Workspace::load(&root, &HashMap::new()).expect("workspace");
        let diagnostics = workspace.diagnostics();
        let post_uri = Url::from_file_path(canonical(&dir).join("post.nautilus")).unwrap();
        let (_, post_diags) = diagnostics
            .iter()
            .find(|(uri, _)| uri.path().ends_with("post.nautilus"))
            .expect("post.nautilus is part of the workspace");
        assert_eq!(post_diags.len(), 1, "{:?}", post_diags);
        assert_eq!(post_diags[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(post_diags[0].range.start.line, 2);
        let _ = post_uri;

        let (_, user_diags) = diagnostics
            .iter()
            .find(|(uri, _)| uri.path().ends_with("user.nautilus"))
            .expect("the root file is part of the workspace");
        assert!(user_diags.is_empty(), "{:?}", user_diags);
    }

    #[test]
    fn unsaved_buffers_win_over_what_is_on_disk() {
        let dir = temp_dir("overlay");
        let imported = write(&dir, "enums.nautilus", "enum Role {\n  USER\n}\n");
        let root = write(
            &dir,
            "user.nautilus",
            "import \"./enums.nautilus\"\n\nmodel User {\n  id    Int    @id\n  status Status\n}\n",
        );

        let mut open = HashMap::new();
        open.insert(
            canonical(&imported),
            "enum Role {\n  USER\n}\n\nenum Status {\n  ACTIVE\n}\n".to_string(),
        );

        let workspace = Workspace::load(&root, &open).expect("workspace");
        for (uri, diagnostics) in workspace.diagnostics() {
            assert!(
                diagnostics.is_empty(),
                "unexpected diagnostics for {}: {:?}",
                uri,
                diagnostics
            );
        }
    }

    #[test]
    fn an_unresolved_import_is_reported_on_the_import_statement() {
        let dir = temp_dir("missing");
        let root = write(
            &dir,
            "user.nautilus",
            "import \"./gone.nautilus\"\n\nmodel User {\n  id Int @id\n}\n",
        );

        let workspace = Workspace::load(&root, &HashMap::new()).expect("workspace");
        let diagnostics = workspace.diagnostics();
        let (_, diags) = diagnostics.first().expect("the root file");
        assert_eq!(diags.len(), 1, "{:?}", diags);
        assert!(diags[0].message.contains("gone.nautilus"), "{:?}", diags[0]);
        assert_eq!(diags[0].range.start.line, 0);
    }
}
