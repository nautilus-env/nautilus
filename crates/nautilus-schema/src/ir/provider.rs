//! The providers a schema names: the database a datasource points at, and the
//! client a generator emits, each parsed from the string written in the block.

use std::fmt;
use std::str::FromStr;

/// The three datasource providers recognised by the Nautilus schema language.
///
/// Obtained by parsing the `provider` field of a `datasource` block:
/// ```text
/// datasource db {
///     provider = "postgresql"  // -> DatabaseProvider::Postgres
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseProvider {
    /// PostgreSQL (provider string: `"postgresql"`).
    Postgres,
    /// MySQL / MariaDB (provider string: `"mysql"`).
    Mysql,
    /// SQLite (provider string: `"sqlite"`).
    Sqlite,
}

/// Error returned when parsing an unknown database provider string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDatabaseProviderError;

impl fmt::Display for ParseDatabaseProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown database provider")
    }
}

impl std::error::Error for ParseDatabaseProviderError {}

impl FromStr for DatabaseProvider {
    type Err = ParseDatabaseProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "postgresql" => Ok(DatabaseProvider::Postgres),
            "mysql" => Ok(DatabaseProvider::Mysql),
            "sqlite" => Ok(DatabaseProvider::Sqlite),
            _ => Err(ParseDatabaseProviderError),
        }
    }
}

impl DatabaseProvider {
    /// All valid datasource provider strings.
    pub const ALL: &'static [&'static str] = &["postgresql", "mysql", "sqlite"];

    /// The canonical provider string used in `.nautilus` schema files.
    pub fn as_str(self) -> &'static str {
        match self {
            DatabaseProvider::Postgres => "postgresql",
            DatabaseProvider::Mysql => "mysql",
            DatabaseProvider::Sqlite => "sqlite",
        }
    }

    /// Human-readable display name (for diagnostic messages).
    pub fn display_name(self) -> &'static str {
        match self {
            DatabaseProvider::Postgres => "PostgreSQL",
            DatabaseProvider::Mysql => "MySQL",
            DatabaseProvider::Sqlite => "SQLite",
        }
    }

    /// Returns `true` when the provider has a native UUIDv7 default function.
    pub fn supports_uuidv7_default(self) -> bool {
        matches!(self, DatabaseProvider::Postgres)
    }
}

impl std::fmt::Display for DatabaseProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The generator (client) providers recognised by the Nautilus schema language.
///
/// Obtained by parsing the `provider` field of a `generator` block:
/// ```text
/// generator client {
///     provider = "nautilus-client-rs"  // -> ClientProvider::Rust
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientProvider {
    /// Rust client (provider string: `"nautilus-client-rs"`).
    Rust,
    /// Python client (provider string: `"nautilus-client-py"`).
    Python,
    /// JavaScript/TypeScript client (provider string: `"nautilus-client-js"`).
    JavaScript,
    /// Java client (provider string: `"nautilus-client-java"`).
    Java,
}

/// Error returned when parsing an unknown client provider string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseClientProviderError;

impl fmt::Display for ParseClientProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown client provider")
    }
}

impl std::error::Error for ParseClientProviderError {}

impl FromStr for ClientProvider {
    type Err = ParseClientProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nautilus-client-rs" => Ok(ClientProvider::Rust),
            "nautilus-client-py" => Ok(ClientProvider::Python),
            "nautilus-client-js" => Ok(ClientProvider::JavaScript),
            "nautilus-client-java" => Ok(ClientProvider::Java),
            _ => Err(ParseClientProviderError),
        }
    }
}

impl ClientProvider {
    /// All valid generator provider strings.
    pub const ALL: &'static [&'static str] = &[
        "nautilus-client-rs",
        "nautilus-client-py",
        "nautilus-client-js",
        "nautilus-client-java",
    ];

    /// The canonical provider string used in `.nautilus` schema files.
    pub fn as_str(self) -> &'static str {
        match self {
            ClientProvider::Rust => "nautilus-client-rs",
            ClientProvider::Python => "nautilus-client-py",
            ClientProvider::JavaScript => "nautilus-client-js",
            ClientProvider::Java => "nautilus-client-java",
        }
    }
}

impl fmt::Display for ClientProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
