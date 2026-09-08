//! The configuration blocks as the IR carries them: the datasource a schema
//! connects to and the client generators it declares.

use crate::span::Span;

/// A single PostgreSQL extension declaration, as it appears in the validated IR.
///
/// Supports both the shorthand syntax (`pg_trgm`, `"uuid-ossp"`) and the
/// structured form (`extension(name = vector, schema = "extensions")`). The
/// shorthand produces entries with `schema = None`, meaning "install in the
/// PostgreSQL default search path".
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PostgresExtensionIr {
    /// Extension name as it appears in `pg_extension.extname` (lower-cased).
    pub name: String,
    /// Optional target schema. When `Some`, the DDL emits
    /// `CREATE EXTENSION ... WITH SCHEMA "<schema>"` and `db pull` round-trips
    /// the declaration as a structured entry.
    pub schema: Option<String>,
}

/// Validated datasource configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasourceIr {
    /// The datasource name (e.g., "db").
    pub name: String,
    /// The provider (e.g., "postgresql", "mysql", "sqlite").
    pub provider: String,
    /// The connection URL (may contain env() references).
    pub url: String,
    /// Optional direct connection URL for admin/introspection paths.
    ///
    /// When present, tooling such as `db pull`, `db push`, and migrations can
    /// prefer this over `url` so runtime traffic can continue to use a pooled
    /// connection string.
    pub direct_url: Option<String>,
    /// PostgreSQL extensions declared in the datasource block.
    ///
    /// Entries are deduplicated by name and sorted for stable output.
    /// Empty for non-Postgres providers (enforced by the validator).
    pub extensions: Vec<PostgresExtensionIr>,
    /// PostgreSQL schemas the datasource spans, in declaration order.
    ///
    /// Empty means single-schema mode: table names are unqualified and resolve
    /// through the connection's `search_path`. When non-empty, every model and
    /// view must name its owning schema with `@@schema("...")`, and `db pull`
    /// introspects exactly these schemas.
    pub schemas: Vec<String>,
    /// Preserve PostgreSQL extensions that are installed in the live database
    /// but not listed in [`extensions`](Self::extensions).
    ///
    /// When `false` (the default), extension management is fully declarative:
    /// extra live extensions are diffed as destructive `DROP EXTENSION`
    /// changes. When `true`, Nautilus still creates declared missing
    /// extensions, but it does not propose dropping extra live extensions.
    pub preserve_extensions: bool,
    /// Span of the datasource block.
    pub span: Span,
}

impl DatasourceIr {
    /// Returns the preferred runtime URL expression.
    ///
    /// Runtime clients should prefer `url` and only fall back to `direct_url`
    /// when `url` is unavailable.
    pub fn runtime_url(&self) -> &str {
        if !self.url.is_empty() {
            &self.url
        } else {
            self.direct_url.as_deref().unwrap_or(&self.url)
        }
    }

    /// Returns the preferred admin/introspection URL expression.
    ///
    /// Admin tooling should prefer `direct_url` when present, then fall back to
    /// the normal runtime `url`.
    pub fn admin_url(&self) -> &str {
        self.direct_url.as_deref().unwrap_or(&self.url)
    }
}

/// Whether the generated client API uses async or sync methods.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InterfaceKind {
    /// Synchronous API (default). Methods are plain `fn`, Rust uses
    /// `tokio::task::block_in_place` internally; Python uses `asyncio.run()`.
    #[default]
    Sync,
    /// Asynchronous API. Methods are `async fn` in Rust and `async def` in Python.
    Async,
}

/// Packaging mode for the generated Java client bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JavaGenerationMode {
    /// Generate the default Maven module layout rooted at `output/`.
    #[default]
    Maven,
    /// Generate the Maven module layout and also build a plain Java jar bundle.
    Jar,
}

/// Validated generator configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorIr {
    /// The generator name (e.g., "client").
    pub name: String,
    /// The provider (e.g., "nautilus-client-rs").
    pub provider: String,
    /// The output path (if specified).
    pub output: Option<String>,
    /// Whether to generate a sync or async client interface.
    /// Defaults to [`InterfaceKind::Sync`] when the `interface` field is omitted.
    pub interface: InterfaceKind,
    /// Depth of recursive include TypedDicts generated for the Python client.
    pub recursive_type_depth: usize,
    /// Root Java package for the generated client (Java provider only).
    pub java_package: Option<String>,
    /// Maven groupId for the generated Java module (Java provider only).
    pub java_group_id: Option<String>,
    /// Maven artifactId for the generated Java module (Java provider only).
    pub java_artifact_id: Option<String>,
    /// Java packaging mode (Java provider only).
    pub java_mode: Option<JavaGenerationMode>,
    /// Span of the generator block.
    pub span: Span,
}
