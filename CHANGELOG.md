# Changelog

## Version 1.4.1

### ⚠️ Breaking — a model must declare its primary key

A model with no `@id` and no `@@id` was silently given its **first field** as
PRIMARY KEY, handing the schema a uniqueness constraint it never asked for and
that only showed up when a second row collided with it. Such a model is now a
validation error. A `view` is exempt: it names a relation the database owns,
which need not have a key.

### ⚠️ Breaking — stricter query input

Input that used to be accepted and quietly ignored is now rejected:

- an unknown key in `data` (`create`, `createMany`, `update`), an unknown
  relation in `include`, an unknown field in `select`, an unknown `orderBy`
  field, and an unknown RPC parameter (`skipDuplicates` among them);
- a nested `select` or `_count` inside an `include` entry, neither of which the
  include path implements;
- `chunkSize: 0`, a `take` outside 32-bit range, an enum value that is not one
  of the declared variants, and a JSON object or array written to a scalar
  field, which used to be stored verbatim (an update operator is now applied
  instead of stored, and is refused with a message that names it wherever it
  cannot apply);
- `avg` / `sum` over a non-numeric field, and `min` / `max` over an unordered
  one; `avg: true` / `sum: true` now expand to the numeric fields only;
- a `findUnique` filter that matches neither the primary key nor any unique
  constraint, and an empty `where` on the single-record `update` / `delete`
  (use `updateMany` / `deleteMany` for "every row");
- an unknown key in `args` itself, on a read as well as on `aggregate` and
  `groupBy`. The Prisma spelling of an aggregate (`_count`, `_avg`, `_sum`,
  `_min`, `_max`) used to leave a `groupBy` result with no aggregates at all;
  the error now names the argument Nautilus expects.

### Added

- **Atomic update operators.** `update`, `updateMany` and the update half of
  `upsert` accept `{"views": {"increment": 1}}` in place of a value, which
  renders as `SET "views" = ("views" + $1)`: the database derives the new value
  from the row's current one, so two concurrent updates both land where a
  read-modify-write in the client would lose one. `decrement`, `multiply` and
  `divide` work the same way, over `Int`, `BigInt`, `Float` and `Decimal`.
  `set` writes its operand as given and is accepted on `create` too. An
  arithmetic operator is refused on `create` (no current value to build on), on
  a primary-key column, and on a field whose type cannot take arithmetic; a
  field holding structured JSON still stores an operator-shaped object verbatim.

  All four generated clients express them. JavaScript and Python widen the
  update input's type to admit the operator object they already forwarded
  unchanged. Java gains one setter per operator beside the plain one
  (`viewsIncrement(5)`). **Breaking, Rust only:** a numeric column on
  `{Model}UpdateInput` is now `Option<NumericUpdate<T>>` rather than
  `Option<T>`, so `views: Some(5)` becomes `views: Some(5.into())` or
  `Some(NumericUpdate::Set(5))`; non-numeric columns are unchanged.
- `generate --install` installs the client into the shared
  `site-packages` / `node_modules` location. Without it, generation now only
  installs when the generator block names no `output` to import from, so two
  projects on one machine no longer overwrite each other's global client. The
  install path is printed as a warning when it happens.
- The generated JavaScript client ships a `package.json` (`"type": "module"`,
  `main`, `types`, `exports`), so it imports from a project that has not opted
  into ES modules.
- `having` accepts the field-first shape (`{"views": {"_sum": {"gt": 10}}}`)
  alongside the aggregate-first one, and a grouped column can now be filtered
  directly (`{"role": {"eq": "ADMIN"}}`).

### Changed

- `db pull` recovers `@default(autoincrement())` on SQLite, which it read out of
  the table's `CREATE` statement rather than `PRAGMA table_xinfo`.
- `db pull` recovers `@updatedAt` on MySQL, which records the column's
  `ON UPDATE CURRENT_TIMESTAMP` in `information_schema`. PostgreSQL and SQLite
  give the column a plain `CURRENT_TIMESTAMP` default and nothing else, so there
  it is still indistinguishable from `@default(now())` — see the CLI README.
- `DateTime` maps to `DATETIME(6)` on MySQL instead of `DATETIME`, which
  silently truncated sub-second precision. Existing MySQL columns are migrated
  by the next push.
- `@updatedAt` takes its first value from the column's `CURRENT_TIMESTAMP`
  default instead of the engine clock, so it is never older than a
  `@default(now())` sibling written by the same insert.
- Aggregate results decode to the same JSON type on every provider: `_count`
  and `_sum` over an integer are integers, `_avg` is a float, `_sum` over a
  `Decimal` is a decimal string, and `_min` / `_max` keep the field's own type.
- A `Boolean` column read from SQLite decodes to `true` / `false` instead of
  `1` / `0`.
- A connection failure reports the underlying cause (bad password, unknown
  database, unreachable host) instead of `pool timed out`.
- A JSON object written by a client keeps its key order, so a multi-key
  `orderBy` no longer has its keys alphabetised and its primary sort key
  silently demoted.

### Fixed

- `where: {field: null}`, `{eq: null}` and `{not: null}` now render
  `IS [NOT] NULL` instead of a comparison against `NULL` that never matched.
- `in: []` and `notIn: []` no longer render `IN ()`, which was a syntax error on
  PostgreSQL and MySQL.
- `contains` / `startsWith` / `endsWith` escape `%`, `_` and `\` in the search
  term, so a user-typed `%` matches itself instead of everything.
- A `String` field holding a UUID-shaped value can be filtered on PostgreSQL;
  the value is no longer bound as `uuid` against a `text` column.
- A `Decimal` returned by the engine can be written straight back on
  PostgreSQL.
- `distinct` collapses duplicates on SQLite and MySQL, which have no
  `DISTINCT ON`; the engine deduplicates the decoded rows and applies
  `take` / `skip` afterwards.
- `count`, `aggregate` and `groupBy` accept relation filters (`some` / `none` /
  `every`), which only `findMany` understood.
- `skip` without `take` no longer emits a bare `OFFSET`, a syntax error on
  SQLite.
- PostgreSQL `SET DEFAULT` keeps the case of the default literal, so a string
  default is no longer lower-cased into a different value and an enum default
  change no longer produces an unappliable statement. `db status` prints the
  default as written for the same reason.
- `ALTER TYPE … ADD VALUE` runs before and outside the migration transaction, so
  adding an enum variant and making it a column default in one push applies on
  PostgreSQL instead of failing with "unsafe use of new value".
- The `@updatedAt` column's `CURRENT_TIMESTAMP` default is reported by the diff,
  so a push no longer proposes dropping it on every run.
- A Python client generated with `interface = "sync"` refuses
  `async with Nautilus()` with a message that names the cause, instead of an
  unrelated `TypeError`. The README's example schema now sets
  `interface = "async"` to match its async examples, and its JavaScript import
  points at the generated `db/index.js`.
- `Loaded DATABASE_URL from .env` is only printed when a `.env` file exists.

## Version 1.4.0

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

- Added `@@schema("...")` for multi-schema PostgreSQL. A datasource declares the
  schemas it spans and every model and view names the one that owns it:

  ```text
  datasource db {
    provider = "postgresql"
    url      = env("DATABASE_URL")
    schemas  = ["app", "analytics"]
  }

  model Post {
    id    Int    @id @default(autoincrement())
    title String @unique
    @@map("posts")
    @@schema("app")
  }

  model PostSnapshot {
    id     Int    @id @default(autoincrement())
    title  String @unique
    bucket String
    @@map("posts")
    @@schema("analytics")
  }
  ```

  Tables are keyed on `(schema, name)` throughout, so two models may share a
  physical table name in different schemas. Query rendering qualifies only the
  table position of a statement — `FROM "analytics"."posts"` — and leaves column
  references bare, which every supported provider resolves through the implicit
  alias. Migrations emit `CREATE SCHEMA IF NOT EXISTS` before the first
  `CREATE TABLE` and never drop a schema; `db pull` introspects exactly the
  declared schemas and writes `schemas = [...]` and `@@schema` back out.

  `@@schema` is required on every block once `schemas` is declared rather than
  defaulting to the first entry, because `search_path` decides where an
  unqualified name lands at runtime. `schemas` is rejected for MySQL and SQLite.
  Exercised end to end against a live PostgreSQL by `examples/multi-schema`.

- Added implicit many-to-many relations. A relation declared as an array on
  both sides, with neither side naming `fields`/`references`, no longer needs a
  join model written by hand:

  ```text
  model Post {
    id   Int   @id @default(autoincrement())
    tags Tag[]
  }

  model Tag {
    id    Int    @id @default(autoincrement())
    posts Post[]
  }
  ```

  Nautilus synthesises the join table. It is called `_<A model>To<B model>` —
  `_PostToTag` here — or `_<relation name>` when the relation is named, and
  carries two columns `A` and `B`, `A` belonging to whichever side sorts first
  by `(model, field)`. Each is typed after the primary key it points at and
  cascades on delete and update, and the pair is the table's primary key, with
  an index on `B` so the relation reads as cheaply from either end. `db push`
  and migrations create and drop it like any other table.

  The join table is not part of any generated client: `Post` gets a `tags`
  list, `Tag` a `posts` list, and the nested writes of both are the way to make
  and break links — `create`, `createMany`, `connect`, `connectOrCreate` on a
  create, plus `disconnect`, `set`, `update`, `updateMany`, `delete` and
  `deleteMany` on an update. `connect` is idempotent: linking twice leaves the
  relation as it was rather than failing on the join table's key. The
  operations that reach existing children resolve the relation's members first
  and narrow to them, so a `where` of the caller's can only narrow the reach,
  never widen it to a row linked to another parent.

  Reads go through the join table in a single query: `include` (with `where`,
  `orderBy` and per-parent `take`/`skip`, and nesting under it), and
  `where: { tags: { some | none | every: ... } }`. The Rust client's generated
  `tags_some(...)` predicate carries the join table too, so the direct
  connector path compiles the same `EXISTS`.

  Both models need a single-field primary key, since each key has one column to
  land in; a composite one is a validation error that points at declaring the
  join table as a model instead. A model can hold a many-to-many with itself,
  which is the one case where the same relation `name` may appear twice inside
  one model. A many-to-many with a `view` is refused: a view is read-only, so
  its join table could never be created.

- Added `view` blocks. A `view` names a read-only relation the database owns —
  Nautilus queries it and never creates, alters, drops or writes to it:

  ```text
  view PublishedPost {
    id     Int    @id
    title  String
    views  Int
    author String

    @@map("published_posts")
  }
  ```

  A view reads exactly like a model: `findMany`, `findFirst`, `findUnique`,
  `where`, `orderBy`, `take`/`skip`, `count`, `aggregate`, `groupBy`,
  `stream_many` and `explain` all work against it. Every write method is
  refused by the engine with `'<View>' is a view and is read-only`, and the
  generated clients simply do not carry one: JavaScript and Python delegates
  have no `create`/`update`/`delete`/`upsert` attribute, and in Rust and Java
  calling one does not compile.

  Migrations leave a view alone. It never reaches DDL, `db push` neither
  creates nor drops it, and a live view is not proposed for deletion. Creating
  the view is the database's job — Nautilus only needs to be told its shape.

  `db pull` now emits a `view` block for every view it finds, so an
  introspected schema round-trips instead of losing them.

  Because a view carries no foreign key, it cannot take part in a relation: a
  relation field on a view, or a model relation pointing at one, is a
  validation error. `@@index`, `@@check`, `@default`, `@updatedAt`, `@computed`
  and `@check` are rejected on a view for the same reason — they describe
  storage a view does not have.

  `view` is now a keyword, so a model, field or enum named `view` has to be
  renamed or `@map`ped.

- Added nested writes. A relation field named in the `data` of `query.create` or
  `query.update` now carries an object of operations instead of a column value,
  and the whole call — parent row, related rows, and the foreign keys between
  them — runs as one atomic write:

  ```json
  { "model": "User",
    "data": { "email": "ada@example.com",
              "posts": { "create": [{ "title": "on engines" }] },
              "team":  { "connect": { "name": "analytical" } } } }
  ```

  Which operations a relation accepts depends on where the foreign key lives.
  The side that holds it takes `create`, `connect` and `connectOrCreate`, plus
  `update`, `disconnect` and `delete` on an update. The side pointed at takes
  `create`, `createMany`, `connect` and `connectOrCreate`, plus `disconnect`,
  `set`, `update`, `updateMany`, `delete` and `deleteMany` on an update. Names
  are accepted in the wire spelling and in snake_case, so a Python caller can
  write `connect_or_create`. A nested `create` payload may itself carry nested
  writes.

  Every operation is scoped to the row it hangs off: a `where` inside
  `updateMany` or `deleteMany` can only narrow the rows reached through the
  relation, never reach rows belonging to another parent. `update` and `delete`
  report `RecordNotFound` when they match nothing, rather than passing
  silently.

  A request that already carries a `transactionId` uses it and leaves the
  commit to its owner; otherwise the engine opens a transaction for the call and
  commits it, so a failing child rolls the parent back. On `query.update` the
  filter has to match exactly one row, since children have to be linked to a
  single parent key; a filter matching several is refused before anything is
  written.

  All four generated clients accept nested writes. JavaScript and Python forward
  `data` unchanged; Rust and Java get a typed nested-write input per relation.

  In Rust, `{Model}CreateInput` and `{Model}UpdateInput` grow a field per
  relation, holding a `{Model}{Relation}CreateNested` / `…UpdateNested` whose
  own fields are the operations. Filters are `nautilus_core::Expr`, payloads are
  the target's own input type, and `ConnectOrCreate` / `NestedUpdate` pair a
  filter with one:

  ```rust
  authors.create(AuthorCreateInput {
      email: Some("ada@example.com".to_string()),
      books: AuthorBooksCreateNested {
          create: vec![BookCreateInput { title: Some("on engines".to_string()), ..Default::default() }],
          connect: vec![Book::title().eq("on looms")],
          ..Default::default()
      },
      ..Default::default()
  })?;
  ```

  Nested writes span several statements in one transaction, which the direct
  connector path cannot run, so a create or update carrying one goes through the
  embedded engine regardless of `EngineMode`, and reports a clear error when the
  client is configured with `EngineMode::Never`. `upsert` refuses them: it
  writes one row with a single statement and has no place to run them from.

  In Java, `CreateInput` and `UpdateInput` grow a builder method per relation,
  taking a `Consumer` of the relation's nested-write builder:

  ```java
  db.author().create(data -> data
      .email("ada@example.com")
      .books(books -> books
          .create(book -> book.title("on engines"))
          .connect(book -> book.title("on looms"))));
  ```

- Added `import "<path>"` to the schema language. A schema file joins other
  files to itself by naming them, relative to its own directory; the path may
  be a `.nautilus` file or a directory of them. Imports are followed
  transitively, a file reached twice is joined once, and cycles terminate. A
  path that does not resolve is an error reported on the `import` line, and the
  files that were reached are still validated. Every command that loads a
  schema follows imports, so `--schema user.nautilus` now loads what that file
  imports as well; pointing `--schema` at a directory keeps working unchanged.

- A schema with more than one `datasource` block, or more than one `generator`
  block, is now rejected instead of silently using the first one. The error is
  reported on the second block and names the one that already claimed the slot.
  With `import` this is the guard that catches a path reaching another schema's
  root file rather than the shared models it was meant to pull in.

- `nautilus-lsp` understands multi-file schemas. An open document is assembled
  with everything it imports before being analysed, so a reference to a model or
  enum declared in another file resolves instead of being reported as unknown,
  completion offers imported declarations, and go-to-definition jumps into the
  file that holds the declaration. Diagnostics are published to the file each
  one belongs to rather than all landing on the open document. Unsaved buffers
  of imported files are used in place of what is on disk, editing an imported
  file re-analyses the documents that import it, and the server registers a
  `**/*.nautilus` watcher so a change made outside the editor is picked up. A
  file that imports nothing is still analysed on its own: sibling files are
  never joined implicitly.

- Added `query.updateMany` and `query.deleteMany`, engine methods that apply a
  filter to every matching row and answer with the affected-row count alone. No
  `RETURNING` projection is ever emitted, so the statement stays one round-trip
  regardless of how many rows it touches. `query.update` and `query.delete` keep
  their existing behaviour, including `returnData`.
- Added `query.aggregate`, which computes `_count` / `_avg` / `_sum` / `_min` /
  `_max` over the whole filtered set without a grouping key. It takes the same
  aggregate arguments as `query.groupBy` minus `by` and `having`, and always
  returns exactly one row — previously the same result needed a `groupBy` with a
  synthetic key. A request naming no aggregate at all is rejected.
- Added `query.explain`, which renders the statement a `findMany` with the given
  arguments would run and hands it to the database's own `EXPLAIN`
  (`EXPLAIN (FORMAT JSON)` on PostgreSQL, `EXPLAIN FORMAT=JSON` on MySQL,
  `EXPLAIN QUERY PLAN` on SQLite). The response carries the rendered SQL, the
  bound parameters in placeholder order, and the plan rows, so the generated SQL
  is inspectable without falling back to `rawQuery`. `analyze` runs the
  statement for real timings where the backend supports it.
- Added `engine.metrics`, a snapshot of the engine's runtime counters: plan
  cache entries, hits, misses and evictions per section; pool size and idle
  connections; open interactive transactions; and per-method call, error and
  latency counters. Passing `reset` zeroes the cumulative counters after reading
  them. Until now none of this was observable from outside the process — the
  plan cache in particular could degrade with no external signal.
- The generated Rust, Java, JS/TS and Python clients now expose the engine
  methods above. Each model delegate gains `updateMany` (affected-row count,
  never a `RETURNING` projection), `aggregate` (one row over the whole filtered
  set) and `explain`; the Rust client spells the count-only delete
  `delete_many_count`, since its `delete_many` already reads the deleted rows
  back. `deleteMany` now calls `query.deleteMany` on its default, non-returning
  path instead of `query.delete`. The client object itself gains `$metrics` (JS),
  `metrics` / `sync_metrics` (Python), `metrics` (Java, Rust).
- Added `query.upsert`, a native engine method that performs an upsert as a
  single atomic statement: `INSERT ... ON CONFLICT ... DO UPDATE` on PostgreSQL
  and SQLite, `INSERT ... ON DUPLICATE KEY UPDATE` on MySQL. The generated
  Python, JS/TS and Java clients now call it directly, and the Rust client uses
  it whenever the embedded engine is available. Previously every client composed
  `upsert` from a lookup followed by an update or a create, which left a race
  window between the two round-trips. The request's `where` must name exactly
  the columns of one unique constraint (or the primary key) — that column list
  becomes the conflict target — and `create` must supply a value for each of
  them; anything else is rejected with a filter error instead of silently
  matching another index. MySQL has no `RETURNING`, so the engine reads the row
  back with a follow-up select when `returnData` is set: only the read is a
  second round-trip, the write stays atomic.
- Added a server-side statement timeout, configurable per datasource through
  `ConnectorPoolOptions::statement_timeout`, `EnginePoolOptions::statement_timeout_ms`,
  the engine's `--statement-timeout-ms` flag, and the `statementTimeoutMs` /
  `statement_timeout_ms` pool option on the generated JS and Python clients. It
  maps to `statement_timeout` on PostgreSQL (sent in the startup packet) and to
  `max_execution_time` on MySQL (set once per pooled connection); SQLite has no
  equivalent and ignores it. This is the only cancellation that reaches the
  database: `request.cancel` aborts the engine-side task but leaves a running
  query holding its connection, which is now stated in the documentation of both
  the protocol method and the transport handler.
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
- Added the `onUpdateMany` CRUD hook to all four generated clients, closing the
  asymmetry with `deleteMany`, which has exposed `onDeleteMany` since events
  were introduced. `updateMany` now runs the same before/after/error chain as
  every other write: `Model.onUpdateMany` in JS/TS and Python, the
  `@OnUpdateMany` annotation in Java, and `#[on_update_many(Model)]` — backed by
  `EventRegistry::on_update_many` and `CrudOperation::UpdateMany` — in Rust. The
  handler context carries the affected-row count as its result, and a
  `StopPropagation` carrying no result defaults to a count of `0`. `aggregate`
  and `explain` stay deliberately event-free, consistent with `count`, `groupBy`
  and `findMany`.
- Added multi-file schemas: pointing `--schema` at a **directory** assembles
  every `.nautilus` file in it, in lexicographic order, into one schema.
  Declaration order across files does not matter — a model in `post.nautilus`
  may reference an enum in `enums.nautilus` — and diagnostics still name the
  file, line and column the developer wrote rather than an offset into the
  assembled source. `nautilus format` on a directory formats each file
  separately. Pointing `--schema` at a single file, and omitting it altogether,
  behave exactly as before.
- Added `@ignore` and `@@ignore`, which declare that a column or a table exists
  in the database but that Nautilus does not manage it. An ignored declaration
  is left out of the generated client and out of every migration: it is never
  created, altered or dropped, and neither are the indexes, foreign keys and
  `CHECK` constraints that reference it. This is what makes `db pull` safe to
  push back on a database Nautilus did not create — a column whose type has no
  Nautilus spelling used to round-trip as `String`, so the next `db push`
  proposed rewriting `interval`, `money` or `int4range` to `text`. `db pull` now
  emits `@ignore` on such columns, and `@@ignore` on a table whose primary key
  or whose required column with no default is one of them.
- Added partial indexes: `@@index([...], where: <predicate>)` renders a
  `CREATE INDEX ... WHERE <predicate>` so the index only covers the matching
  rows. The predicate uses the same boolean expression language as `@check` /
  `@@check` and is resolved against physical column names, so `@map`-ed fields
  work. PostgreSQL and SQLite support it; MySQL has no equivalent syntax and the
  validator rejects the attribute there rather than silently widening the index.
  `db pull` reads the predicate back (`pg_index.indpred` on PostgreSQL, the
  stored `CREATE INDEX` text on SQLite) and writes it out as `where:`, and the
  diff compares predicates in normalised form so a database that re-renders its
  own predicate does not produce an endless drop/recreate cycle.
- Added `nautilus_core::ExtensionScalar`, the marker a generated client
  implements on its PostgreSQL extension wrappers (pgvector, PostGIS, citext,
  hstore, ltree) to opt them into array encoding and decoding. The orphan rule
  keeps a generated crate from writing those conversions itself, so they live in
  `nautilus-core` keyed on this marker.
- Schema tooling now documents the declarations this release adds. Hovering
  `schemas` in a datasource, `@@schema("...")` on a model or a view, `@ignore`
  on a field or `@@ignore` on a model explains what each one means, where it
  applies and what it does to migrations, with an example. The VS Code
  extension gains snippets for `view` and `type` blocks, a PostgreSQL
  datasource spanning several schemas, and `@@schema`, `@ignore`, `@@ignore`,
  `@check`, `@@check`, `@computed` and `@updatedAt`, and highlights `view`
  blocks and the `schemas` config key the way it already highlighted the rest.

### Changed

- Schema imports now validate their filesystem targets: a path may resolve to
  a `.nautilus` file or a directory containing `.nautilus` files. Missing paths,
  files with another extension, and directories without schema files report an
  error on the `import` declaration. LSP completion inside an import string
  reads the importing file's directory, offers folders as targets or for path
  navigation, filters out non-schema files, and inserts the selected relative
  path; `/` and `\\` also trigger another completion pass while navigating.
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
- The code generation backends now name a generated file with the shared
  `GeneratedFile` alias — a path relative to the output directory and its
  contents — instead of repeating `(String, String)` at every boundary, and
  `write_js_code` takes a single `JsOutput` struct in place of thirteen
  positional arguments, so the halves of the JavaScript output that may be
  absent are named at the call site. Generated output is unchanged; a caller of
  `write_js_code` outside the CLI has to pass the struct.

### Fixed

- The JavaScript and Python clients now prepare an `include` node the way they
  prepare the arguments of a top-level read. Both used to walk the include tree
  as a plain object, recursing into every value they found, so a nested
  `where` never reached the filter preparation the same `where` gets at the top
  level. Three things followed, two of them silently:

  ```js
  // the Date became {} on the wire, and the include matched nothing
  db.author.findMany({ include: { posts: { where: { publishedAt: { gt: new Date(...) } } } } });
  // `equals` reached the engine untranslated, which only knows `eq`
  db.author.findMany({ include: { posts: { where: { title: { equals: 'looms' } } } } });
  // a lone orderBy object was rejected: "orderBy must be an array"
  db.author.findMany({ include: { posts: { orderBy: { title: 'asc' } } } });
  ```

  A `Date`, a `Buffer` and any value with a `toWire()` were rebuilt field by
  field and lost, `equals` was never mapped to `eq`, mapped column names were
  left unresolved, and the guard that asks for `{ equals: ... }` on a JSON
  column never fired. An include node is now prepared by the model it loads —
  its own field map, its own serializers — and a lone `orderBy` object becomes
  the one-element list the engine expects, at every depth. The TypeScript
  declaration of a nested `orderBy` widens to `OrderByInput | OrderByInput[]`
  to match the top-level one it always accepted in fact.

  Rust and Java were unaffected: one passes a typed `IncludeRelation`, the other
  builds the node through its DSL, so neither could carry an unprepared value
  into one.

- The generated Rust client no longer loses the result of `create`, `update` and
  `delete` on MySQL. Without `RETURNING` the direct connector path gets no rows
  back from the statement, so a create failed with "Expected exactly one row,
  got 0" and an update or a delete answered with an empty list even though it
  had written. These three now go through the embedded engine on a dialect
  without `RETURNING`, which reads the written rows back on the connection that
  wrote them. PostgreSQL and SQLite are unaffected.

- `query.create`, `query.update` and `query.delete` now return the rows they
  wrote on MySQL. MySQL has no `RETURNING`, so the statement was rendered
  without one and `returnData: true` answered with `{"count": 0, "data": []}` —
  a generated JavaScript or Python client raised "returned no data" on every
  create. The engine now reads the affected rows back on the same connection,
  inside a transaction so the read cannot land on a different one:
  `LAST_INSERT_ID()` for a generated key, the supplied key otherwise, and the
  primary keys captured before the statement for an update or a delete. A create
  whose key is neither supplied nor an `AUTO_INCREMENT` column is refused with
  an error naming the column, instead of silently returning nothing. PostgreSQL
  and SQLite keep using `RETURNING` and are unaffected. `query.createMany` still
  returns no rows on MySQL: a multi-row insert reports only the first generated
  key, and MySQL does not guarantee the rest are contiguous.
- `db reset` and `db drop` no longer fail on SQLite when the tables reference
  each other. The drops run in a transaction, where SQLite ignores
  `PRAGMA foreign_keys`, so dropping tables in name order hit
  `FOREIGN KEY constraint failed` as soon as a child table outlived its parent.
  The statements now open with `PRAGMA defer_foreign_keys = ON`, which holds the
  checks until commit — by which point no table is left to reference another.
- `db pull` no longer writes `@check(...)` expressions that the schema parser
  rejects. A live `CHECK` the expression language cannot represent — `IS NULL`,
  `LIKE`, a function call, a typed literal such as `interval '0'` — used to be
  copied out as raw database text, and the pulled schema then failed to parse at
  all. Such a constraint is now emitted as a quoted raw predicate
  (`@check("uptime IS NULL OR uptime > interval '0'")`), which parses, formats
  and pushes back verbatim, so pulling and pushing an existing database stays a
  no-op. The same form is accepted in hand-written schemas as the escape hatch
  for provider-specific predicates in `@check`, `@@check` and
  `@@index(..., where:)`; nothing inside it is resolved, so its identifiers are
  database column names and `@map` does not apply to them.
- A string literal inside a `@check`, `@@check` or `@@index(where:)`
  expression now keeps an embedded quote when it is written out. The quote was
  rendered unescaped, so `label <> 'O''Reilly'` came back from `db pull` as
  `label <> 'O'Reilly'` — a schema that no longer parsed and SQL that no longer
  ran. Schema text and SQL both double the quote, and schema text additionally
  escapes backslashes, which start an escape sequence there but not in SQL.
- A parenthesised expression whose first parenthesis closes before the end is no
  longer mangled when a `CHECK` or `DEFAULT` is normalised. PostgreSQL reports
  `((notes IS NULL) OR (char_length(notes) > 3))`, and stripping "balanced"
  outer parentheses turned it into `notes IS NULL) OR (char_length(notes) > 3`,
  because the check only required the parentheses to balance overall. Quotes are
  honoured too, so a parenthesis inside a string literal no longer counts.
- `db pull` now rewrites a MySQL `CHECK` clause into executable SQL.
  `information_schema` reports it with every quote and backslash escaped one
  extra time and with a charset introducer on each literal
  (``upper(`code`) like _utf8mb4\'DEV-%\'``); pushing that text back failed
  with error 1064, and the introducer also stopped an `IN` list of literals from
  round-tripping as an expression.
- Boolean literals spelled `TRUE` / `FALSE` are now accepted in `@check`,
  `@@check` and `@@index(where:)` expressions. Both the formatter and `db pull`
  render booleans in that SQL-style upper case, but only the lower-case
  spellings were lexer keywords, so a formatted or pulled schema containing a
  boolean comparison failed to re-validate with "references non-existent field
  'TRUE'".
- `_avg` and `_sum` results now decode in the generated Rust client on
  PostgreSQL. PostgreSQL computes `AVG` — and `SUM` over exact numerics — as
  `numeric`, which the engine puts on the wire as a JSON string so no precision
  is lost; the client decoded that as a `String` and failed with a type error.
  This affected `group_by` as well as the new `aggregate`.
- The generated Rust client's engine-only operations (`count`, `groupBy`,
  `aggregate`, `updateMany`, `deleteMany`, `explain`) now run under
  `EngineMode::Auto` as well as `Always`. They have no direct-connector
  equivalent, so they only need to know that an engine may be built, not that
  the mode prefers the engine for simple CRUD.
- Fixed `db push` never converging on MySQL when a model has a `Boolean`
  `@default(...)`. The DDL generator wrote `DEFAULT TRUE`, but MySQL stores
  booleans as `tinyint(1)` and reports the default back as `1`, so every push
  compared `true` against `1` and re-applied the same `DEFAULT` change forever.
  MySQL now emits `1`/`0`, matching what it reports and what SQLite already did;
  PostgreSQL, which has a real boolean type, keeps `TRUE`/`FALSE`.
- Fixed `db pull` writing a schema that fails validation when a MySQL table has
  a boolean column with a default. `tinyint(1) DEFAULT '1'` was rendered as
  `Boolean @default(1)`, which the validator rejects as a type mismatch; a
  boolean column's `1`/`0` default is now rendered as `true`/`false`.
- Fixed `db pull` dropping `@default(autoincrement())` from MySQL keys. MySQL
  leaves `COLUMN_DEFAULT` empty on an `AUTO_INCREMENT` column, so there was no
  default expression to infer the key from the way PostgreSQL's `nextval(...)`
  allows, and pulling then pushing would have removed the attribute from the
  table. The pulled schema is now a faithful round-trip.
- Fixed `@default(autoincrement())` on MySQL, which was silently dropped: the
  DDL generator handled SQLite (`INTEGER PRIMARY KEY AUTOINCREMENT`) and
  PostgreSQL (`SERIAL`/`BIGSERIAL`) but had no MySQL branch, so the column was
  created as a plain `INT NOT NULL` and every insert that did not set the key
  explicitly failed with MySQL error 1364. MySQL primary keys now get
  `INT`/`BIGINT NOT NULL AUTO_INCREMENT`. `autoincrement()` outside a
  single-column primary key has no MySQL spelling and is now reported as a
  validation error instead of producing a table that rejects its own inserts.
- Fixed MySQL column rewrites stripping `AUTO_INCREMENT` from a primary key. A
  type, nullability, or default change restates the whole column through
  `MODIFY COLUMN`, and the restated definition omitted the attribute.
- `db push` now repairs a MySQL table created before the above fix. The live
  schema records whether a column is `AUTO_INCREMENT`, and a mismatch against
  the schema is reported as the new safe `Change::AutoIncrementChanged` and
  applied as a `MODIFY COLUMN`. The check is MySQL-only: PostgreSQL carries the
  same idea in the column's `nextval(...)` default and SQLite in the inline
  `INTEGER PRIMARY KEY AUTOINCREMENT`, neither of which can drift from the
  schema this way.
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
- The VS Code extension no longer highlights `cuid()` and `dbgenerated()` as
  built-in functions. The schema language has neither, so a schema using those
  names as anything else was coloured as if Nautilus knew them.

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
