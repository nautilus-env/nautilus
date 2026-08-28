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
