# nautilus-engine

`nautilus-engine` is the JSON-RPC runtime used by generated multi-language clients.

It loads a validated schema, connects to a database, and serves requests on stdin/stdout.

## Supported RPC methods

| Category | Methods |
| --- | --- |
| Engine | `engine.handshake`, `engine.metrics`, `request.cancel` |
| Reads | `query.findMany`, `query.findFirst`, `query.findUnique`, `query.findFirstOrThrow`, `query.findUniqueOrThrow`, `query.explain` |
| Writes | `query.create`, `query.createMany`, `query.update`, `query.updateMany`, `query.upsert`, `query.delete`, `query.deleteMany` |
| Aggregation | `query.count`, `query.groupBy`, `query.aggregate` |
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
- MySQL isolation overrides apply only to the requested transaction. The engine
  uses the connector's shared transaction opener to set isolation before `BEGIN`
  and discard connections whose preparation fails or is cancelled. See the
  [connector integration tests](../nautilus-connector/README.md#integration-test-strategy)
  for the real MySQL isolation and connection reuse checks.
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
  All four generated clients express them: JavaScript and Python pass the
  operator object, Java gains a setter per operator (`viewsIncrement(5)`), and
  Rust carries the operator in the update input's type.
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

The engine maps protocol isolation levels to connector levels through the
exhaustive `connector_isolation_level` function in `state/transactions.rs`.
Adding a level requires updating that boundary, the protocol's wire contract,
and the connector's SQL handling. The connector stays independent of the
protocol; `tests/mysql_transaction_tests.rs` checks every protocol level against
the server's effective isolation.

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

Paths below are relative to `src/`.

| Module | Responsibility |
| --- | --- |
| `args.rs`, `pool_options.rs` | Standalone arguments and pool/runtime options |
| `handlers/mod.rs`, `handlers/request.rs`, `handlers/embedded.rs` | Wire dispatch, request/model resolution and Rust in-process adapters |
| `handlers/service.rs`, `handlers/transactions.rs` | Handshake, metrics, schema validation and transaction request handlers |
| `handlers/crud/read/` | Shared planning and ordering, buffered reads, streaming, count and explain |
| `handlers/crud/write/` | Per-operation writes, shared input rules and same-connection read-back |
| `handlers/crud/nested/` | Operation parsing, relation binding and execution for owning, inverse and many-to-many relations |
| `handlers/crud/include.rs`, `handlers/crud/aggregation.rs`, `handlers/crud/raw.rs` | Relation hydration, aggregate/group queries and raw execution |
| `filter/` | JSON and typed argument adapters, shared checks, predicates, ordering and includes |
| `metadata/` | Cached field hints, logical/physical names and relation maps |
| `conversion/` | Request values, row normalization/serialization, extensions and composite literals |
| `state/` | State construction, database clients, statement execution and transaction lifetime |
| `plan_cache.rs`, `metrics.rs`, `observability.rs` | Cached read plans, measurements and tracing configuration |
| `transport.rs` | Stdin/stdout concurrency, response delivery and request cancellation |

The [query-operation route](../../CONTRIBUTING.md#add-a-query-operation) connects
these owners to protocol types, SQL rendering and generated client APIs. Shared
scalar coercions live in the connector; the engine maps their errors to its
protocol contract. Metadata construction does not depend on request handlers.

Runtime equivalence with generated Rust clients is tested by codegen's
[`path_equivalence_tests`](../nautilus-codegen/tests/path_equivalence_tests.rs),
which reuses this crate's SQLite fixture. The
[coverage matrix](../nautilus-codegen/README.md#testing) identifies the adapters
and client modes compared for each contract.

## Dependencies in the workspace

- `nautilus-schema` for parsing and validated schema metadata
- `nautilus-core` for query AST types
- `nautilus-dialect` for SQL rendering
- `nautilus-connector` for execution
- `nautilus-migrate` for optional startup DDL application
- `nautilus-protocol` for wire-format types
