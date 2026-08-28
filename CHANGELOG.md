# Changelog

## Unreleased

### ⚠️ Breaking — MySQL enum columns change type

Enum fields on MySQL are now created as a native column `ENUM('A', 'B')`
instead of `TEXT`. Storing them as text made MySQL reject the column outright
in two common cases: an enum with `@default(...)` failed with error 1101
(`BLOB, TEXT, GEOMETRY or JSON column can't have a default value`) and an enum
covered by `@@index` failed with error 1170 (`BLOB/TEXT column used in key
specification without a key length`).

**On an existing MySQL database the first `db push` after upgrading proposes a
`TEXT` → `ENUM(...)` change on every enum column**, reported as destructive and
gated behind the usual confirmation.

Applying it is safe by default: under `STRICT_TRANS_TABLES`, MySQL 8's default,
a stored value that is not one of the declared variants makes the `ALTER` fail
with error 1265 (`Data truncated for column '<column>' at row N`) and the
statement is rolled back, leaving the column and its data untouched. Clean the
offending rows and push again:

```sql
SELECT DISTINCT <column> FROM <table>;
```

A value that differs only in case is accepted and normalised to the declared
spelling (`draft` becomes `DRAFT`). With strict mode disabled MySQL would
instead coerce an unknown value to the empty string, so check `@@sql_mode`
before pushing if your server has been reconfigured. PostgreSQL and SQLite are
unaffected.

### Added

- Added `Change::risk()` and `Change::describe()` on the migration `Change`
  enum, so a change's risk classification and its user-facing description live
  next to the variant they describe. `describe()` returns the new
  `ChangeDescription` struct (sigil, subject, annotation) and replaces the
  parallel match the CLI kept in its TUI module. The free function
  `change_risk()` is retained and now delegates to `Change::risk()`.
- Added structured logging to the engine, replacing the unconditional
  `eprintln!` diagnostics. Records are emitted on stderr through `tracing` and
  filtered by `NAUTILUS_LOG` (falling back to `RUST_LOG`, defaulting to
  `nautilus_engine=info`), so per-request transaction lifecycle events now sit
  at `debug` instead of being printed on every call.
- Added a slow-statement log to the engine, enabled by setting
  `NAUTILUS_SLOW_QUERY_MS`. Statements running past the threshold are logged on
  the `nautilus_engine::slow_query` target with their SQL text, operation tag
  and duration. Unset — the default — the timing path is skipped entirely.
- Added `max_concurrent_requests` to `EnginePoolOptions`, exposed as
  `nautilus engine serve --max-concurrent-requests`, the engine
  `--max-concurrent-requests` flag, and `maxConcurrentRequests` /
  `max_concurrent_requests` on the generated JS and Python clients. The engine
  transport now admits at most this many handlers at once instead of spawning a
  task per input line, so a client that pipelines faster than the engine drains
  no longer grows the task set without bound. Unset, the limit is four times the
  configured pool size.

- Added pgvector nearest-neighbor search to the generated Rust client, closing
  the last client parity gap: `FindManyArgs::nearest` is now honoured on the
  direct SQL path and ordered by the pgvector distance operator, and each model
  exposes one `{field}_nearest(query, metric)` constructor per `Vector(dim)`
  field so the searched field is checked at compile time. The restrictions match
  the engine — a positive `take` is required, and `nearest` rejects `cursor`,
  `distinct` and backward pagination.
- Added `nautilus_core::ExtensionScalar`, the marker a generated client
  implements on its PostgreSQL extension wrappers (pgvector, PostGIS, citext,
  hstore, ltree) to opt them into array encoding and decoding. The orphan rule
  keeps a generated crate from writing those conversions itself, so they live in
  `nautilus-core` keyed on this marker.

### Changed

- The workspace now declares `rust-version = "1.92"`, so an older toolchain
  reports an unsupported-version error instead of failing later with unrelated
  compilation errors.
- Template rendering in the code generators now returns an error instead of
  panicking. A regressed template used to abort `nautilus generate` with a Rust
  backtrace that did not name the template; the failure is now reported as a
  CLI error naming it. The generator entry points that render templates —
  `generate_all_models`, `generate_all_python_models`, `generate_all_js_models`,
  `generate_java_client`, the enum / composite type / extension file
  generators and the per-language client and `__init__` generators — return
  `Result` accordingly.
- Array includes carrying `take` or `skip` are now loaded by the same single
  batched query as unpaginated includes, using
  `ROW_NUMBER() OVER (PARTITION BY <child fk> ORDER BY ...)` to bound each
  parent's children independently. The previous per-parent fallback issued one
  query per parent row, so `include: { posts: { take: 3 } }` over 100 parents
  cost 100 queries and now costs one. To-one relations and a negative `take`
  still take the per-parent path. Window functions require PostgreSQL >= 8.4,
  MySQL >= 8.0 or SQLite >= 3.25.
- The engine read-plan cache now reclaims its least-recently-used entries a
  batch at a time instead of ranking all 1024 slots on every insert at
  capacity. The recency scan runs under the write lock with every reader
  blocked, so a saturated cache now pays for it once per batch rather than on
  each miss. The cap is unchanged; evictions are logged at `debug`.
- The four code generation backends now share a single language-neutral
  `ModelView` of each model instead of each walking the IR on its own. Primary
  key membership, numeric and orderable classification, enum / composite type /
  extension imports, relation foreign key resolution and composite order-by
  paths are computed once and mapped per language, so a new field kind is added
  in one place rather than four. Generated output is unchanged.

### Fixed

- `db pull` no longer writes the database password into the schema it
  generates. The resolved connection string was embedded in the datasource
  block, in a file that normally gets committed; the pulled schema now points
  at `env("NAME")`, reusing the source schema's own variable when it has one.
- `db pull` reconstructs MySQL enums. A column enum lives inline in the type
  (`enum('DRAFT','PUBLISHED')`) and came back as a plain `String`, so pushing a
  pulled schema back proposed a destructive downgrade of every enum column to
  `varchar`. Columns sharing a variant list share one declaration, named after
  the first table and column that introduce it.
- A string literal is now accepted as the default of every text-backed native
  type (`VarChar`, `Char`, `Citext`, `Ltree`, `Xml`), which only `String`
  allowed. `db pull` emits exactly that for a `varchar` column with a default,
  so the pulled schema failed to validate with a type mismatch.
- `citext` and `ltree` columns are no longer compared case sensitively.
  A `citext` value travelled to the database as an untyped string, so
  PostgreSQL resolved `slug = $1` as `text = text` — a query for `slug-near`
  did not match a stored `Slug-Near`, silently defeating the point of the type.
  Both now travel as `Value::Extension` carrying their type name, so the
  dialect emits `$1::citext` exactly as it already did for enums. This applies
  to every client: the Rust one and, through the engine, JavaScript, Python and
  Java.
- MySQL string and enum defaults no longer produce a phantom change on every
  `db push`. `information_schema` reports the *value* of a literal default, so
  `@default("basic")` came back as `basic` and compared unequal to the
  generated `'basic'` forever.
- The MySQL schema inspector works again on MySQL 8, so `db pull` and any
  `db push` against a database that already has tables no longer fail with
  `no column found for name: table_name`. MySQL reports `information_schema`
  column labels in upper case and their values as `VARBINARY`, so the queries
  now alias and `CAST(... AS CHAR)` every column they read. A first push
  against an empty database was unaffected, which is why this went unnoticed.
- `db push` is idempotent again for composite-typed and `decimal` columns.
  Introspection reports `address` and `decimal(6,2)` where the DDL generator
  writes `"address"` and `decimal(6, 2)`, and the diff compared the two
  spellings literally, so every push after the first proposed a destructive
  `TypeChanged` for a column that had not changed.
- Generated JavaScript clients decode arrays of extension types (`Citext[]`,
  `Vector[]`, …) instead of leaving a function on the model. The array coercer
  was interpolated into the assignment without being applied, so the field held
  an arrow function that `JSON.stringify` dropped and every read silently lost
  the column.
- Generated JavaScript and Python clients can write SQL NULL into a nullable
  extension column. The extension coercers only know how to build a value and
  rejected `null` / `None` with `Unsupported Citext input`, even though the
  declared input type allows it.
- PostgreSQL `INSERT` and `UPDATE` now cast a bound parameter to its column
  type, the way the `WHERE` and `SELECT` paths already did. Values that bind as
  text — pgvector, PostGIS and JSON — were rejected by the server with
  `column "…" is of type vector but expression is of type text`, which made a
  `Vector`, `Geometry`, `Geography` or `jsonb` column impossible to write to.
- A pgvector `vector` column now decodes from the binary wire format the
  extended protocol returns. Reading one back failed with
  `Failed to decode VECTOR: invalid utf-8 sequence`, because the decoder always
  treated the payload as a text literal.
- The engine read-plan cache no longer becomes a permanent no-op after a handler
  panic. The transport converts panics into JSON-RPC errors rather than aborting,
  so a panic taken while the cache lock was held poisoned it for the rest of the
  process lifetime and silently disabled plan reuse with no error surface.
- The engine transport now rejects a request line above 64 MiB instead of
  buffering it, so a malformed writer can no longer grow the read buffer without
  bound.
- Generated Python clients for a model with a `Vector(dim)` field are now valid
  Python. Tera whitespace control stripped the indentation off the `nearest`
  argument block in `find_many`, `find_first` and `find_unique`, so the module
  failed to import with an `IndentationError`.
- Generated Rust clients that use a PostgreSQL extension type now compile. The
  extension wrappers carried `impl From<Option<Wrapper>> for Value`,
  `impl From<Vec<Wrapper>> for Value` and `impl FromValue for Vec<Wrapper>`,
  all of which the orphan rule rejects from the generated crate, so any schema
  declaring a `Vector`, `Citext`, `Ltree`, `Hstore`, `Geometry` or `Geography`
  column produced a client that failed to build. `Option<T>` conversion is now a
  single generic impl in `nautilus-core` and the array conversions go through
  [`ExtensionScalar`].
- Enum, composite type and relation imports in generated model files are now
  emitted in a stable order. They were collected in a `HashSet`, so two runs of
  `nautilus generate` over the same schema could produce files that differed
  only in the order of their import lines.

## Version 1.3.5

### Added

- Added `@default(uuidv7())` for PostgreSQL, generating native `uuidv7()` column
  defaults so time-ordered UUID primary keys are produced by the database on
  insert. The schema validator rejects `uuidv7()` on non-`Uuid` fields and on
  providers without native support (MySQL/SQLite), and DDL generation errors for
  those providers rather than emitting invalid SQL. When the schema has no
  datasource with a recognized provider to validate against, `uuidv7()` produces
  an analysis warning instead, since it is PostgreSQL-only.

### Fixed

- Versioned `migrate apply`, `rollback`, and `status` now work on PostgreSQL.
  The migration tracker rendered `?` bind placeholders, which PostgreSQL rejects
  with a syntax error, so no versioned migration could be applied (`db push` was
  unaffected). Bind placeholders are now provider-aware (`$1` for PostgreSQL,
  `?` for MySQL/SQLite).
- Down migrations generated from a schema diff now drop tables in reverse
  dependency order. Rolling back a migration that creates a parent table before a
  child table holding a foreign key no longer fails with a dependency error.
- Down `DROP TABLE` reversals on PostgreSQL now use `CASCADE`, consistent with
  the other drop paths, so a rollback also clears objects that came to depend on
  a created table (for example views) outside the migration.

## Version 1.3.4

### Added

- Added a bounded LRU read-plan cache for repeated `findUnique`, `findFirst`,
  and cacheable `findMany` request shapes, reusing rendered SQL and row hints
  while binding fresh parameters per call.
- Added borrowed `Value` serializers, including `PlainValueRef`, so row payloads
  and tagged values can be serialized without building intermediate JSON trees.
- Added Criterion benchmark coverage for row access and decoding, value
  serialization, SQL rendering, schema parsing, row JSON serialization, and
  include hydration.

### Changed

- RPC request params are now kept as `RawValue` and deserialized directly into
  each handler's concrete parameter type, reducing duplicate JSON parsing and
  centralizing invalid-params error reporting.
- Row decoding and JSON output hot paths now avoid more cloning by batching
  connector decodes, sharing column-name metadata across rows, and serializing
  borrowed values directly.
- Include hydration now groups relation results more efficiently and uses
  bounded concurrency for follow-up include loads.
- PostgreSQL row streaming and query execution paths now do less per-row and
  per-request work on repeated read workloads.
- Shared crate dependencies are now centralized in the workspace manifest, and
  internal handler visibility has been narrowed while keeping benchmark helpers
  available.

### Fixed

- UUID-shaped strings now use a fast shape check before parsing, preserving UUID
  binding behavior for PostgreSQL parameters without attempting parses for
  ordinary strings.

## Version 1.3.3

### Added

- Added nested composite-field ordering via dotted `orderBy` paths, with typed generated client support across Rust,
  JavaScript/TypeScript, Python, and Java.
- Composite-field ordering now renders through native PostgreSQL composite
  attributes and JSON path extraction for SQLite/MySQL, including numeric casts
  for MySQL JSON-backed composite fields.

### Fixed

- MySQL JSON columns now decode into protocol JSON values instead of raw strings,
  preserving JSON-backed composite payloads in query results.

## Version 1.3.2

### Fixed

- PostgreSQL composite columns returned in the binary protocol format are now
  decoded before runtime normalization, fixing Python client reads and
  `RETURNING` results for native composite fields.
- Generated Python composite `TypedDict` helpers now import from
  `typing_extensions` on Python versions before 3.12, keeping Pydantic v2
  model construction compatible with Python 3.11.

## Version 1.3.1

### Added

- Added PostgreSQL native composite type support, including schema `type`
  blocks, create/update binding, row decoding, and generated client type
  support across Rust, JavaScript/TypeScript, Python, and Java.
- Migrations can now create, drop, introspect, serialize, and reparse
  PostgreSQL composite types and composite arrays, while SQLite and MySQL keep
  composite fields on the JSON storage path.
- Schema tooling now understands composite type declarations in parsing,
  validation, formatting, completion, hover, and go-to-definition flows.

### Fixed

- Composite type `@@map` and field `@map` names are now preserved through IR,
  DDL generation, migration serialization, runtime conversion, and generated
  code.
- PostgreSQL composite type names are now quoted consistently in generated DDL
  and runtime casts, so mapped mixed-case SQL type names work correctly.
- Composite type validation now rejects unsupported model/field attributes and
  nested composite references with clearer diagnostics.

## Version 1.3.0

### Added

- Added a cross-language CRUD event system for generated JavaScript/TypeScript,
  Python, Java, and Rust clients, with before, after, and error phases for
  create, createMany, update, delete, and deleteMany operations.
- Generated clients now expose typed event contexts and registration APIs with
  shared per-operation state, transaction metadata, handler priorities, and
  stop-propagation/default-result handling.
- Java code generation now emits event annotations and an `EventRegistry` wired
  through generated clients, while Rust code generation now emits an events
  runtime and includes the new `nautilus-events-macros` workspace crate.

### Fixed

- Generated JavaScript/TypeScript and Python input types now preserve schema
  nullability for create, update, and where inputs.
- Nullable extension-backed fields now allow explicit `null`/`None` values in
  generated filter input expressions.

### Changed

- Python composite types are now emitted as `TypedDict` declarations instead of
  dataclasses.
- Updated the JavaScript/TypeScript install command to the
  `@y0gm4/nautilus-orm` package scope.
- Refreshed lockfile dependencies, including `rkyv`/`rkyv_derive` 0.8.16,
  `getrandom` 0.3.4, and the new `hashbrown` 0.17.1 entry.

## Version 1.2.3

### Added

- Added GitHub-backed CLI update checks and shared release metadata helpers.
- Added array default literal support across schema validation, DDL rendering,
  code generation, formatting, IR, and tests.

### Changed

- `@relation` now accepts a positional relation name in addition to the
  existing named argument form.
