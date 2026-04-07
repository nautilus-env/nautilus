//! Shared row stream type for all database backends.
//!
//! All three backends (PostgreSQL, MySQL, SQLite) use the single [`RowStream`] type
//! rather than three separate structs. Per-backend type aliases are provided to keep
//! the public API stable and code at call sites readable.

use crate::error::Result;
use crate::Row;
use futures::stream::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A type-erased async stream of [`Row`] values.
///
/// Current connector implementations eagerly fetch database results and then
/// expose those rows through this stream interface. The inner stream is
/// heap-allocated and pinned, which is why implementing `Unpin` is safe here.
pub struct RowStream {
    inner: Pin<Box<dyn Stream<Item = Result<Row>> + Send>>,
}

impl RowStream {
    /// Create a new `RowStream` wrapping a boxed async stream.
    pub(crate) fn new_from_stream(stream: Pin<Box<dyn Stream<Item = Result<Row>> + Send>>) -> Self {
        Self { inner: stream }
    }
}

/// Delegates `poll_next` to the inner boxed stream.
impl Stream for RowStream {
    type Item = Result<Row>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// `RowStream` is `Unpin` because the inner stream is heap-allocated and pinned.
impl Unpin for RowStream {}
