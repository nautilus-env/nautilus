//! What the server advertises to the editor at `initialize`.

use tower_lsp::lsp_types::{
    CompletionOptions, HoverProviderCapability, OneOf, SaveOptions, SemanticTokenType,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensServerCapabilities, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions,
};

/// The capabilities of [`crate::backend::Backend`].
///
/// Saves carry the text so that a document reanalysed on save does not depend
/// on the cache, and the trigger characters cover attributes, attribute
/// arguments, and both separators of an import path.
pub(crate) fn server_capabilities() -> ServerCapabilities {
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
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
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
            },
        )),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::server_capabilities;

    #[test]
    fn server_capabilities_match_documented_triggers_and_formatting() {
        let caps = server_capabilities();
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
}
