//! Conversion helpers between nautilus-schema byte offsets and LSP UTF-16
//! line/character positions.

use nautilus_schema::{
    analysis::{CompletionItem, CompletionKind, HoverInfo, SemanticKind, SemanticToken},
    diagnostic::{Diagnostic, Severity},
    Span,
};
use tower_lsp::lsp_types::{
    self, CompletionItemKind, DiagnosticSeverity, InsertTextFormat, Position, Range,
    SemanticToken as LspSemanticToken,
};

/// Convert a byte `offset` in `source` to an LSP [`Position`].
///
/// The returned position is 0-indexed (line, character).
pub fn offset_to_position(source: &str, offset: usize) -> Position {
    let safe = offset.min(source.len());
    let mut line = 0u32;
    let mut character = 0u32;

    for (idx, ch) in source.char_indices() {
        if idx >= safe {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

/// Convert an LSP [`Position`] to a byte offset in `source`.
///
/// Clamps to `source.len()` if the position is past the end.
pub fn position_to_offset(source: &str, pos: Position) -> usize {
    let mut current_line = 0u32;
    let mut line_start = 0usize;

    for (i, ch) in source.char_indices() {
        if current_line == pos.line {
            break;
        }
        if ch == '\n' {
            current_line += 1;
            line_start = i + ch.len_utf8();
        }
    }

    if current_line != pos.line {
        return source.len();
    }

    let mut utf16_col = 0u32;
    for (rel_idx, ch) in source[line_start..].char_indices() {
        let abs_idx = line_start + rel_idx;

        if ch == '\n' {
            return abs_idx;
        }

        let next_utf16_col = utf16_col + ch.len_utf16() as u32;
        if pos.character <= utf16_col || pos.character < next_utf16_col {
            return abs_idx;
        }
        if pos.character == next_utf16_col {
            return abs_idx + ch.len_utf8();
        }

        utf16_col = next_utf16_col;
    }

    source.len()
}

pub fn span_to_range(source: &str, span: &Span) -> Range {
    Range {
        start: offset_to_position(source, span.start),
        end: offset_to_position(source, span.end),
    }
}

pub fn nautilus_diagnostic_to_lsp(source: &str, d: &Diagnostic) -> lsp_types::Diagnostic {
    let severity = match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    };
    lsp_types::Diagnostic {
        range: span_to_range(source, &d.span),
        severity: Some(severity),
        message: d.message.clone(),
        source: Some("nautilus-schema".to_string()),
        ..Default::default()
    }
}

pub fn nautilus_completion_to_lsp(item: &CompletionItem) -> lsp_types::CompletionItem {
    let kind = match item.kind {
        CompletionKind::Keyword => CompletionItemKind::KEYWORD,
        CompletionKind::Type => CompletionItemKind::CLASS,
        CompletionKind::FieldAttribute => CompletionItemKind::PROPERTY,
        CompletionKind::ModelAttribute => CompletionItemKind::PROPERTY,
        CompletionKind::ModelName => CompletionItemKind::STRUCT,
        CompletionKind::EnumName => CompletionItemKind::ENUM,
        CompletionKind::FieldName => CompletionItemKind::FIELD,
    };
    lsp_types::CompletionItem {
        label: item.label.clone(),
        kind: Some(kind),
        detail: item.detail.clone(),
        insert_text: item.insert_text.clone(),
        insert_text_format: if item.is_snippet {
            Some(InsertTextFormat::SNIPPET)
        } else {
            None
        },
        ..Default::default()
    }
}

pub fn hover_info_to_lsp(source: &str, h: &HoverInfo) -> lsp_types::Hover {
    let range = h.span.as_ref().map(|s| span_to_range(source, s));
    lsp_types::Hover {
        contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: h.content.clone(),
        }),
        range,
    }
}

/// Encode a sorted list of [`SemanticToken`]s into the LSP delta format.
///
/// Token types legend (must match `SemanticTokensLegend` in `initialize`):
/// - `0` -> `nautilusModel`        (model reference)
/// - `1` -> `nautilusEnum`         (enum reference)
/// - `2` -> `nautilusCompositeType` (composite type reference)
pub fn semantic_tokens_to_lsp(source: &str, tokens: &[SemanticToken]) -> Vec<LspSemanticToken> {
    let mut result = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for token in tokens {
        let pos = offset_to_position(source, token.span.start);
        let length = (token.span.end - token.span.start) as u32;

        let delta_line = pos.line - prev_line;
        let delta_start = if delta_line == 0 {
            pos.character - prev_start
        } else {
            pos.character
        };

        let token_type = match token.kind {
            SemanticKind::ModelRef => 0,
            SemanticKind::EnumRef => 1,
            SemanticKind::CompositeTypeRef => 2,
        };

        result.push(LspSemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: 0,
        });

        prev_line = pos.line;
        prev_start = pos.character;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_offset_position() {
        let source = "model User {\n  id Int\n  name String\n}";
        let pos = offset_to_position(source, 13);
        assert_eq!(
            pos,
            Position {
                line: 1,
                character: 0
            }
        );
        let back = position_to_offset(source, pos);
        assert_eq!(back, 13);
    }

    #[test]
    fn offset_to_position_at_col() {
        let source = "model User {\n  id Int\n}";
        let pos = offset_to_position(source, 5);
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 5
            }
        );
    }

    #[test]
    fn utf16_positions_handle_astral_chars() {
        let source = "model User {\n  note String @default(\"hi 😀\")\n}\n";
        let emoji_offset = source.find("😀").unwrap();
        let pos = offset_to_position(source, emoji_offset);
        assert_eq!(
            pos,
            Position {
                line: 1,
                character: 27
            }
        );
        assert_eq!(position_to_offset(source, pos), emoji_offset);
    }

    #[test]
    fn position_to_offset_clamps_past_end_line() {
        let source = "model User {\n  id Int\n}\n";
        assert_eq!(
            position_to_offset(
                source,
                Position {
                    line: 99,
                    character: 0
                }
            ),
            source.len()
        );
    }

    #[test]
    fn position_to_offset_clamps_past_end_column_to_line_end() {
        let source = "name 😀\nnext";
        let line_end = source.find('\n').unwrap();
        assert_eq!(
            position_to_offset(
                source,
                Position {
                    line: 0,
                    character: 99
                }
            ),
            line_end
        );
    }
}
