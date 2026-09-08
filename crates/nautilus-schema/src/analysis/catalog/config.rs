//! The configuration fields a `datasource` or `generator` block accepts,
//! described once for every reader.
//!
//! One entry names the key, the one-line detail completion shows in each block
//! that accepts it, and the documentation hover shows. A field offered in only
//! one kind of block has no detail for the other.

/// The kind of block a key or one of its values belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::analysis) enum ConfigBlock {
    /// A `datasource` block.
    Datasource,
    /// A `generator` block.
    Generator,
}

/// One value a configuration key accepts.
pub(in crate::analysis) struct ConfigValueDoc {
    /// The value, written as a string literal in the schema.
    pub value: &'static str,
    /// The block that accepts it, when only one of them does.
    pub block: Option<ConfigBlock>,
    /// The one-line detail shown next to a completion item.
    pub detail: &'static str,
}

/// One key of a `datasource` or `generator` block.
pub(in crate::analysis) struct ConfigFieldDoc {
    /// The key as written before the `=`.
    pub key: &'static str,
    /// The completion detail inside a `datasource` block, when it belongs there.
    pub datasource_detail: Option<&'static str>,
    /// The completion detail inside a `generator` block, when it belongs there.
    pub generator_detail: Option<&'static str>,
    /// The documentation shown on hover, as the lines it is written in.
    pub documentation: &'static [&'static str],
    /// The values the key accepts, when they are a fixed set.
    pub values: &'static [ConfigValueDoc],
}

impl ConfigFieldDoc {
    /// The documentation as one Markdown string.
    pub(in crate::analysis) fn documentation(&self) -> String {
        self.documentation.concat()
    }
}

/// Every configuration key, in the order completion offers them.
pub(in crate::analysis) const CONFIG_FIELDS: &[ConfigFieldDoc] = &[
    ConfigFieldDoc {
        key: "provider",
        datasource_detail: Some("Database provider"),
        generator_detail: Some("Client generator provider"),
        documentation: &[
            "**provider**  \n",
            "Specifies the database provider or code-generator target.  \n\n",
            "Datasource values: `\"postgresql\"`, `\"mysql\"`, `\"sqlite\"`  \n",
            "Generator values: `\"nautilus-client-rs\"`, `\"nautilus-client-py\"`, `\"nautilus-client-js\"`, `\"nautilus-client-java\"`",
        ],
        values: &[
            ConfigValueDoc {
                value: "postgresql",
                block: Some(ConfigBlock::Datasource),
                detail: "PostgreSQL database",
            },
            ConfigValueDoc {
                value: "mysql",
                block: Some(ConfigBlock::Datasource),
                detail: "MySQL database",
            },
            ConfigValueDoc {
                value: "sqlite",
                block: Some(ConfigBlock::Datasource),
                detail: "SQLite database",
            },
            ConfigValueDoc {
                value: "nautilus-client-rs",
                block: Some(ConfigBlock::Generator),
                detail: "Rust client generator",
            },
            ConfigValueDoc {
                value: "nautilus-client-py",
                block: Some(ConfigBlock::Generator),
                detail: "Python client generator",
            },
            ConfigValueDoc {
                value: "nautilus-client-js",
                block: Some(ConfigBlock::Generator),
                detail: "JavaScript/TypeScript client generator",
            },
            ConfigValueDoc {
                value: "nautilus-client-java",
                block: Some(ConfigBlock::Generator),
                detail: "Java client generator",
            },
        ],
    },
    ConfigFieldDoc {
        key: "url",
        datasource_detail: Some("Connection URL"),
        generator_detail: None,
        documentation: &[
            "**url**  \n",
            "Database connection URL.  \n\n",
            "Supports the `env(\"VAR\")` helper to read from environment variables.",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "direct_url",
        datasource_detail: Some("Direct admin/introspection URL"),
        generator_detail: None,
        documentation: &[
            "**direct_url**  \n",
            "Optional direct database connection URL for admin tooling.  \n\n",
            "Use this for migrations, introspection, and schema management when `url` points at a pooled or proxied connection.  \n\n",
            "Supports the `env(\"VAR\")` helper to read from environment variables.",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "extensions",
        datasource_detail: Some("PostgreSQL extensions to install before DDL"),
        generator_detail: None,
        documentation: &[
            "**extensions**  \n",
            "Optional PostgreSQL-only array of extension names to ensure installed before schema DDL runs.  \n\n",
            "Accepts bare identifiers like `pg_trgm` and string literals like `\"uuid-ossp\"`.  \n\n",
            "Example: `extensions = [pg_trgm, pgcrypto, \"uuid-ossp\"]`",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "preserve_extensions",
        datasource_detail: Some("Preserve live PostgreSQL extensions not listed in the schema"),
        generator_detail: None,
        documentation: &[
            "**preserve_extensions**  \n",
            "Optional PostgreSQL-only boolean.  \n\n",
            "When `true`, Nautilus creates missing declared extensions but does not propose dropping live extensions that are absent from `extensions`.  \n\n",
            "Default: `false`.",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "schemas",
        datasource_detail: Some("PostgreSQL schemas this datasource spans"),
        generator_detail: None,
        documentation: &[
            "**schemas**  \n",
            "PostgreSQL-only, non-empty array of unique schema names that this datasource spans. Every entry must be a non-empty string literal.  \n\n",
            "When `schemas` is present, every model and view must use `@@schema(\"...\")` with one of the declared names; Nautilus does not choose an implicit default.  \n\n",
            "Migrations create missing declared schemas before their tables but never drop a schema. `db pull` introspects exactly the schemas listed here.  \n\n",
            "**Example:**  \n",
            "```  \n",
            "datasource db {  \n",
            "  provider = \"postgresql\"  \n",
            "  url      = env(\"DATABASE_URL\")  \n",
            "  schemas  = [\"public\", \"analytics\"]  \n",
            "}  \n\n",
            "model Event {  \n",
            "  id Int @id  \n",
            "  @@schema(\"analytics\")  \n",
            "}  \n",
            "```",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "output",
        datasource_detail: None,
        generator_detail: Some("Output path for generated files"),
        documentation: &[
            "**output**  \n",
            "Output directory path for generated client files.  \n\n",
            "Relative paths are resolved from the schema file location.",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "interface",
        datasource_detail: None,
        generator_detail: Some("Client interface style: \"sync\" (default) or \"async\""),
        documentation: &[
            "**interface**  \n",
            "Controls whether the generated client uses a synchronous or asynchronous API.  \n\n",
            "- `\"sync\"` *(default)* — blocking API; safe to call from any context.  \n",
            "- `\"async\"` — `async/await` API; requires an async runtime.",
        ],
        values: &[
            ConfigValueDoc {
                value: "sync",
                block: None,
                detail: "Synchronous client interface (default)",
            },
            ConfigValueDoc {
                value: "async",
                block: None,
                detail: "Asynchronous client interface",
            },
        ],
    },
    ConfigFieldDoc {
        key: "recursive_type_depth",
        datasource_detail: None,
        generator_detail: Some("Depth of recursive include TypedDicts — Python client only (default: 5)"),
        documentation: &[
            "**recursive_type_depth**  \n",
            "*(Python client only)* Depth of recursive include TypedDicts generated for the Python client.  \n\n",
            "Default: `5`.  \n\n",
            "Each depth level adds a `{Model}IncludeRecursive{N}` type and the corresponding  \n",
            "`FindMany{Target}ArgsFrom{Source}Recursive{N}` typed-dict classes.  \n",
            "At the maximum depth the `include` field is omitted to prevent infinite type recursion.  \n\n",
            "Example: `recursive_type_depth = 3`",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "package",
        datasource_detail: None,
        generator_detail: Some("Root Java package for generated sources"),
        documentation: &[
            "**package**  \n",
            "*(Java client only)* Root Java package for the generated client sources.  \n\n",
            "Example: `package = \"com.acme.db\"`",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "group_id",
        datasource_detail: None,
        generator_detail: Some("Maven groupId for the Java module"),
        documentation: &[
            "**group_id**  \n",
            "*(Java client only)* Maven `groupId` used in the generated `pom.xml`.  \n\n",
            "Example: `group_id = \"com.acme\"`",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "artifact_id",
        datasource_detail: None,
        generator_detail: Some("Maven artifactId for the Java module"),
        documentation: &[
            "**artifact_id**  \n",
            "*(Java client only)* Maven `artifactId` used in the generated `pom.xml`.  \n\n",
            "Example: `artifact_id = \"db-client\"`",
        ],
        values: &[],
    },
    ConfigFieldDoc {
        key: "mode",
        datasource_detail: None,
        generator_detail: Some("Java output mode: \"maven\" (default) or \"jar\""),
        documentation: &[
            "**mode**  \n",
            "*(Java client only)* Controls the Java packaging output.  \n\n",
            "- `\"maven\"` *(default)* — generate the Maven module layout under `output/`.  \n",
            "- `\"jar\"` — generate the Maven module layout and also build a plain Java jar bundle under `output/dist/`.",
        ],
        values: &[
            ConfigValueDoc {
                value: "maven",
                block: None,
                detail: "Generate a Maven module (default)",
            },
            ConfigValueDoc {
                value: "jar",
                block: None,
                detail: "Also build a plain Java jar bundle under output/dist",
            },
        ],
    },
];

/// The entry describing a configuration key.
pub(in crate::analysis) fn config_field(key: &str) -> Option<&'static ConfigFieldDoc> {
    CONFIG_FIELDS.iter().find(|doc| doc.key == key)
}
