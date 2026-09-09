//! LSP [`LanguageServer`] implementation for nautilus schemas.
//!
//! All schema intelligence (parse, validate, complete, hover, goto-definition)
//! lives in `nautilus-schema`; this module is pure glue. It owns the sequence —
//! analyse, publish, refresh the documents that share the schema — while
//! [`crate::documents`] holds the cache and the assembly, and
//! [`crate::diagnostics`] decides which file each squiggle belongs to.

use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentFormattingParams, FileSystemWatcher, GlobPattern, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverParams, InitializeParams, InitializeResult,
    InitializedParams, Location, MessageType, Registration, SemanticTokens, SemanticTokensParams,
    SemanticTokensResult, ServerInfo, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer};

use crate::capabilities::server_capabilities;
use crate::convert::{
    hover_info_to_lsp_with_index, nautilus_completion_to_lsp_with_index,
    offset_to_position_with_index, position_to_offset_with_index, span_to_range_with_index,
};
use crate::diagnostics::Diagnostics;
use crate::documents::Documents;
use crate::import_completion::import_path_completions;
use crate::workspace::file_path_from_uri;

/// The LSP backend.  Holds the client handle and the per-document cache.
pub struct Backend {
    client: Client,
    pub(crate) documents: Documents,
    diagnostics: Diagnostics,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            diagnostics: Diagnostics::new(client.clone()),
            documents: Documents::new(),
            client,
        }
    }

    /// Re-run analysis on `source`, store the result, publish diagnostics, and
    /// refresh the documents that import this one.
    async fn reanalyze(&self, uri: Url, source: String) {
        if self.documents.is_current(&uri, &source) {
            return;
        }

        self.reanalyze_only(uri.clone(), source).await;
        for (related, source) in self.documents.related(&uri) {
            self.reanalyze_only(related, source).await;
        }
    }

    /// Analyze one document and publish its diagnostics, without touching the
    /// documents that import it.
    async fn reanalyze_only(&self, uri: Url, source: String) {
        let state = self.documents.analyze(&uri, source);
        self.diagnostics.publish(&uri, &state).await;
        self.documents.store(uri, state);
    }

    /// Re-analyze every open document, used when a file changed on disk outside
    /// the editor.
    async fn refresh_all(&self) {
        for (uri, source) in self.documents.open() {
            self.reanalyze_only(uri, source).await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: server_capabilities(),
            server_info: Some(ServerInfo {
                name: "nautilus-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        // Imported files are often not open in the editor, so the only notice
        // of them changing comes from the client watching the filesystem.
        let watcher = Registration {
            id: "nautilus-watch-schemas".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                watchers: vec![FileSystemWatcher {
                    glob_pattern: GlobPattern::String("**/*.nautilus".to_string()),
                    kind: None,
                }],
            })
            .ok(),
        };
        let _ = self.client.register_capability(vec![watcher]).await;

        self.client
            .log_message(MessageType::INFO, "nautilus-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        self.reanalyze(uri, params.text_document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let changes = params.content_changes;

        let Some(source) = self
            .documents
            .get(&uri)
            .map(|state| state.apply_content_changes(&changes))
            .or_else(|| {
                changes
                    .into_iter()
                    .next()
                    .filter(|change| change.range.is_none())
                    .map(|change| change.text)
            })
        else {
            return;
        };

        self.reanalyze(uri, source).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.close(&uri);
        self.diagnostics.clear(&uri).await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        if params.changes.is_empty() {
            return;
        }
        self.refresh_all().await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        // `include_text` is set to true in ServerCapabilities, so `text` is
        // always present.  Fall back to the cache only as a safety net.
        if let Some(text) = params.text {
            self.reanalyze(uri, text).await;
        } else if let Some(state) = self.documents.get(&uri) {
            let source = state.source.clone();
            drop(state);
            self.reanalyze(uri, source).await;
        }
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let Some(state) = self.documents.get(uri) else {
            return Ok(None);
        };
        let offset = position_to_offset_with_index(&state.source, &state.line_index, pos);
        if let Some(path) = file_path_from_uri(uri) {
            if let Some(items) =
                import_path_completions(&state.source, &state.line_index, offset, &path)
            {
                return Ok(Some(CompletionResponse::Array(items)));
            }
        }
        let items = state.completion(offset);
        let lsp_items: Vec<CompletionItem> = items
            .iter()
            .map(|item| {
                nautilus_completion_to_lsp_with_index(
                    &state.source,
                    &state.line_index,
                    &state.analysis.tokens,
                    offset,
                    item,
                )
            })
            .collect();

        Ok(Some(CompletionResponse::Array(lsp_items)))
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(state) = self.documents.get(uri) else {
            return Ok(None);
        };
        let offset = position_to_offset_with_index(&state.source, &state.line_index, pos);

        Ok(state
            .hover(offset)
            .as_ref()
            .map(|h| hover_info_to_lsp_with_index(&state.source, &state.line_index, h)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(state) = self.documents.get(uri) else {
            return Ok(None);
        };
        let offset = position_to_offset_with_index(&state.source, &state.line_index, pos);

        let Some(span) = state.goto_definition(offset) else {
            return Ok(None);
        };

        // With a workspace the span is an offset into the assembled schema, so
        // the definition may well live in an imported file.
        let location = match &state.workspace {
            Some(workspace) => workspace
                .locate(span)
                .map(|(uri, range)| Location { uri, range }),
            None => Some(Location {
                uri: uri.clone(),
                range: span_to_range_with_index(&state.source, &state.line_index, &span),
            }),
        };

        Ok(location.map(GotoDefinitionResponse::Scalar))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> LspResult<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let Some(state) = self.documents.get(uri) else {
            return Ok(None);
        };
        let Some(data) = state.semantic_tokens() else {
            return Ok(None);
        };

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: data.to_vec(),
        })))
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> LspResult<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let Some(state) = self.documents.get(uri) else {
            return Ok(None);
        };
        let Some(formatted) = state.formatted() else {
            return Ok(None);
        };
        if formatted == state.source {
            return Ok(Some(Vec::new()));
        }

        let edit = TextEdit {
            range: tower_lsp::lsp_types::Range {
                start: tower_lsp::lsp_types::Position::new(0, 0),
                end: offset_to_position_with_index(
                    &state.source,
                    &state.line_index,
                    state.source.len(),
                ),
            },
            new_text: formatted.to_string(),
        };

        Ok(Some(vec![edit]))
    }
}

#[cfg(test)]
mod tests {
    use tower_lsp::lsp_types::{
        CompletionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
        DidSaveTextDocumentParams, GotoDefinitionParams, Position, Range,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
        TextDocumentPositionParams, VersionedTextDocumentIdentifier,
    };
    use tower_lsp::LanguageServer;

    use crate::test_support::{file_uri, schema_dir, service, vscode_file_uri};

    #[tokio::test]
    async fn an_imported_file_answers_completion_and_definition_across_files() {
        let dir = schema_dir("imports");
        let enums = dir.join("enums.nautilus");
        std::fs::write(&enums, "enum Role {\n  USER\n}\n").expect("write enums");
        let user = dir.join("user.nautilus");
        let user_source =
            "import \"./enums.nautilus\"\n\nmodel User {\n  id   Int  @id\n  role Role\n}\n";
        std::fs::write(&user, user_source).expect("write user");

        let (service, _socket) = service();
        let backend = service.inner();
        let uri = file_uri(&user);

        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: user_source.to_string(),
                },
            })
            .await;

        let state = backend.documents.get(&uri).expect("cached document");
        let workspace = state.workspace.as_ref().expect("assembled workspace");
        assert!(
            workspace.diagnostics().iter().all(|(_, d)| d.is_empty()),
            "a resolved cross-file reference is not an error: {:?}",
            workspace.diagnostics()
        );
        drop(state);

        let role_line = user_source
            .lines()
            .position(|line| line.contains("role Role"))
            .expect("role field") as u32;
        let definition = backend
            .goto_definition(GotoDefinitionParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position: Position::new(role_line, 8),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .await
            .expect("definition result")
            .expect("definition payload");
        let tower_lsp::lsp_types::GotoDefinitionResponse::Scalar(location) = definition else {
            panic!("expected a single definition location");
        };
        assert_eq!(
            location.uri,
            file_uri(&enums),
            "the definition of Role lives in the imported file"
        );
        assert_eq!(location.range.start.line, 0);

        let completion = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position: Position::new(role_line, 7),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .expect("completion result")
            .expect("completion payload");
        let tower_lsp::lsp_types::CompletionResponse::Array(items) = completion else {
            panic!("expected completion array");
        };
        assert!(
            items.iter().any(|item| item.label == "Role"),
            "an imported enum is offered as a field type"
        );
    }

    #[tokio::test]
    async fn import_path_completion_lists_folders_and_nautilus_files() {
        let dir = schema_dir("import-path-completion");
        std::fs::create_dir(dir.join("domain")).expect("create domain");
        std::fs::write(dir.join("enums.nautilus"), "enum Role { USER }").expect("write enums");
        std::fs::write(dir.join("notes.txt"), "not a schema").expect("write notes");
        let schema = dir.join("schema.nautilus");
        let source = "import \"\"";
        std::fs::write(&schema, source).expect("write schema");

        let (service, _socket) = service();
        let backend = service.inner();
        let uri = vscode_file_uri(&schema);
        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: source.to_string(),
                },
            })
            .await;

        let completion = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: Position::new(0, 8),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .expect("completion result")
            .expect("completion payload");
        let tower_lsp::lsp_types::CompletionResponse::Array(items) = completion else {
            panic!("expected completion array");
        };

        assert!(items.iter().any(|item| {
            item.label == "domain/"
                && item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FOLDER)
        }));
        assert!(items.iter().any(|item| {
            item.label == "enums.nautilus"
                && item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FILE)
        }));
        assert!(!items.iter().any(|item| item.label == "notes.txt"));
    }

    #[tokio::test]
    async fn untitled_documents_are_cached_and_serve_requests() {
        let (service, _socket) = service();
        let backend = service.inner();
        let uri = tower_lsp::lsp_types::Url::parse("untitled:Untitled-1").expect("valid uri");

        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: "model User {\n  role \n}\n".to_string(),
                },
            })
            .await;

        let state = backend
            .documents
            .get(&uri)
            .expect("cached untitled document");
        assert_eq!(state.source, "model User {\n  role \n}\n");
        drop(state);

        let completion = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position: Position::new(1, 7),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .expect("completion result")
            .expect("completion payload");
        let tower_lsp::lsp_types::CompletionResponse::Array(items) = completion else {
            panic!("expected completion array");
        };
        assert!(
            items.iter().any(|item| item.label == "String"),
            "expected scalar completions for untitled document"
        );

        backend
            .did_change(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: Some(Range::new(Position::new(3, 0), Position::new(3, 0))),
                    range_length: None,
                    text: "enum Role {\n  Member\n}\n".to_string(),
                }],
            })
            .await;

        let state = backend
            .documents
            .get(&uri)
            .expect("updated untitled document remains cached");
        assert_eq!(
            state.source,
            "model User {\n  role \n}\nenum Role {\n  Member\n}\n"
        );
        drop(state);

        backend
            .did_save(DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                text: None,
            })
            .await;

        let completion = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position: Position::new(1, 7),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .expect("completion result after save")
            .expect("completion payload after save");
        let tower_lsp::lsp_types::CompletionResponse::Array(items) = completion else {
            panic!("expected completion array");
        };
        assert!(
            items.iter().any(|item| item.label == "Role"),
            "expected updated completions after save fallback for untitled document"
        );
    }
}
