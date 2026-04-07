//! Source code position and span tracking for diagnostics.

use std::fmt;

/// A position in source code (line and column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Line number (1-indexed).
    pub line: usize,
    /// Column number (1-indexed).
    pub column: usize,
}

impl Position {
    /// Create a new position.
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A span in source code (byte offsets).
///
/// Spans use byte offsets for efficient slicing. Line/column information
/// can be computed from the source text when needed for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the start of the span (inclusive).
    pub start: usize,
    /// Byte offset of the end of the span (exclusive).
    pub end: usize,
}

impl Span {
    /// Create a new span.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Create a span from a single byte offset.
    pub const fn single(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos + 1,
        }
    }

    /// Merge two spans into one that covers both.
    pub const fn merge(self, other: Span) -> Span {
        let start = if self.start < other.start {
            self.start
        } else {
            other.start
        };
        let end = if self.end > other.end {
            self.end
        } else {
            other.end
        };
        Span { start, end }
    }

    /// Get the length of the span in bytes.
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    /// Check if the span is empty.
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Extract the text covered by this span from source.
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }

    /// Convert byte offset span to line/column positions.
    ///
    /// This scans the source text to compute line and column numbers.
    /// For performance, avoid calling this repeatedly; cache results if needed.
    pub fn to_positions(&self, source: &str) -> (Position, Position) {
        let start_pos = byte_offset_to_position(source, self.start);
        let end_pos = byte_offset_to_position(source, self.end);
        (start_pos, end_pos)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// Convert a byte offset to a line/column position.
fn byte_offset_to_position(source: &str, offset: usize) -> Position {
    let mut line = 1;
    let mut column = 1;

    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    Position { line, column }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_display() {
        let pos = Position::new(10, 25);
        assert_eq!(pos.to_string(), "10:25");
    }

    #[test]
    fn test_span_merge() {
        let span1 = Span::new(5, 10);
        let span2 = Span::new(8, 15);
        let merged = span1.merge(span2);
        assert_eq!(merged, Span::new(5, 15));
    }

    #[test]
    fn test_span_len() {
        let span = Span::new(10, 20);
        assert_eq!(span.len(), 10);
    }

    #[test]
    fn test_span_slice() {
        let source = "hello world";
        let span = Span::new(0, 5);
        assert_eq!(span.slice(source), "hello");
    }

    #[test]
    fn test_byte_offset_to_position() {
        let source = "hello\nworld\nfoo";
        assert_eq!(byte_offset_to_position(source, 0), Position::new(1, 1));
        assert_eq!(byte_offset_to_position(source, 5), Position::new(1, 6));
        assert_eq!(byte_offset_to_position(source, 6), Position::new(2, 1));
        assert_eq!(byte_offset_to_position(source, 12), Position::new(3, 1));
    }

    #[test]
    fn test_span_to_positions() {
        let source = "hello\nworld";
        let span = Span::new(0, 5);
        let (start, end) = span.to_positions(source);
        assert_eq!(start, Position::new(1, 1));
        assert_eq!(end, Position::new(1, 6));
    }
}
