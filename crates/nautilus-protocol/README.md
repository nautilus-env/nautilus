# Nautilus Protocol

`nautilus-protocol` defines the JSON-RPC 2.0 contract used by Nautilus clients and the Nautilus engine.

## Transport model

- Transport: line-delimited JSON over stdin/stdout
- Envelope: JSON-RPC 2.0
- Versioning: every request carries `protocolVersion`
- Current version: **1**
- All client requests must include `protocolVersion: 1`.

The crate contains typed request/response structs, method-name constants, and stable error-code definitions. The actual runtime encoding/decoding logic lives in the engine and generated runtimes.

## Method matrix

Each family has its own module under `src/methods/` and its own test file; `methods/mod.rs` re-exports every name, so the public paths do not mention the modules.

| Category | Module | Methods |
| --- | --- | --- |
| Engine | `engine` | `engine.handshake`, `engine.metrics`, `request.cancel` |
| Reads | `read` | `query.findMany`, `query.findFirst`, `query.findUnique`, `query.findFirstOrThrow`, `query.findUniqueOrThrow`, `query.explain` |
| Writes | `write` | `query.create`, `query.createMany`, `query.update`, `query.updateMany`, `query.upsert`, `query.delete`, `query.deleteMany` |
| Aggregation | `aggregate` | `query.count`, `query.groupBy`, `query.aggregate` |
| Raw SQL | `raw` | `query.rawQuery`, `query.rawStmtQuery` |
| Transactions | `transaction` | `transaction.start`, `transaction.commit`, `transaction.rollback`, `transaction.batch` |
| Schema | `schema` | `schema.validate` |

## Important cross-method fields

- `protocolVersion`: required on all public requests
- `transactionId`: optional on supported read/write methods
- `returnData`: optional on the row-returning mutations, defaults to `true`; `query.updateMany` and `query.deleteMany` accept it and ignore it, since they always answer with a count
- `chunkSize`: optional on `query.findMany`; lets the engine emit partial responses

## Minimal request example

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "query.findMany",
  "params": {
    "protocolVersion": 1,
    "model": "User",
    "args": {
      "where": {
        "email": {
          "contains": "example.com"
        }
      },
      "take": 10
    }
  }
}
```

## Value encoding notes

The stable wire contract uses ordinary JSON values plus a few conventions:

- decimals are carried as strings
- datetimes are RFC3339 strings
- UUIDs are strings
- bytes are base64 strings
- JSON values pass through as JSON

The exact conversion code is intentionally kept out of this crate so the method/type layer stays transport-focused.

## Implementation owners

`src/methods/` owns method names and payloads; `wire.rs` owns request/response
envelopes, `error.rs` owns stable errors, and `version.rs` owns compatibility
checks. This crate has no workspace runtime dependencies. Engine conversion
and generated-client codecs implement the values carried by those contracts.

The [query-operation route](../../CONTRIBUTING.md#add-a-query-operation) connects
a new method to dispatch, typed adapters, SQL execution and generated APIs.
Update the relevant family under `tests/` and use the
[test map](../../TESTING.md) for execution and client coverage.

## Error model

- JSON-RPC transport and parse errors still use standard JSON-RPC error envelopes.
- Domain-level errors are expressed through `ProtocolError` and converted to stable numeric error codes.
- Batch transaction failures preserve the failing sub-operation code and include structured context in `error.data`.

## Schema validation

`schema.validate` accepts a raw schema string and returns a success payload with:

- `valid = true` when analysis finds no error-level diagnostics
- `valid = false` plus `errors[]` when lexing, parsing, or validation fails

Invalid request shapes or unsupported protocol versions still return normal RPC errors.

## Testing

```bash
cargo test -p nautilus-orm-protocol
```
