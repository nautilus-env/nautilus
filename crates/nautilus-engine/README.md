# nautilus-engine

`nautilus-engine` is the JSON-RPC runtime used by generated multi-language clients.

It loads a validated schema, connects to a database, and serves requests on stdin/stdout.

## Supported RPC methods

| Category | Methods |
| --- | --- |
| Handshake | `engine.handshake` |
| Reads | `query.findMany`, `query.findFirst`, `query.findUnique`, `query.findFirstOrThrow`, `query.findUniqueOrThrow` |
| Writes | `query.create`, `query.createMany`, `query.update`, `query.upsert`, `query.delete` |
| Aggregation | `query.count`, `query.groupBy` |
| Raw SQL | `query.rawQuery`, `query.rawStmtQuery` |
| Transactions | `transaction.start`, `transaction.commit`, `transaction.rollback`, `transaction.batch` |
| Schema | `schema.validate` |

## Running it

Via the dedicated binary:

```bash
cargo run -p nautilus-orm-engine -- --migrate
```

Via the main CLI:

```bash
nautilus engine serve --migrate
```

If `--schema` is omitted, the engine auto-detects the first `.nautilus` file
in the current directory.

## Runtime notes

- `transactionId` is supported on request types that can run inside an open transaction.
- `query.findMany` also supports protocol-level chunking via `chunkSize`; partial responses are emitted before the final response when the client opts in.
- `query.upsert` runs as one atomic statement. Its `where` must name exactly the columns of one unique constraint (or the primary key), and `create` must supply a value for each of them.
- `query.update`, `query.updateMany` and the update half of `query.upsert`
  accept **atomic operators** in place of a value: `{"views": {"increment": 1}}`
  renders as `SET "views" = ("views" + $1)`, so the database derives the new
  value from the row's current one and two concurrent updates both land.
  `decrement`, `multiply` and `divide` work the same way and take `Int`,
  `BigInt`, `Float` and `Decimal` columns. `set` writes its operand as given and
  is accepted on `create` too. An arithmetic operator is refused on `create`
  (there is no current value), on a primary-key column (the new key is unknown
  until the statement has run, and the read-back on a backend without
  `RETURNING` looks for the key captured before it), and on a field whose type
  cannot take arithmetic. A field that holds structured JSON — `Json`, `Bytes`,
  a composite, any list — is never read this way: there the object is the value.
- `query.create` and `query.update` accept **nested writes**: a relation field in
  `data` carries an object of operations instead of a column value. The side of
  the relation that holds the foreign key takes `create`, `connect` and
  `connectOrCreate`, plus `update`, `disconnect` and `delete` on an update; the
  side pointed at takes `create`, `createMany`, `connect` and `connectOrCreate`,
  plus `disconnect`, `set`, `update`, `updateMany`, `delete` and `deleteMany` on
  an update. Operation names are accepted in the wire spelling and in
  snake_case. Every operation is scoped to the parent row, so a `where` inside
  one can only narrow the rows reached through the relation. A request without a
  `transactionId` gets a transaction for the whole call; one with a
  `transactionId` runs on it and leaves the commit to its owner. On
  `query.update` the filter must match exactly one row. All four generated
  clients expose them: JavaScript and Python forward `data` unchanged, Rust and
  Java carry a typed nested-write input per relation.
- On a backend without `RETURNING` (MySQL), `returnData: true` reads the written
  rows back on the same connection: `LAST_INSERT_ID()` for a generated key, the
  supplied key otherwise, and the primary keys captured before the statement for
  an update or a delete. `query.createMany` is the exception — a multi-row
  insert reports only the first generated key — and still answers with a count.
- `request.cancel` aborts the engine-side task only; the statement keeps running on the database. Use `--statement-timeout-ms` to bound it server-side.
- The engine owns schema-aware field mapping, relation hydration for includes, mutation-side `@updatedAt`, transaction timeout handling, and aggregate/raw-query execution.

## Diagnostics

Diagnostics are emitted on stderr through `tracing`; stdout is reserved for the
JSON-RPC stream.

| Variable | Effect |
| --- | --- |
| `NAUTILUS_LOG` | `tracing` filter directives, e.g. `nautilus_engine=debug`. Falls back to `RUST_LOG`; defaults to `nautilus_engine=info` |
| `NAUTILUS_SLOW_QUERY_MS` | Logs every statement running past this many milliseconds, with its SQL text and duration, on target `nautilus_engine::slow_query`. Unset or `0` disables it |

Per-request transaction lifecycle events are logged at `debug`, so
`NAUTILUS_LOG=nautilus_engine=debug` traces transaction start, commit and
rollback.

## Main modules

| Module | Responsibility |
| --- | --- |
| `args` | Standalone binary CLI parsing |
| `handlers` | RPC routing and method handlers |
| `filter` | JSON query args -> `nautilus-core` expressions |
| `observability` | Log subscriber setup and the slow-statement threshold |
| `state` | Schema metadata, connector client, transaction registry |
| `transport` | Stdin/stdout request loop |

## Dependencies in the workspace

- `nautilus-schema` for parsing and validated schema metadata
- `nautilus-core` for query AST types
- `nautilus-dialect` for SQL rendering
- `nautilus-connector` for execution
- `nautilus-migrate` for optional startup DDL application
- `nautilus-protocol` for wire-format types
