//! The open documents and the schema each one is analysed inside.
//!
//! A document with a path is never analysed alone: it is assembled with the
//! files it imports (see [`crate::workspace`]) so that cross-file references
//! resolve. Editing one file therefore changes the meaning of the others that
//! share its schema, which is what [`Documents::related`] reports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::mapref::one::Ref;
use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::document::DocumentState;
use crate::workspace::{canonical, file_path_from_uri, Workspace};

/// The cache of open documents, keyed by URI.
#[derive(Default)]
pub(crate) struct Documents {
    cache: DashMap<Url, DocumentState>,
}

impl Documents {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The cached state of an open document.
    pub(crate) fn get(&self, uri: &Url) -> Option<Ref<'_, Url, DocumentState>> {
        self.cache.get(uri)
    }

    pub(crate) fn store(&self, uri: Url, state: DocumentState) {
        self.cache.insert(uri, state);
    }

    pub(crate) fn close(&self, uri: &Url) {
        self.cache.remove(uri);
    }

    /// Whether `uri` is already cached with exactly this text, in which case
    /// analysing it again would produce the same state.
    pub(crate) fn is_current(&self, uri: &Url, source: &str) -> bool {
        self.cache
            .get(uri)
            .is_some_and(|existing| existing.source == source)
    }

    /// Build the state for `uri`, assembling the schema it belongs to when it
    /// names a file on disk.
    pub(crate) fn analyze(&self, uri: &Url, source: String) -> DocumentState {
        let Some(path) = file_path_from_uri(uri) else {
            return DocumentState::new(source);
        };

        let mut open = self.open_buffers();
        open.insert(canonical(&path), source.clone());

        // A file another open document imports is a piece of that schema, and
        // reading it on its own would report the very references the import
        // exists to resolve.
        let workspace = self
            .importing_root(uri)
            .and_then(|(root, root_uri)| Workspace::load_for_uri(&root, &root_uri, &open))
            .filter(|workspace| workspace.contains(uri))
            .or_else(|| Workspace::load_for_uri(&path, uri, &open));

        match workspace {
            Some(workspace) => DocumentState::with_workspace(source, uri, Arc::new(workspace)),
            None => DocumentState::new(source),
        }
    }

    /// Every open document that shares a schema with `uri`, with its text.
    ///
    /// Editing a file changes the meaning of the files that import it, and of
    /// the files it imports — those documents are now looking at a different
    /// schema, and their squiggles have to move with it.
    pub(crate) fn related(&self, uri: &Url) -> Vec<(Url, String)> {
        let covered: Vec<Url> = self
            .cache
            .get(uri)
            .and_then(|state| {
                state
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.uris().cloned().collect())
            })
            .unwrap_or_default();

        self.cache
            .iter()
            .filter(|entry| entry.key() != uri)
            .filter(|entry| {
                covered.contains(entry.key())
                    || entry
                        .value()
                        .workspace
                        .as_ref()
                        .is_some_and(|workspace| workspace.contains(uri))
            })
            .map(|entry| (entry.key().clone(), entry.value().source.clone()))
            .collect()
    }

    /// Every open document with its text, used when a file changed on disk
    /// outside the editor and no single document is the origin of the change.
    pub(crate) fn open(&self) -> Vec<(Url, String)> {
        self.cache
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().source.clone()))
            .collect()
    }

    /// The path of an open document whose schema already includes `uri`.
    fn importing_root(&self, uri: &Url) -> Option<(PathBuf, Url)> {
        self.cache.iter().find_map(|entry| {
            if entry.key() == uri {
                return None;
            }
            let workspace = entry.value().workspace.as_ref()?;
            workspace.contains(uri).then_some(())?;
            let root_uri = entry.key().clone();
            Some((file_path_from_uri(&root_uri)?, root_uri))
        })
    }

    /// The text of every open document, keyed by path, so that an imported file
    /// being edited is assembled as the developer sees it rather than as it was
    /// last saved.
    fn open_buffers(&self) -> HashMap<PathBuf, String> {
        self.cache
            .iter()
            .filter_map(|entry| {
                let path = file_path_from_uri(entry.key())?;
                Some((canonical(&path), entry.value().source.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use tower_lsp::lsp_types::{DidOpenTextDocumentParams, TextDocumentItem};
    use tower_lsp::LanguageServer;

    use crate::backend::Backend;
    use crate::test_support::{file_uri, schema_dir, service};

    fn opened(uri: &tower_lsp::lsp_types::Url, text: &str) -> DidOpenTextDocumentParams {
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "nautilus".to_string(),
                version: 1,
                text: text.to_string(),
            },
        }
    }

    #[tokio::test]
    async fn editing_an_imported_buffer_refreshes_the_file_that_imports_it() {
        let dir = schema_dir("refresh");
        let enums = dir.join("enums.nautilus");
        std::fs::write(&enums, "enum Role {\n  USER\n}\n").expect("write enums");
        let user = dir.join("user.nautilus");
        let user_source = "import \"./enums.nautilus\"\n\nmodel User {\n  id     Int    @id\n  status Status\n}\n";
        std::fs::write(&user, user_source).expect("write user");

        let (service, _socket) = service();
        let backend = service.inner();
        let user_uri = file_uri(&user);
        let enums_uri = file_uri(&enums);

        backend.did_open(opened(&user_uri, user_source)).await;

        let unknown_status = |backend: &Backend| {
            let state = backend.documents.get(&user_uri).expect("cached document");
            let workspace = state.workspace.as_ref().expect("assembled workspace");
            workspace
                .diagnostics()
                .into_iter()
                .any(|(_, diags)| diags.iter().any(|d| d.message.contains("Status")))
        };
        assert!(
            unknown_status(backend),
            "Status is not declared anywhere yet"
        );

        backend
            .did_open(opened(
                &enums_uri,
                "enum Role {\n  USER\n}\n\nenum Status {\n  ACTIVE\n}\n",
            ))
            .await;

        assert!(
            !unknown_status(backend),
            "the unsaved buffer of the imported file resolves the reference"
        );
    }

    #[tokio::test]
    async fn opening_an_imported_file_analyses_it_inside_the_schema_that_imports_it() {
        let dir = schema_dir("imported-open");
        let post = dir.join("post.nautilus");
        let post_source = "model Post {\n  id       Int  @id\n  authorId Int\n  author   User @relation(fields: [authorId], references: [id])\n}\n";
        std::fs::write(&post, post_source).expect("write post");
        let user = dir.join("user.nautilus");
        let user_source =
            "import \"./post.nautilus\"\n\nmodel User {\n  id    Int    @id\n  posts Post[]\n}\n";
        std::fs::write(&user, user_source).expect("write user");

        let (service, _socket) = service();
        let backend = service.inner();

        let post_uri = file_uri(&post);
        backend.did_open(opened(&post_uri, post_source)).await;

        let has_unknown_user = |backend: &Backend| {
            let state = backend.documents.get(&post_uri).expect("cached document");
            let workspace = state.workspace.as_ref().expect("workspace");
            workspace
                .diagnostics()
                .into_iter()
                .any(|(_, diags)| diags.iter().any(|d| d.message.contains("User")))
        };
        assert!(
            has_unknown_user(backend),
            "on its own, post.nautilus cannot see User"
        );

        backend
            .did_open(opened(&file_uri(&user), user_source))
            .await;

        assert!(
            !has_unknown_user(backend),
            "once the importing file is open, post.nautilus is read as part of that schema"
        );
    }
}
