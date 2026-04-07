//! `ColumnMarker` — lightweight owned column identifier.

/// A lightweight column identifier for use in selection descriptors.
///
/// Stores owned `String` fields so callers can construct markers from
/// dynamic (runtime) data without resorting to `Box::leak`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMarker {
    /// Table name.
    pub table: String,
    /// Column name.
    pub name: String,
}

impl ColumnMarker {
    /// Create a new column marker.
    ///
    /// Accepts any type that implements `Into<String>`, so both
    /// `&str` literals and owned `String` values work without ceremony.
    pub fn new(table: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            name: name.into(),
        }
    }

    /// Returns the join-safe alias for this column.
    ///
    /// The alias uses the format "table__column" which is safe to use
    /// in queries with joins, preventing column name conflicts.
    ///
    /// # Example
    ///
    /// ```
    /// use nautilus_core::ColumnMarker;
    ///
    /// let marker = ColumnMarker::new("users", "id");
    /// assert_eq!(marker.alias(), "users__id");
    /// ```
    pub fn alias(&self) -> String {
        format!("{}__{}", self.table, self.name)
    }
}
