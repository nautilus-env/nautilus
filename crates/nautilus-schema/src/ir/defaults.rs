//! The default value a field carries, whether written as a literal or as a
//! function the database evaluates.

/// Default value for a field.
#[derive(Debug, Clone, PartialEq)]
pub enum DefaultValue {
    /// A literal string value.
    String(String),
    /// A literal number value (stored as string to preserve precision).
    Number(String),
    /// A literal boolean value.
    Boolean(bool),
    /// An array literal value.
    Array(Vec<DefaultValue>),
    /// An enum variant name.
    EnumVariant(String),
    /// A function call (autoincrement, uuid, uuidv7, now, etc.).
    Function(FunctionCall),
}

/// Function call in a default value.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    /// The function name (e.g., "autoincrement", "uuid", "uuidv7", "now").
    pub name: String,
    /// Function arguments (if any).
    pub args: Vec<String>,
}

impl FunctionCall {
    /// Returns `true` when this default value is supplied by the database on insert.
    pub fn is_database_generated_default(&self) -> bool {
        matches!(
            self.name.as_str(),
            "autoincrement" | "uuid" | "uuidv7" | "now"
        )
    }

    /// Returns `true` for UUID-generating defaults.
    pub fn is_uuid_default(&self) -> bool {
        matches!(self.name.as_str(), "uuid" | "uuidv7")
    }
}
