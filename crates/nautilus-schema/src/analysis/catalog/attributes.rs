//! The attributes the language accepts, described once for every reader.
//!
//! Each entry carries what completion offers — the label, the snippet and the
//! one-line detail — next to the documentation hover shows, so an attribute is
//! added, renamed or documented in one place.

/// One `@attr` or `@@attr` as the language surfaces it.
#[derive(Clone, Copy)]
pub(in crate::analysis) struct AttributeDoc {
    /// The attribute name as written after the `@`, and the hover key.
    pub name: &'static str,
    /// The completion label, including the argument shape it suggests.
    pub label: &'static str,
    /// LSP snippet inserted instead of the label, where the arguments deserve
    /// placeholders.
    pub snippet: Option<&'static str>,
    /// The one-line detail shown next to a completion item.
    pub detail: &'static str,
    /// The documentation shown on hover, as the lines it is written in.
    pub documentation: &'static [&'static str],
}

impl AttributeDoc {
    /// The documentation as one Markdown string.
    pub(in crate::analysis) fn documentation(&self) -> String {
        self.documentation.concat()
    }
}

/// Every field-level attribute, in the order completion offers them.
pub(in crate::analysis) const FIELD_ATTRIBUTES: &[AttributeDoc] = &[
    AttributeDoc {
        name: "id",
        label: "id",
        snippet: None,
        detail: "Mark as primary key",
        documentation: &["**@id**  \nMarks this field as the primary key of the model."],
    },
    AttributeDoc {
        name: "unique",
        label: "unique",
        snippet: None,
        detail: "Add a unique constraint",
        documentation: &["**@unique**  \nAdds a `UNIQUE` constraint on this column."],
    },
    AttributeDoc {
        name: "default",
        label: "default()",
        snippet: None,
        detail: "Set a default value",
        documentation: &[
            "**@default(expr)**  ",
            "Sets the default value for this field when not explicitly provided.  \n",
            "Common expressions: `autoincrement()`, `now()`, `uuid()`, `uuidv7()`,",
            " enum variants, or literal values.",
        ],
    },
    AttributeDoc {
        name: "relation",
        label: "relation()",
        snippet: None,
        detail: "Define a relation",
        documentation: &[
            "**@relation**  \n",
            "Defines an explicit foreign-key relation between two models.",
            "{base}  \n\n{extra}",
            "**@{other}**",
        ],
    },
    AttributeDoc {
        name: "map",
        label: "map(\"\")",
        snippet: None,
        detail: "Override the column name",
        documentation: &["**@map(\"name\")** \nMaps this field to a different physical column name in the database."],
    },
    AttributeDoc {
        name: "store",
        label: "store(json)",
        snippet: None,
        detail: "Store as JSON column",
        documentation: &[
            "**@store(json)**  \n",
            "Stores this array field as a JSON value in the database.  \n",
            "Useful for databases without native array support (MySQL, SQLite).",
        ],
    },
    AttributeDoc {
        name: "updatedAt",
        label: "updatedAt",
        snippet: None,
        detail: "Auto-set to current timestamp on every write",
        documentation: &[
            "**@updatedAt**  \n",
            "Marks this `DateTime` field to be automatically set to the current timestamp ",
            "on every CREATE and UPDATE operation.  \n",
            "The framework manages this value — it is excluded from all user-input types.",
        ],
    },
    AttributeDoc {
        name: "computed",
        label: "computed(…, Stored)",
        snippet: Some("computed(${1:expr}, ${2|Stored,Virtual|})"),
        detail: "Database-generated column (Stored or Virtual)",
        documentation: &[
            "**@computed(expr, Stored | Virtual)**  \n",
            "Declares a database-generated (computed) column.  \n\n",
            "- `expr` — raw SQL expression evaluated by the database (e.g. `price * quantity`, ",
            "`first_name || ' ' || last_name`)  \n",
            "- `Stored` — value is computed on write and persisted physically  \n",
            "- `Virtual` — value is computed on read (not supported on PostgreSQL)  \n\n",
            "Maps to SQL `GENERATED ALWAYS AS (expr) STORED` (PostgreSQL / MySQL) or ",
            "`AS (expr) STORED` (SQLite).  \n",
            "Computed fields are **read-only** — they are excluded from all create/update input types.",
        ],
    },
    AttributeDoc {
        name: "check",
        label: "check(…)",
        snippet: Some("check(${1:expr})"),
        detail: "Add a CHECK constraint on this field",
        documentation: &[
            "**@check(expr)**  \n",
            "Adds a SQL `CHECK` constraint on this column.  \n\n",
            "The boolean expression can use SQL-style operators: ",
            "`=`, `!=`, `<`, `>`, `<=`, `>=`, `AND`, `OR`, `NOT`, `IN`.  \n\n",
            "Field-level `@check` can only reference the decorated field itself.  \n",
            "Use `@@check` at the model level to reference multiple fields.  \n\n",
            "**Examples:**  \n",
            "```  \n",
            "age    Int  @check(age >= 0 AND age <= 150)  \n",
            "status Status @check(status IN [ACTIVE, PENDING])  \n",
            "```",
        ],
    },
    AttributeDoc {
        name: "ignore",
        label: "ignore",
        snippet: None,
        detail: "Leave this column out of the client and of every migration",
        documentation: &[
            "**@ignore**  \n",
            "Marks a database column as unmanaged by Nautilus. The field is omitted from generated clients and every migration; the column is never created, altered, or dropped.  \n\n",
            "An ignored field cannot use `@id`, `@unique`, or `@relation`, and cannot be referenced by `@@id`, `@@unique`, or `@@index`. A required ignored field without `@default` requires the whole model to be `@@ignore`.  \n\n",
            "`db pull` adds `@ignore` when a database column type has no Nautilus representation, preserving that column without mapping it to an incorrect type.  \n\n",
            "**Example:**  \n",
            "```  \n",
            "model Device {  \n",
            "  id     Int     @id  \n",
            "  uptime String? @ignore  \n",
            "}  \n",
            "```",
        ],
    },
];

/// Every model-level attribute, in the order completion offers them.
pub(in crate::analysis) const MODEL_ATTRIBUTES: &[AttributeDoc] = &[
    AttributeDoc {
        name: "id",
        label: "id([])",
        snippet: None,
        detail: "Composite primary key",
        documentation: &["**@@id([fields])**  \nDefines a composite primary key spanning multiple fields."],
    },
    AttributeDoc {
        name: "unique",
        label: "unique([])",
        snippet: None,
        detail: "Composite unique constraint",
        documentation: &["**@@unique([fields])**  \nDefines a composite unique constraint spanning multiple fields."],
    },
    AttributeDoc {
        name: "index",
        label: "index([])",
        snippet: None,
        detail: "Add a database index — optionally with type: BTree|Hash|Gin|Gist|Brin|FullText",
        documentation: &["**@@index([fields], type?, opclass?, m?, ef_construction?, lists?, name?, map?)**  \nCreates a database index on the listed fields.  \n\nOptional arguments:  \n- `type:` — index access method: `BTree` (default, all DBs), `Hash` (PG/MySQL), `Gin` / `Gist` / `Brin` / `Hnsw` / `Ivfflat` (PostgreSQL only), `FullText` (MySQL only)  \n- `opclass:` — pgvector operator class for `Hnsw` / `Ivfflat`: `vector_l2_ops`, `vector_ip_ops`, `vector_cosine_ops`  \n- `m:` / `ef_construction:` — pgvector HNSW build parameters  \n- `lists:` — pgvector IVFFlat build parameter  \n- `name:` — logical developer name (ignored in DDL)  \n- `map:` — physical DDL index name override  \n\n**Examples:**  \n```  \n@@index([email])  \n@@index([email], type: Hash)  \n@@index([content], type: Gin)  \n@@index([embedding], type: Hnsw, opclass: vector_cosine_ops, m: 16, ef_construction: 64)  \n```"],
    },
    AttributeDoc {
        name: "map",
        label: "map(\"\")",
        snippet: None,
        detail: "Override the table name",
        documentation: &["**@@map(\"name\")** \nMaps this declaration to a different physical name in the database — the table name for a model, or the SQL composite type name for a `type`."],
    },
    AttributeDoc {
        name: "check",
        label: "check(…)",
        snippet: Some("check(${1:expr})"),
        detail: "Add a table-level CHECK constraint",
        documentation: &[
            "**@@check(expr)**  \n",
            "Adds a table-level SQL `CHECK` constraint.  \n\n",
            "Unlike field-level `@check`, the expression can reference any scalar field in the model.  \n\n",
            "**Example:**  \n",
            "```  \n",
            "@@check(start_date < end_date)  \n",
            "@@check(age > 18 OR status IN [MINOR])  \n",
            "```",
        ],
    },
    AttributeDoc {
        name: "ignore",
        label: "ignore",
        snippet: None,
        detail: "Leave this table out of the client and of every migration",
        documentation: &[
            "**@@ignore**  \n",
            "Marks a database table as unmanaged by Nautilus. The model is omitted from generated clients and every migration; its table is never created, altered, or dropped.  \n\n",
            "A managed model cannot declare a relation to an ignored model. Mark that relation field with `@ignore`, or remove `@@ignore` from its target.  \n\n",
            "`db pull` adds `@@ignore` when a table cannot be represented safely, such as when its primary key or a required column without a default has an unsupported database type.  \n\n",
            "**Example:**  \n",
            "```  \n",
            "model LegacyAudit {  \n",
            "  id   Int    @id  \n",
            "  span String @ignore  \n\n",
            "  @@map(\"legacy_audit\")  \n",
            "  @@ignore  \n",
            "}  \n",
            "```",
        ],
    },
    AttributeDoc {
        name: "schema",
        label: "schema(\"\")",
        snippet: Some("schema(\"${1:public}\")"),
        detail: "PostgreSQL schema that owns this table",
        documentation: &[
            "**@@schema(\"name\")**  \n",
            "Selects the PostgreSQL schema that contains this model's table or this view.  \n\n",
            "The name must be listed in the datasource's `schemas = [...]` field. When that field is present, every model and view must declare `@@schema`; Nautilus does not choose an implicit default.  \n\n",
            "PostgreSQL only — MySQL and SQLite do not support this attribute.  \n\n",
            "**Example:**  \n",
            "```  \n",
            "datasource db {  \n",
            "  provider = \"postgresql\"  \n",
            "  schemas  = [\"public\", \"analytics\"]  \n",
            "}  \n\n",
            "model Event {  \n",
            "  id Int @id  \n",
            "  @@schema(\"analytics\")  \n",
            "}  \n",
            "```",
        ],
    },
];

/// The only attribute a composite `type` block accepts: `@@map`, with the
/// wording that fits a SQL type rather than a table.
pub(in crate::analysis) fn type_attribute_map() -> AttributeDoc {
    AttributeDoc {
        detail: "Override the SQL composite type name",
        ..*model_attribute("map").expect("@@map is a model attribute")
    }
}

/// The entry describing a field-level attribute, by the name after the `@`.
pub(in crate::analysis) fn field_attribute(name: &str) -> Option<&'static AttributeDoc> {
    FIELD_ATTRIBUTES.iter().find(|doc| doc.name == name)
}

/// The entry describing a model-level attribute, by the name after the `@@`.
pub(in crate::analysis) fn model_attribute(name: &str) -> Option<&'static AttributeDoc> {
    MODEL_ATTRIBUTES.iter().find(|doc| doc.name == name)
}
