# Changelog

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
