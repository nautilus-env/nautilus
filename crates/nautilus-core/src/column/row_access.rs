//! `RowAccess` trait for abstracting row data access with lifetime support.

use crate::value::Value;

/// Trait for abstracting row data access with lifetime support.
///
/// This trait allows both borrowed and owned row implementations,
/// enabling database-specific optimizations while maintaining a common interface.
pub trait RowAccess<'row> {
    /// Get a value by column position (0-indexed).
    ///
    /// Returns `None` if the position is out of bounds.
    fn get_by_pos(&'row self, idx: usize) -> Option<&'row Value>;

    /// Get a value by column name.
    ///
    /// Returns `None` if the column doesn't exist.
    fn get(&'row self, name: &str) -> Option<&'row Value>;

    /// Get the column name at the given position.
    ///
    /// Returns `None` if the position is out of bounds.
    fn column_name(&'row self, idx: usize) -> Option<&'row str>;

    /// Returns the number of columns in the row.
    fn len(&self) -> usize;

    /// Returns true if the row contains no columns.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
