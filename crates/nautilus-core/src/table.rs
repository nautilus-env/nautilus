//! Table names, optionally qualified by the schema that owns them.

use std::fmt;

/// The physical name of a table, plus the schema it lives in when the schema is
/// not the connection's default one.
///
/// Only the *table position* of a statement — `FROM`, `JOIN`, `INSERT INTO`,
/// `UPDATE`, `DELETE FROM` — renders the schema. Column references keep using
/// the bare [`name`](Self::name), because every supported provider gives a
/// schema-qualified table the bare name as its implicit alias.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct TableName {
    /// Schema that owns the table, or `None` for the connection's default one.
    pub schema: Option<String>,
    /// Physical table name, never schema-qualified.
    pub name: String,
}

impl TableName {
    /// Builds an unqualified name, resolved against the connection's default schema.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: None,
            name: name.into(),
        }
    }

    /// Builds a name qualified by `schema`.
    pub fn qualified(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            schema: Some(schema.into()),
            name: name.into(),
        }
    }

    /// Builds a name from an optional schema.
    pub fn with_schema(schema: Option<impl Into<String>>, name: impl Into<String>) -> Self {
        Self {
            schema: schema.map(Into::into),
            name: name.into(),
        }
    }

    /// The bare table name, which is also how column references address it.
    pub fn as_str(&self) -> &str {
        &self.name
    }

    /// The schema qualifier, when there is one.
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Whether the name carries a schema qualifier.
    pub fn is_qualified(&self) -> bool {
        self.schema.is_some()
    }

    /// Whether the name is empty, which builders reject.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }
}

impl fmt::Display for TableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.schema {
            Some(schema) => write!(f, "{schema}.{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

impl From<&str> for TableName {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for TableName {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

impl From<&String> for TableName {
    fn from(name: &String) -> Self {
        Self::new(name.clone())
    }
}

impl From<&TableName> for TableName {
    fn from(name: &TableName) -> Self {
        name.clone()
    }
}

impl PartialEq<str> for TableName {
    fn eq(&self, other: &str) -> bool {
        self.schema.is_none() && self.name == other
    }
}

impl PartialEq<&str> for TableName {
    fn eq(&self, other: &&str) -> bool {
        self.schema.is_none() && self.name == *other
    }
}

impl PartialEq<String> for TableName {
    fn eq(&self, other: &String) -> bool {
        self.schema.is_none() && &self.name == other
    }
}

impl PartialEq<TableName> for &str {
    fn eq(&self, other: &TableName) -> bool {
        other == self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unqualified_name_displays_bare() {
        assert_eq!(TableName::new("users").to_string(), "users");
    }

    #[test]
    fn qualified_name_displays_dotted() {
        assert_eq!(
            TableName::qualified("analytics", "events").to_string(),
            "analytics.events"
        );
    }

    #[test]
    fn qualified_name_never_equals_a_bare_string() {
        assert!(TableName::new("events") == "events");
        assert!(TableName::qualified("analytics", "events") != "events");
        assert!(TableName::qualified("analytics", "events") != "analytics.events");
    }

    #[test]
    fn same_table_in_two_schemas_hashes_apart() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(TableName::qualified("analytics", "events"));
        set.insert(TableName::qualified("public", "events"));
        set.insert(TableName::new("events"));
        assert_eq!(set.len(), 3);
    }
}
