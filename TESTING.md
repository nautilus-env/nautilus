# Finding tests for a feature

Start with the row matching the behavior being changed. Schema tests establish
what the language accepts and lowers; SQL tests cover rendering and migration
plans; engine tests execute requests; client tests check generated APIs and
their runtime behavior. A source snapshot alone does not establish that a
generated client compiles or that a database accepts its SQL.

Paths below are relative to the repository root. The table points to existing
coverage, not a requirement to add the same assertion at every layer.
The [contributor guide](CONTRIBUTING.md) identifies implementation owners and
the routes for adding a scalar type, query operation or schema attribute.

| Feature | Schema | SQL and migrations | Engine | Generated clients |
| --- | --- | --- | --- | --- |
| Scalar types, null, defaults and mapped names | [validation](crates/nautilus-schema/tests/validation_tests.rs), [IR](crates/nautilus-schema/tests/ir_tests.rs) | [DDL](crates/nautilus-migrate/tests/ddl_tests.rs), [diff](crates/nautilus-migrate/tests/diff_tests.rs), [pull](crates/nautilus-migrate/tests/serializer_tests.rs), [identifier round-trip](crates/nautilus-migrate/tests/sqlite_identifier_roundtrip.rs) | [schema-aware reads](crates/nautilus-engine/tests/schema_aware_read_tests.rs), [conversions](crates/nautilus-engine/tests/conversion_tests.rs) | [models](crates/nautilus-codegen/tests/snapshot/models.rs), [inputs](crates/nautilus-codegen/tests/snapshot/inputs.rs), [type mappings](crates/nautilus-codegen/tests/type_helpers_tests.rs), [path equivalence](crates/nautilus-codegen/tests/fixtures/path_equivalence/) |
| Composite and extension types, vector search | [validation](crates/nautilus-schema/tests/validation_tests.rs), [IR](crates/nautilus-schema/tests/ir_tests.rs) | [DDL](crates/nautilus-migrate/tests/ddl_tests.rs), [PostgreSQL extensions](crates/nautilus-migrate/tests/postgres_extensions_e2e.rs), [dialect](crates/nautilus-dialect/tests/dialect_tests.rs) | [filters](crates/nautilus-engine/tests/filter_tests.rs), [schema-aware reads](crates/nautilus-engine/tests/schema_aware_read_tests.rs) | [inputs](crates/nautilus-codegen/tests/snapshot/inputs.rs), [extensions](crates/nautilus-codegen/tests/snapshot/extensions.rs), [vector](crates/nautilus-codegen/tests/snapshot/vector.rs) |
| Relations, includes and many-to-many | [relation validation](crates/nautilus-schema/tests/validation_tests.rs), [many-to-many](crates/nautilus-schema/tests/many_to_many_tests.rs) | [many-to-many](crates/nautilus-migrate/tests/many_to_many_tests.rs), [pull](crates/nautilus-migrate/tests/serializer_tests.rs) | [includes](crates/nautilus-engine/tests/include_tests.rs) | [relations](crates/nautilus-codegen/tests/snapshot/relations.rs), [path equivalence](crates/nautilus-codegen/tests/fixtures/path_equivalence/) |
| Filters, projections, ordering and aggregates | [IR](crates/nautilus-schema/tests/ir_tests.rs) | [dialect](crates/nautilus-dialect/tests/dialect_tests.rs) | [filters](crates/nautilus-engine/tests/filter_tests.rs), [single-row reads](crates/nautilus-engine/tests/find_first_tests.rs), [grouping](crates/nautilus-engine/tests/group_by_advanced_tests.rs), [aggregation](crates/nautilus-engine/tests/aggregation_and_raw_sql_tests.rs) | [queries](crates/nautilus-codegen/tests/snapshot/queries.rs), [filters](crates/nautilus-codegen/tests/snapshot/filters.rs), [TypeScript consumer](crates/nautilus-codegen/tests/fixtures/stream_runtime_e2e/) |
| Writes, upsert and nested operations | [relation and default validation](crates/nautilus-schema/tests/validation_tests.rs) | [dialect](crates/nautilus-dialect/tests/dialect_tests.rs) | [nested writes](crates/nautilus-engine/tests/nested_write_tests.rs), [atomic updates](crates/nautilus-engine/tests/atomic_update_tests.rs), [create-many](crates/nautilus-engine/tests/create_many_tests.rs), [mutation filters](crates/nautilus-engine/tests/mutation_filter_shape_tests.rs) | [writes](crates/nautilus-codegen/tests/snapshot/writes.rs), [inputs](crates/nautilus-codegen/tests/snapshot/inputs.rs), [path equivalence](crates/nautilus-codegen/tests/fixtures/path_equivalence/) |
| Streaming and chunked reads | Uses the validated model IR | [dialect](crates/nautilus-dialect/tests/dialect_tests.rs), [connector streaming](crates/nautilus-connector/tests/) | [streaming reads](crates/nautilus-engine/tests/streaming_find_many_tests.rs) | [streaming APIs](crates/nautilus-codegen/tests/snapshot/streaming.rs), [runtime E2E](crates/nautilus-codegen/tests/stream_runtime_e2e_tests.rs) |
| Events, transactions and engine configuration | [generator validation](crates/nautilus-schema/tests/validation_tests.rs) | [connector transactions](crates/nautilus-connector/tests/), [partial migration application](crates/nautilus-migrate/tests/apply_phases_e2e.rs) | [batch](crates/nautilus-engine/tests/transaction_batch_tests.rs), [timeout](crates/nautilus-engine/tests/transaction_timeout_tests.rs), [MySQL isolation](crates/nautilus-engine/tests/mysql_transaction_tests.rs) | [events](crates/nautilus-codegen/tests/snapshot/events.rs), [clients](crates/nautilus-codegen/tests/snapshot/clients.rs), [runtime options](crates/nautilus-codegen/tests/snapshot/runtime.rs), [macro diagnostics](crates/nautilus-events-macros/tests/ui/), [compiled consumers](crates/nautilus-codegen/tests/writer_tests.rs) |
| Views and database schemas | [views](crates/nautilus-schema/tests/view_tests.rs), [multi-schema](crates/nautilus-schema/tests/multi_schema_tests.rs) | [views](crates/nautilus-migrate/tests/view_tests.rs), [multi-schema](crates/nautilus-migrate/tests/multi_schema_tests.rs) | [views](crates/nautilus-engine/tests/view_tests.rs) | [writer](crates/nautilus-codegen/tests/writer_tests.rs), [Python import compatibility](crates/nautilus-codegen/tests/stream_runtime_e2e_tests.rs) |
| Schema syntax, imports and generated import order | [parser](crates/nautilus-schema/tests/parser_tests.rs), [schema sets](crates/nautilus-schema/tests/schema_set_tests.rs), [editor analysis](crates/nautilus-schema/tests/analysis_tests.rs) | Consumes validated IR | [schema validation](crates/nautilus-engine/tests/schema_validate_tests.rs) | [schema discovery](crates/nautilus-codegen/tests/parse_schema_tests.rs), [imports](crates/nautilus-codegen/tests/snapshot/imports.rs) |

Wire contracts have their own feature targets under
[protocol/tests](crates/nautilus-protocol/tests/): read, write, aggregate, raw,
transaction, schema and engine methods. Provider decoding and binding are
covered by the PostgreSQL, MySQL and SQLite targets in
[connector/tests](crates/nautilus-connector/tests/), with private codec tests
next to their implementations.

## Fixtures and private tests

Reuse the existing [schema](crates/nautilus-schema/tests/common/mod.rs),
[migration](crates/nautilus-migrate/tests/common/mod.rs) and
[engine](crates/nautilus-engine/tests/common/mod.rs) `tests/common` modules for
their respective parsing and database setup. Engine fixtures create an isolated
SQLite database and apply the schema's DDL. The generated Rust equivalence
consumer also includes that engine helper; its scenario modules and shared
schema live in [path_equivalence](crates/nautilus-codegen/tests/fixtures/path_equivalence/).

Codegen's reusable input schemas live in
[fixtures/schemas](crates/nautilus-codegen/tests/fixtures/schemas/).
Keep language-specific generator blocks in the test when the model content is
shared, as in [nested writes](crates/nautilus-codegen/tests/snapshot/writes.rs).
Executable Python, JS/TS and Java consumers live in
[stream_runtime_e2e](crates/nautilus-codegen/tests/fixtures/stream_runtime_e2e/).
Their harness checks toolchain prerequisites and runs the generated output.

Keep a small schema inline when it explains one rejection or edge case; share
fixture content when cases need to agree on the same model. Unit tests that
need private functions remain in their implementation module or its `tests.rs`
child. Moving them must not require exposing runtime internals or replacing a
database assertion with a source-string assertion.

## Running a focused check

For example, changing includes calls for the engine suite and the relevant
generated-client contracts:

```bash
cargo test --locked -p nautilus-orm-engine --test include_tests
cargo test --locked -p nautilus-orm-codegen --test snapshot_tests snapshot::relations::
cargo test --locked -p nautilus-orm-codegen --test path_equivalence_tests
```

`snapshot_tests` remains one Cargo target. Its feature modules live in
[tests/snapshot](crates/nautilus-codegen/tests/snapshot/); function names are
preserved under `snapshot::<feature>::`, so use that full path with `--exact`.
Baseline names and generated payloads do not depend on the module path.
Set `INSTA_UPDATE=no` to prohibit snapshot writes. For an intentional output
change, update only the affected cases, review the baseline diff, then verify
with updates disabled. See the [codegen testing guide](crates/nautilus-codegen/README.md#testing)
for compiled consumers, runtime prerequisites and PowerShell equivalents.

After moving cases, compare `cargo test ... -- --list` before and after and run
the affected targets. A renamed module must not hide cases behind a stale test
filter or drop a target from CI. Run formatting and Clippy for the change;
provider behavior needs the corresponding real-database target as well.
The [CI workflow](.github/workflows/ci.yml) owns the full matrix, including
MSRV, Windows/macOS, generated-client consumers and provider services.
CI sets `NAUTILUS_REQUIRE_E2E=1` for runtime and migration E2E prerequisites;
when running locally without it, inspect skip messages with `--nocapture`.
