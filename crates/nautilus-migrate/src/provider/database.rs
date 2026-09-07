/// Supported database providers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseProvider {
    /// PostgreSQL
    Postgres,
    /// SQLite
    Sqlite,
    /// MySQL
    Mysql,
}

impl DatabaseProvider {
    /// Parse a datasource `provider` string from a `.nautilus` schema.
    pub fn from_schema_provider(provider: &str) -> Option<Self> {
        match provider {
            "postgresql" | "postgres" => Some(Self::Postgres),
            "sqlite" => Some(Self::Sqlite),
            "mysql" => Some(Self::Mysql),
            _ => None,
        }
    }

    /// Return the canonical `.nautilus` datasource provider string.
    pub fn schema_provider_name(self) -> &'static str {
        match self {
            Self::Postgres => "postgresql",
            Self::Sqlite => "sqlite",
            Self::Mysql => "mysql",
        }
    }

    /// The delimiter this provider wraps identifiers in: double quotes for
    /// PostgreSQL and SQLite, backticks for MySQL.
    pub fn identifier_quote(self) -> char {
        match self {
            Self::Postgres | Self::Sqlite => '"',
            Self::Mysql => '`',
        }
    }

    /// Quote an identifier for this database provider, doubling any delimiter
    /// the name contains.
    pub fn quote_identifier(self, name: &str) -> String {
        nautilus_core::ident::quote_ident(name, self.identifier_quote())
    }

    /// Whether a rollback undoes the DDL a transaction has already run.
    ///
    /// PostgreSQL and SQLite keep DDL transactional. MySQL commits implicitly
    /// before and after most DDL, so a statement that succeeded there stays in
    /// place even when a later one in the same transaction fails.
    pub fn ddl_rolls_back(self) -> bool {
        !matches!(self, Self::Mysql)
    }

    /// Render the positional bind placeholder for the 1-based argument `index`
    /// in this provider's dialect.
    pub fn placeholder(self, index: usize) -> String {
        match self {
            Self::Postgres => format!("${index}"),
            Self::Mysql | Self::Sqlite => "?".to_string(),
        }
    }

    /// Render `count` comma-separated positional placeholders (1-based) for this
    /// provider, e.g. `$1, $2, $3` (PostgreSQL) or `?, ?, ?` (MySQL/SQLite).
    pub fn placeholders(self, count: usize) -> String {
        (1..=count)
            .map(|i| self.placeholder(i))
            .collect::<Vec<_>>()
            .join(", ")
    }
}
