//! Row access abstracted over the lifetime of the underlying buffer.

use crate::value::Value;

/// Reads values out of a database row by position or by name.
///
/// The `'row` lifetime lets an implementation hand back a reference into a
/// buffer it still owns, so a backend need not copy every value to satisfy
/// the common interface.
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
