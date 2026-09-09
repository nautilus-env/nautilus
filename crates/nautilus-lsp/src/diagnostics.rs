//! Publishing diagnostics to the file each one belongs to.
//!
//! A document is analysed together with the files it imports, so one analysis
//! produces diagnostics for several files. Which files an open document last
//! spoke for is remembered, so that a file dropping out of a schema — or the
//! document closing — has its squiggles cleared instead of left behind.

use dashmap::DashMap;
use tower_lsp::lsp_types::Url;
use tower_lsp::Client;

use crate::convert::nautilus_diagnostic_to_lsp_with_index;
use crate::document::DocumentState;

pub(crate) struct Diagnostics {
    client: Client,
    /// Files each open document last published diagnostics for.
    published: DashMap<Url, Vec<Url>>,
}

impl Diagnostics {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            published: DashMap::new(),
        }
    }

    /// Publish the diagnostics of `state`, one batch per file of its schema.
    pub(crate) async fn publish(&self, uri: &Url, state: &DocumentState) {
        let batches = match &state.workspace {
            Some(workspace) => workspace.diagnostics(),
            None => {
                let diagnostics = state
                    .analysis
                    .diagnostics
                    .iter()
                    .map(|d| {
                        nautilus_diagnostic_to_lsp_with_index(&state.source, &state.line_index, d)
                    })
                    .collect();
                vec![(uri.clone(), diagnostics)]
            }
        };

        let covered: Vec<Url> = batches.iter().map(|(uri, _)| uri.clone()).collect();
        for (target, diagnostics) in batches {
            self.client
                .publish_diagnostics(target, diagnostics, None)
                .await;
        }

        let previous = self.published.insert(uri.clone(), covered.clone());
        for stale in previous.into_iter().flatten() {
            if !covered.contains(&stale) && !self.is_covered(&stale, uri) {
                self.clear_one(stale).await;
            }
        }
    }

    /// Clear everything a closing document was speaking for, keeping the files
    /// another open document still reports on.
    pub(crate) async fn clear(&self, uri: &Url) {
        let covered = self
            .published
            .remove(uri)
            .map(|(_, covered)| covered)
            .unwrap_or_default();
        self.clear_one(uri.clone()).await;

        for stale in covered {
            if stale != *uri && !self.is_covered(&stale, uri) {
                self.clear_one(stale).await;
            }
        }
    }

    async fn clear_one(&self, uri: Url) {
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    /// Whether a document other than `except` still reports diagnostics for
    /// `uri`.
    fn is_covered(&self, uri: &Url, except: &Url) -> bool {
        self.published
            .iter()
            .any(|entry| entry.key() != except && entry.value().contains(uri))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use tower_lsp::lsp_types::{
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        Position, PublishDiagnosticsParams, Range, TextDocumentContentChangeEvent,
        TextDocumentIdentifier, TextDocumentItem, VersionedTextDocumentIdentifier,
    };
    use tower_lsp::LanguageServer;

    use crate::test_support::{file_uri, initialized_service, schema_dir, vscode_file_uri, within};

    fn published(request: &tower_lsp::jsonrpc::Request) -> PublishDiagnosticsParams {
        assert_eq!(request.method(), "textDocument/publishDiagnostics");
        serde_json::from_value(request.params().cloned().expect("diagnostic params"))
            .expect("valid diagnostic params")
    }

    #[tokio::test]
    async fn changing_to_a_missing_import_publishes_a_diagnostic() {
        let dir = schema_dir("missing-import-diagnostic");
        let schema = dir.join("schema.nautilus");
        let initial = "model User {\r\n  id Int @id\r\n}\r\n";
        std::fs::write(&schema, initial).expect("write schema");

        let (service, mut socket) = initialized_service().await;
        let backend = service.inner();
        let uri = vscode_file_uri(&schema);
        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: initial.to_string(),
                },
            })
            .await;
        let opened = socket.next().await.expect("open diagnostics");
        assert_eq!(opened.method(), "textDocument/publishDiagnostics");

        backend
            .did_change(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                    range_length: None,
                    text: concat!(
                        "import \"./missing.nautilus\"\r\n",
                        "import \"./models\"\r\n\r\n"
                    )
                    .to_string(),
                }],
            })
            .await;

        let changed = socket.next().await.expect("changed diagnostics");
        let params = published(&changed);
        assert_eq!(params.uri, uri);
        assert_eq!(params.diagnostics.len(), 2, "{params:?}");
        assert!(
            params
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("missing.nautilus")),
            "{params:?}"
        );
        assert!(
            params
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(".nautilus extension")),
            "{params:?}"
        );
        assert_eq!(params.diagnostics[0].range.start, Position::new(0, 0));
    }

    /// One analysis publishes a batch per file, and the client channel holds a
    /// single message, so the notifications have to be read while the handler
    /// is still sending them.
    async fn collect(
        socket: &mut tower_lsp::ClientSocket,
        count: usize,
    ) -> Vec<PublishDiagnosticsParams> {
        let mut batches = Vec::with_capacity(count);
        for _ in 0..count {
            batches.push(published(&socket.next().await.expect("diagnostics")));
        }
        batches
    }

    #[tokio::test]
    async fn closing_a_document_clears_the_imported_file_it_spoke_for() {
        let dir = schema_dir("close-clears");
        let shared = dir.join("shared.nautilus");
        std::fs::write(
            &shared,
            "model Note {
  id   Int     @id
  kind Missing
}
",
        )
        .expect("write shared");
        let user = dir.join("user.nautilus");
        let user_source = "import \"./shared.nautilus\"

model User {
  id Int @id
}
";
        std::fs::write(&user, user_source).expect("write user");

        let (service, mut socket) = initialized_service().await;
        let backend = service.inner();
        let user_uri = file_uri(&user);
        let shared_uri = file_uri(&shared);

        let opened = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: user_uri.clone(),
                language_id: "nautilus".to_string(),
                version: 1,
                text: user_source.to_string(),
            },
        };
        let (_, batches) = within(futures::future::join(
            backend.did_open(opened),
            collect(&mut socket, 2),
        ))
        .await;

        let shared_batch = batches
            .iter()
            .find(|params| params.uri == shared_uri)
            .expect("the imported file receives its own diagnostics");
        assert!(
            !shared_batch.diagnostics.is_empty(),
            "the unknown type is reported on the file that holds it: {batches:?}"
        );

        let closed = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: user_uri.clone(),
            },
        };
        let (_, cleared) = within(futures::future::join(
            backend.did_close(closed),
            collect(&mut socket, 2),
        ))
        .await;

        for params in &cleared {
            assert!(params.diagnostics.is_empty(), "{params:?}");
        }
        let cleared: Vec<_> = cleared.into_iter().map(|params| params.uri).collect();
        assert!(cleared.contains(&user_uri));
        assert!(
            cleared.contains(&shared_uri),
            "the imported file is not left with squiggles nobody owns"
        );
    }
}
