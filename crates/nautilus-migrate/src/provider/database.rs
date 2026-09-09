use nautilus_schema::ir::DatabaseProvider as SchemaDatabaseProvider;

/// Migration backend and its SQL capabilities. Converts to and from the schema's
/// provider identity without passing through a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseProvider {
    /// PostgreSQL
    Postgres,
    /// SQLite
    Sqlite,
    /// MySQL
    Mysql,
}

impl From<SchemaDatabaseProvider> for DatabaseProvider {
    fn from(provider: SchemaDatabaseProvider) -> Self {
        match provider {
            SchemaDatabaseProvider::Postgres => Self::Postgres,
            SchemaDatabaseProvider::Mysql => Self::Mysql,
            SchemaDatabaseProvider::Sqlite => Self::Sqlite,
        }
    }
}

impl From<DatabaseProvider> for SchemaDatabaseProvider {
    fn from(provider: DatabaseProvider) -> Self {
        match provider {
            DatabaseProvider::Postgres => Self::Postgres,
            DatabaseProvider::Mysql => Self::Mysql,
            DatabaseProvider::Sqlite => Self::Sqlite,
        }
    }
}

impl DatabaseProvider {
    /// Parse a canonical datasource provider or the legacy `postgres` alias.
    /// The alias is accepted here without extending the schema language.
    pub fn from_schema_provider(provider: &str) -> Option<Self> {
        if provider == "postgres" {
            return Some(SchemaDatabaseProvider::Postgres.into());
        }
        provider
            .parse::<SchemaDatabaseProvider>()
            .ok()
            .map(Self::from)
    }

    /// Return the canonical `.nautilus` datasource provider string.
    pub fn schema_provider_name(self) -> &'static str {
        SchemaDatabaseProvider::from(self).as_str()
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
