//! Per-document state cached by the LSP server.
//!
//! Every time the client sends a `didOpen`, `didChange`, or `didSave`
//! notification the server updates the cached source, re-runs analysis only
//! when the content actually changed, and stores a fresh [`DocumentState`].
//! Subsequent `textDocument/completion`, `hover`, `definition`,
//! `semanticTokens/full`, and formatting requests read from this cache rather
//! than re-analysing.
//!
//! A document that imports other files carries a [`Workspace`] as well: names
//! are resolved against the whole assembled schema, while semantic tokens and
//! formatting stay strictly about the text of this one file.

use std::sync::Arc;
use std::sync::OnceLock;

use crate::convert::{position_to_offset_with_index, semantic_tokens_to_lsp_with_index};
use crate::workspace::Workspace;
use nautilus_schema::{
    analysis::{
        analyze, completion_with_analysis, goto_definition_with_analysis, hover_with_analysis,
        semantic_tokens, AnalysisResult, CompletionItem, HoverInfo,
    },
    format_schema, LineIndex, Span,
};
use tower_lsp::lsp_types::{
    SemanticToken as LspSemanticToken, TextDocumentContentChangeEvent, Url,
};

/// Snapshot of a single `.nautilus` document.
pub struct DocumentState {
    /// Full text of the document as last received from the client.
    pub source: String,
    /// Cached line offsets for fast span/position conversion.
    pub line_index: LineIndex,
    /// Analysis result produced from `source` alone.
    pub analysis: AnalysisResult,
    /// The assembled schema this document belongs to, when it has a path.
    pub workspace: Option<Arc<Workspace>>,
    /// Offset of this document's text inside the workspace source.
    base: usize,
    semantic_tokens: OnceLock<Option<Vec<LspSemanticToken>>>,
    formatted: OnceLock<Option<String>>,
}

impl DocumentState {
    /// Analyze `source` on its own and build a new [`DocumentState`].
    pub fn new(source: String) -> Self {
        let line_index = LineIndex::new(&source);
        let analysis = analyze(&source);
        Self {
            source,
            line_index,
            analysis,
            workspace: None,
            base: 0,
            semantic_tokens: OnceLock::new(),
            formatted: OnceLock::new(),
        }
    }

    /// Analyze `source` and attach the assembled schema it belongs to, so that
    /// names declared in imported files resolve.
    ///
    /// `uri` names this document inside `workspace`; a document the workspace
    /// does not contain falls back to seeing only itself.
    pub fn with_workspace(source: String, uri: &Url, workspace: Arc<Workspace>) -> Self {
        let Some(base) = workspace.base_of(uri) else {
            return Self::new(source);
        };
        let mut state = Self::new(source);
        state.base = base;
        state.workspace = Some(workspace);
        state
    }

    /// The source every offset handed to the analysis functions refers to: the
    /// assembled schema when there is one, this document's text otherwise.
    fn analyzed_source(&self) -> &str {
        match &self.workspace {
            Some(workspace) => workspace.source(),
            None => &self.source,
        }
    }

    fn analyzed(&self) -> &AnalysisResult {
        match &self.workspace {
            Some(workspace) => workspace.analysis(),
            None => &self.analysis,
        }
    }

    /// Apply a batch of LSP content changes to the cached source text.
    pub fn apply_content_changes(&self, changes: &[TextDocumentContentChangeEvent]) -> String {
        if changes.is_empty() {
            return self.source.clone();
        }

        let mut source = self.source.clone();
        let mut line_index = self.line_index.clone();

        for change in changes {
            if let Some(range) = change.range {
                let start = position_to_offset_with_index(&source, &line_index, range.start);
                let end = position_to_offset_with_index(&source, &line_index, range.end);
                source.replace_range(start..end, &change.text);
            } else {
                source = change.text.clone();
            }
            line_index = LineIndex::new(&source);
        }

        source
    }

    /// Completion items derived from the cached analysis.
    pub fn completion(&self, offset: usize) -> Vec<CompletionItem> {
        completion_with_analysis(self.analyzed_source(), self.analyzed(), self.base + offset)
    }

    /// Hover info derived from the cached analysis, with its span rebased onto
    /// this document.
    pub fn hover(&self, offset: usize) -> Option<HoverInfo> {
        let mut info =
            hover_with_analysis(self.analyzed_source(), self.analyzed(), self.base + offset)?;
        info.span = info.span.and_then(|span| self.local_span(span));
        Some(info)
    }

    /// Rebase a span of the analysed source onto this document, dropping one
    /// that points outside it.
    fn local_span(&self, span: Span) -> Option<Span> {
        let end = self.base + self.source.len();
        if span.start < self.base || span.end > end {
            return None;
        }
        Some(Span::new(span.start - self.base, span.end - self.base))
    }

    /// Definition span derived from the cached analysis.
    ///
    /// The span is an offset into [`Workspace::source`] when the document has a
    /// workspace, because the definition may live in an imported file; the
    /// caller resolves it with [`Workspace::locate`].
    pub fn goto_definition(&self, offset: usize) -> Option<Span> {
        goto_definition_with_analysis(self.analyzed(), self.base + offset)
    }

    /// Semantic tokens derived from the cached analysis and memoized per
    /// document version.
    pub fn semantic_tokens(&self) -> Option<&[LspSemanticToken]> {
        self.semantic_tokens
            .get_or_init(|| {
                let ast = self.analysis.ast.as_ref()?;
                let tokens = semantic_tokens(ast, &self.analysis.tokens);
                Some(semantic_tokens_to_lsp_with_index(
                    &self.source,
                    &self.line_index,
                    &tokens,
                ))
            })
            .as_deref()
    }

    /// Canonical formatted source derived from the cached AST.
    pub fn formatted(&self) -> Option<&str> {
        self.formatted
            .get_or_init(|| {
                self.analysis
                    .ast
                    .as_ref()
                    .map(|ast| format_schema(ast, &self.source))
            })
            .as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::DocumentState;
    use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent};

    #[test]
    fn cached_analysis_serves_completion_hover_and_definition() {
        let source = r#"
type Address {
  kind Role
}

enum Role {
  Home
  Work
}

model User {
  id      Int @id
  address 
}
"#;
        let state = DocumentState::new(source.to_string());

        let completion_offset = source.find("address ").unwrap() + "address ".len();
        let completion_labels: Vec<_> = state
            .completion(completion_offset)
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert!(completion_labels.iter().any(|label| label == "Address"));

        let hover_offset = source.find("kind").unwrap() + 1;
        let hover = state.hover(hover_offset).expect("hover");
        assert!(hover.content.contains("Role"));

        let definition_offset = source.find("kind Role").unwrap() + "kind ".len() + 1;
        let definition = state
            .goto_definition(definition_offset)
            .expect("definition");
        assert!(source[definition.start..definition.end].contains("Role"));
    }

    #[test]
    fn formatted_uses_cached_ast() {
        let source = "model User {\nname String\nid Int @id\n}\n";
        let state = DocumentState::new(source.to_string());
        let formatted = state.formatted().expect("formatted source");
        assert!(formatted.contains("name String"));
        assert_ne!(formatted, source);
        let formatted_again = state.formatted().expect("cached formatted source");
        assert!(std::ptr::eq(formatted.as_ptr(), formatted_again.as_ptr()));
    }

    #[test]
    fn incremental_changes_are_applied_against_cached_source() {
        let source = "model User {\n  role \n}\n";
        let state = DocumentState::new(source.to_string());
        let updated = state.apply_content_changes(&[TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(3, 0), Position::new(3, 0))),
            range_length: None,
            text: "enum Role {\n  Member\n}\n".to_string(),
        }]);

        assert_eq!(
            updated,
            "model User {\n  role \n}\nenum Role {\n  Member\n}\n"
        );
    }

    #[test]
    fn semantic_tokens_are_cached_per_document_version() {
        let source = r#"
enum Role {
  MEMBER
}

model User {
  id   Int  @id
  role Role
}
"#;
        let state = DocumentState::new(source.to_string());
        let first = state.semantic_tokens().expect("semantic tokens");
        assert_eq!(first.len(), 1);
        let second = state.semantic_tokens().expect("cached semantic tokens");
        assert!(std::ptr::eq(first.as_ptr(), second.as_ptr()));
    }
}
