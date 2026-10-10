//! The contract every dialect implements, and the rendered SQL it returns.

use nautilus_core::{Delete, Insert, Result, Select, Update, Value};

/// SQL query with bound parameters.
///
/// Separates the SQL text from parameter values for use with prepared statements.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Sql {
    /// The SQL query text with parameter placeholders.
    pub text: String,
    /// The parameter values to bind to the query.
    pub params: Vec<Value>,
}

/// Rendered SQL with the original names of shortened result aliases.
///
/// Execute `sql`, then restore each returned SQL alias to its original name
/// before accessing or serializing the row. Raw SQL needs no such mapping.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct SqlOutput {
    /// The statement and its bound values.
    pub sql: Sql,
    /// SQL alias and original result name, shared across returned rows.
    pub aliases: Vec<(String, std::sync::Arc<str>)>,
}

impl From<Sql> for SqlOutput {
    fn from(sql: Sql) -> Self {
        Self {
            sql,
            aliases: Vec::new(),
        }
    }
}

impl std::ops::Deref for SqlOutput {
    type Target = Sql;
    fn deref(&self) -> &Sql {
        &self.sql
    }
}

/// Trait for SQL dialect renderers.
///
/// Allows rendering AST queries into dialect-specific SQL strings.
pub trait Dialect {
    /// Render a SELECT with result-name metadata for provider alias limits.
    fn render_select_output_owned(&self, select: Select) -> Result<SqlOutput> {
        self.render_select_owned(select).map(Into::into)
    }

    /// Render an INSERT with result-name metadata for RETURNING columns.
    fn render_insert_output_owned(&self, insert: Insert) -> Result<SqlOutput> {
        self.render_insert_owned(insert).map(Into::into)
    }

    /// Render an UPDATE with result-name metadata for RETURNING columns.
    fn render_update_output_owned(&self, update: Update) -> Result<SqlOutput> {
        self.render_update_owned(update).map(Into::into)
    }

    /// Render a DELETE with result-name metadata for RETURNING columns.
    fn render_delete_output_owned(&self, delete: Delete) -> Result<SqlOutput> {
        self.render_delete_owned(delete).map(Into::into)
    }

    /// Whether this dialect natively supports the RETURNING clause
    /// on INSERT, UPDATE, and DELETE statements.
    ///
    /// Dialects that return `false` (e.g. MySQL) will have RETURNING
    /// emulated at the connector layer via separate queries.
    fn supports_returning(&self) -> bool {
        true
    }

    /// Whether this dialect can restrict a result set to one row per
    /// distinct-column combination on its own (PostgreSQL's `DISTINCT ON`).
    ///
    /// A dialect that returns `false` renders `SELECT DISTINCT`, which
    /// deduplicates whole rows and therefore cannot honour `distinct` by
    /// itself; the engine deduplicates the decoded rows instead.
    fn supports_distinct_on(&self) -> bool {
        false
    }

    /// Render an owned SELECT query into SQL, moving bound values out of the AST
    /// instead of cloning them. Use [`Self::render_select_output_owned`] when
    /// returned field names must survive provider identifier limits.
    fn render_select_owned(&self, select: Select) -> Result<Sql>;

    /// Render a borrowed SELECT query into SQL.
    ///
    /// Clones the AST once and delegates to [`Self::render_select_owned`]. Used by
    /// previews, tests, and other non-hot paths that only hold a `&Select`.
    fn render_select(&self, select: &Select) -> Result<Sql> {
        self.render_select_owned(select.clone())
    }

    /// Render an owned INSERT query into SQL, moving bound values out of the AST
    /// instead of cloning them.
    fn render_insert_owned(&self, insert: Insert) -> Result<Sql>;

    /// Render a borrowed INSERT query into SQL by cloning and delegating to
    /// [`Self::render_insert_owned`].
    fn render_insert(&self, insert: &Insert) -> Result<Sql> {
        self.render_insert_owned(insert.clone())
    }

    /// Render an owned UPDATE query into SQL, moving bound values out of the AST
    /// instead of cloning them.
    fn render_update_owned(&self, update: Update) -> Result<Sql>;

    /// Render a borrowed UPDATE query into SQL by cloning and delegating to
    /// [`Self::render_update_owned`].
    fn render_update(&self, update: &Update) -> Result<Sql> {
        self.render_update_owned(update.clone())
    }

    /// Render an owned DELETE query into SQL, moving bound values out of the AST
    /// instead of cloning them.
    fn render_delete_owned(&self, delete: Delete) -> Result<Sql>;

    /// Render a borrowed DELETE query into SQL by cloning and delegating to
    /// [`Self::render_delete_owned`].
    fn render_delete(&self, delete: &Delete) -> Result<Sql> {
        self.render_delete_owned(delete.clone())
    }
}
