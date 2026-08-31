//! LSP [`LanguageServer`] implementation for nautilus schemas.
//!
//! All schema intelligence (parse, validate, complete, hover, goto-definition)
//! lives in `nautilus-schema`; this module is pure glue.
//!
//! A document is never analysed alone when it has a path: it is assembled with
//! the files it imports (see [`crate::workspace`]) so that cross-file
//! references resolve, and the diagnostics of that assembled schema are
//! published to the file each one came from.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentFormattingParams,
    FileSystemWatcher, GlobPattern, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams,
    Location, MessageType, OneOf, Registration, SaveOptions, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer};

use crate::convert::{
    hover_info_to_lsp_with_index, nautilus_completion_to_lsp_with_index,
    nautilus_diagnostic_to_lsp_with_index, offset_to_position_with_index,
    position_to_offset_with_index, span_to_range_with_index,
};
use crate::document::DocumentState;
use crate::import_completion::import_path_completions;
use crate::workspace::{canonical, file_path_from_uri, Workspace};

/// The LSP backend.  Holds the client handle and the per-document cache.
pub struct Backend {
    pub client: Client,
    pub docs: DashMap<Url, DocumentState>,
    /// Files each open document last published diagnostics for, so that a file
    /// dropping out of a schema has its squiggles cleared.
    pub published: DashMap<Url, Vec<Url>>,
}

impl Backend {
    /// Re-run analysis on `source`, store the result, publish diagnostics, and
    /// refresh the documents that import this one.
    async fn reanalyze(&self, uri: Url, source: String) {
        if self
            .docs
            .get(&uri)
            .is_some_and(|existing| existing.source == source)
        {
            return;
        }

        self.reanalyze_only(uri.clone(), source).await;
        self.refresh_related(&uri).await;
    }

    /// Analyze one document and publish its diagnostics, without touching the
    /// documents that import it.
    async fn reanalyze_only(&self, uri: Url, source: String) {
        let state = self.analyze_document(&uri, source);
        self.publish(&uri, &state).await;
        self.docs.insert(uri, state);
    }

    /// Build the state for `uri`, assembling the schema it belongs to when it
    /// names a file on disk.
    fn analyze_document(&self, uri: &Url, source: String) -> DocumentState {
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

    /// The path of an open document whose schema already includes `uri`.
    fn importing_root(&self, uri: &Url) -> Option<(PathBuf, Url)> {
        self.docs.iter().find_map(|entry| {
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
        self.docs
            .iter()
            .filter_map(|entry| {
                let path = file_path_from_uri(entry.key())?;
                Some((canonical(&path), entry.value().source.clone()))
            })
            .collect()
    }

    /// Publish the diagnostics of `state`, one batch per file of its schema.
    async fn publish(&self, uri: &Url, state: &DocumentState) {
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
                self.client
                    .publish_diagnostics(stale, Vec::new(), None)
                    .await;
            }
        }
    }

    /// Whether a document other than `except` still reports diagnostics for
    /// `uri`.
    fn is_covered(&self, uri: &Url, except: &Url) -> bool {
        self.published
            .iter()
            .any(|entry| entry.key() != except && entry.value().contains(uri))
    }

    /// Re-analyze every open document that shares a schema with `uri`.
    ///
    /// Editing a file changes the meaning of the files that import it, and of
    /// the files it imports — those documents are now looking at a different
    /// schema, and their squiggles have to move with it.
    async fn refresh_related(&self, uri: &Url) {
        let covered: Vec<Url> = self
            .docs
            .get(uri)
            .and_then(|state| {
                state
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.uris().cloned().collect())
            })
            .unwrap_or_default();

        let affected: Vec<(Url, String)> = self
            .docs
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
            .collect();

        for (uri, source) in affected {
            self.reanalyze_only(uri, source).await;
        }
    }

    /// Re-analyze every open document, used when a file changed on disk outside
    /// the editor.
    async fn refresh_all(&self) {
        let open: Vec<(Url, String)> = self
            .docs
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().source.clone()))
            .collect();

        for (uri, source) in open {
            self.reanalyze_only(uri, source).await;
        }
    }

    fn server_capabilities() -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::INCREMENTAL),
                    save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                        include_text: Some(true),
                    })),
                    ..Default::default()
                },
            )),
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(vec![
                    "@".to_string(),
                    "=".to_string(),
                    "\"".to_string(),
                    "/".to_string(),
                    "\\".to_string(),
                ]),
                ..Default::default()
            }),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                    legend: SemanticTokensLegend {
                        token_types: vec![
                            SemanticTokenType::from("nautilusModel"),
                            SemanticTokenType::from("nautilusEnum"),
                            SemanticTokenType::from("nautilusCompositeType"),
                        ],
                        token_modifiers: vec![],
                    },
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                    ..Default::default()
                }),
            ),
            ..Default::default()
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: Self::server_capabilities(),
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
            .docs
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
        self.docs.remove(&uri);
        let covered = self
            .published
            .remove(&uri)
            .map(|(_, covered)| covered)
            .unwrap_or_default();
        self.client
            .publish_diagnostics(uri.clone(), Vec::new(), None)
            .await;

        for stale in covered {
            if stale != uri && !self.is_covered(&stale, &uri) {
                self.client
                    .publish_diagnostics(stale, Vec::new(), None)
                    .await;
            }
        }
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
        } else if let Some(state) = self.docs.get(&uri) {
            let source = state.source.clone();
            drop(state);
            self.reanalyze(uri, source).await;
        }
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let Some(state) = self.docs.get(uri) else {
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

        let Some(state) = self.docs.get(uri) else {
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

        let Some(state) = self.docs.get(uri) else {
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
        let Some(state) = self.docs.get(uri) else {
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
        let Some(state) = self.docs.get(uri) else {
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
    use super::Backend;
    use dashmap::DashMap;
    use futures::StreamExt;
    use tower_lsp::jsonrpc::Request;
    use tower_lsp::lsp_types::{
        CompletionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
        DidSaveTextDocumentParams, GotoDefinitionParams, Position, PublishDiagnosticsParams, Range,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
        TextDocumentPositionParams, VersionedTextDocumentIdentifier,
    };
    use tower_lsp::{LanguageServer, LspService};
    use tower_service::Service;

    #[test]
    fn server_capabilities_match_documented_triggers_and_formatting() {
        let caps = Backend::server_capabilities();
        let completion = caps.completion_provider.expect("completion provider");
        let triggers = completion.trigger_characters.expect("trigger characters");
        assert_eq!(triggers, vec!["@", "=", "\"", "/", "\\"]);
        let sync = caps.text_document_sync.expect("text sync");
        let tower_lsp::lsp_types::TextDocumentSyncCapability::Options(sync) = sync else {
            panic!("expected text sync options");
        };
        assert_eq!(
            sync.change,
            Some(tower_lsp::lsp_types::TextDocumentSyncKind::INCREMENTAL)
        );
        assert_eq!(
            caps.document_formatting_provider,
            Some(tower_lsp::lsp_types::OneOf::Left(true))
        );
    }

    fn schema_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nautilus-lsp-backend-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn file_uri(path: &std::path::Path) -> tower_lsp::lsp_types::Url {
        tower_lsp::lsp_types::Url::from_file_path(
            std::fs::canonicalize(path).expect("canonical path"),
        )
        .expect("file uri")
    }

    #[cfg(windows)]
    fn vscode_file_uri(path: &std::path::Path) -> tower_lsp::lsp_types::Url {
        let mut serialized = file_uri(path).to_string();
        let path_start = serialized.find(":///").expect("file URI path") + 4;
        let drive_colon = path_start
            + serialized[path_start..]
                .find(':')
                .expect("Windows drive colon");
        serialized.replace_range(drive_colon..=drive_colon, "%3A");
        tower_lsp::lsp_types::Url::parse(&serialized).expect("VS Code file URI")
    }

    #[cfg(not(windows))]
    fn vscode_file_uri(path: &std::path::Path) -> tower_lsp::lsp_types::Url {
        file_uri(path)
    }

    #[tokio::test]
    async fn an_imported_file_answers_completion_and_definition_across_files() {
        let dir = schema_dir("imports");
        let enums = dir.join("enums.nautilus");
        std::fs::write(&enums, "enum Role {\n  USER\n}\n").expect("write enums");
        let user = dir.join("user.nautilus");
        let user_source =
            "import \"./enums.nautilus\"\n\nmodel User {\n  id   Int  @id\n  role Role\n}\n";
        std::fs::write(&user, user_source).expect("write user");

        let (service, _socket) = LspService::new(|client| Backend {
            client,
            docs: DashMap::new(),
            published: DashMap::new(),
        });
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

        let state = backend.docs.get(&uri).expect("cached document");
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

        let (service, _socket) = LspService::new(|client| Backend {
            client,
            docs: DashMap::new(),
            published: DashMap::new(),
        });
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
    async fn changing_to_a_missing_import_publishes_a_diagnostic() {
        let dir = schema_dir("missing-import-diagnostic");
        let schema = dir.join("schema.nautilus");
        let initial = "model User {\r\n  id Int @id\r\n}\r\n";
        std::fs::write(&schema, initial).expect("write schema");

        let (mut service, mut socket) = LspService::new(|client| Backend {
            client,
            docs: DashMap::new(),
            published: DashMap::new(),
        });
        service
            .call(
                Request::build("initialize")
                    .id(1)
                    .params(serde_json::json!({ "capabilities": {} }))
                    .finish(),
            )
            .await
            .expect("initialize service")
            .expect("initialize response");
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

        let published = socket.next().await.expect("changed diagnostics");
        assert_eq!(published.method(), "textDocument/publishDiagnostics");
        let params: PublishDiagnosticsParams =
            serde_json::from_value(published.params().cloned().expect("diagnostic params"))
                .expect("valid diagnostic params");
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

    #[tokio::test]
    async fn editing_an_imported_buffer_refreshes_the_file_that_imports_it() {
        let dir = schema_dir("refresh");
        let enums = dir.join("enums.nautilus");
        std::fs::write(&enums, "enum Role {\n  USER\n}\n").expect("write enums");
        let user = dir.join("user.nautilus");
        let user_source = "import \"./enums.nautilus\"\n\nmodel User {\n  id     Int    @id\n  status Status\n}\n";
        std::fs::write(&user, user_source).expect("write user");

        let (service, _socket) = LspService::new(|client| Backend {
            client,
            docs: DashMap::new(),
            published: DashMap::new(),
        });
        let backend = service.inner();
        let user_uri = file_uri(&user);
        let enums_uri = file_uri(&enums);

        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: user_uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: user_source.to_string(),
                },
            })
            .await;

        let unknown_status = |backend: &Backend| {
            let state = backend.docs.get(&user_uri).expect("cached document");
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
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: enums_uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: "enum Role {\n  USER\n}\n\nenum Status {\n  ACTIVE\n}\n".to_string(),
                },
            })
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

        let (service, _socket) = LspService::new(|client| Backend {
            client,
            docs: DashMap::new(),
            published: DashMap::new(),
        });
        let backend = service.inner();

        let post_uri = file_uri(&post);
        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: post_uri.clone(),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: post_source.to_string(),
                },
            })
            .await;

        let has_unknown_user = |backend: &Backend| {
            let state = backend.docs.get(&post_uri).expect("cached document");
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
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: file_uri(&user),
                    language_id: "nautilus".to_string(),
                    version: 1,
                    text: user_source.to_string(),
                },
            })
            .await;

        assert!(
            !has_unknown_user(backend),
            "once the importing file is open, post.nautilus is read as part of that schema"
        );
    }

    #[tokio::test]
    async fn untitled_documents_are_cached_and_serve_requests() {
        let (service, _socket) = LspService::new(|client| Backend {
            client,
            docs: DashMap::new(),
            published: DashMap::new(),
        });
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

        let state = backend.docs.get(&uri).expect("cached untitled document");
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
            .docs
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
