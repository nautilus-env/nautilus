# Adding a feature to Nautilus

Start with the [workspace map](README.md#workspace-map) for crate responsibilities
and dependencies, then follow the relevant route below. Paths are relative to
the repository root. The [test map](TESTING.md) links each feature to existing
schema, SQL, engine and generated-client coverage.

## Ownership boundaries

`nautilus-schema` owns syntax, validation and `SchemaIr`, without database or
client runtime dependencies. `nautilus-core` owns query expressions, builders
and values, independently of the engine, connector and migrations. The connector
binds and decodes database values; the engine adapts requests and applies
schema-aware rules. Dialects render query SQL; migrate owns DDL, introspection,
comparison and application plans.

Codegen consumes `SchemaIr`. Its `ModelView` holds shared field/model facts and
`ExtensionRegistry` selects declared extension wrappers. Language backends own
output types, imports, literals and syntax. CLI commands coordinate these
components; the LSP serves schema analysis and manages document state. Event
attribute expansion belongs to the macro crate; event dispatch belongs to the
generated client runtime.

Keep new implementation modules private or `pub(crate)` and preserve existing
public paths through their facades. A filesystem module split does not require
a new public API. Check generated clients and benchmarks before changing an
existing export, including one hidden from rustdoc.

## Add a scalar type

1. **Define the language and its constraints.** Extend `FieldType` in
   [ast/types.rs](crates/nautilus-schema/src/ast/types.rs), parse its spelling and
   parameters in [parser/fields.rs](crates/nautilus-schema/src/parser/fields.rs),
   and define `ScalarType` and provider support in
   [ir/types.rs](crates/nautilus-schema/src/ir/types.rs). Validate parameters and
   extension requirements in `validator/fields.rs`, defaults in
   `validator/defaults.rs`, and lower the AST in `validator/ir_builder/fields.rs`.
   Update the shared completion/hover entry in `analysis/catalog/types.rs` and
   the [grammar](crates/nautilus-schema/GRAMMAR.md).
2. **Specify the database representation in both directions.** Add DDL mapping
   in [ddl/types.rs](crates/nautilus-migrate/src/ddl/types.rs), provider
   introspection in `inspector/`, comparison in `normalize/types.rs`, and pull
   inference in `serializer/types.rs`. The [migration guide](crates/nautilus-migrate/README.md#architecture)
   identifies forward/reverse change handlers when the type needs more than a
   new column mapping. Verify that push, pull and a second diff agree.
3. **Connect values to execution.** Reuse a `Value` representation where it is
   faithful; if adding a variant, follow the
   [core value guide](crates/nautilus-core/README.md#where-value-behavior-lives)
   for Rust conversions, tagged/plain codecs and owned/borrowed row decoding.
   Add binding and decoding in the supported
   [connector backends](crates/nautilus-connector/README.md#implementation-owners).
   PostgreSQL classifies once in `postgres/decode_plan.rs` before dispatching to
   its codec. Parameter casts belong to `nautilus-dialect/src/postgres.rs` or
   the corresponding renderer, not to the client template.
4. **Adapt schema-aware input and output.** In the engine, extend
   [metadata/fields.rs](crates/nautilus-engine/src/metadata/fields.rs) for field
   hints and `conversion/input.rs`, `conversion/normalize.rs` and
   `conversion/serialize.rs` for the affected direction. Shared scalar
   normalization lives in `nautilus-connector/src/value_hint/scalar.rs`; the
   engine retains its protocol error mapping. Define the wire form in the
   [protocol documentation](crates/nautilus-protocol/README.md#value-encoding-notes)
   when it adds a convention.
5. **Generate each language's API.** Rust scalar mappings live in
   [type_helpers.rs](crates/nautilus-codegen/src/type_helpers.rs); Python, JS and
   Java mappings live in their `src/<language>/backend.rs`, with full-field
   composition in `type_mapper.rs`. Review shared capabilities in `model_view.rs`
   and `backend.rs`, and extension selection in `extension_types.rs`. Update
   the language's field/decoder contexts and templates listed below; runtime
   conversion independent of a model belongs in the shared runtime.

Use the scalar or extension row of [TESTING.md](TESTING.md): parser/validation
and IR cases, DDL/diff/pull, a real provider round-trip, engine conversion, and
compiled generated consumers. Include null and collection behavior where
supported, plus precision-sensitive values. Codec or renderer changes should
also use the existing `value_serde`, `decode_rows`, `rows_json` or `render`
benchmark for the path actually changed.

## Add a query operation

1. **Define the request and result.** Add the method constant and serde payload
   to the relevant family under
   [protocol/src/methods](crates/nautilus-protocol/src/methods/), re-export through
   `methods/mod.rs`, and update the protocol method matrix. Specify errors,
   defaults, transaction support and whether the result is rows, a count or
   chunks. `error.rs` and `version.rs` own error codes and version compatibility.
2. **Implement the operation behind its adapters.** Route the wire method in
   [handlers/mod.rs](crates/nautilus-engine/src/handlers/mod.rs); Rust in-process
   dispatch and typed entry points live in `handlers/embedded.rs`. Place query
   behavior in the relevant `handlers/crud/read/`, `write/`, `aggregation.rs`
   or `raw.rs` owner. `filter/json_args.rs` and `filter/typed_args.rs` adapt
   inputs, with common checks in `filter/validate.rs`. Keep both adapters on
   the same operation and preserve logical/physical field mapping.
3. **Reuse planning and execution.** Reads share `read/plan.rs`; streaming has
   its own consumer in `read/stream.rs`. Add SQL-independent query structure in
   [core](crates/nautilus-core/src/) and rendering in
   [dialect](crates/nautilus-dialect/README.md#architecture) only when existing
   expressions/builders cannot represent the operation. Rust argument-to-JSON
   conversion lives in `crates/nautilus-core/src/protocol_json/`. Use engine
   `state/execution.rs` and `state/transactions.rs` for connection choice and transaction lifetime;
   mutations share input rules in `write/input.rs` and read-back in
   `write/read_back.rs`.
4. **Expose the generated API.** Update language contexts, operation partials
   and template registration/assembly using the table below. Preserve Rust
   `EngineMode` routing and the sync/async and view surfaces. Reuse existing
   event dispatch for a write; changing Rust event attributes also touches
   [events-macros](crates/nautilus-events-macros/README.md).

Use the query/write row of [TESTING.md](TESTING.md). Check protocol payloads and
errors, rendered SQL and parameter order, real database results, and compiled
client calls. Extend `path_equivalence_tests` where RPC, embedded, typed and
Rust direct/engine modes promise the same behavior. For streaming, exercise
chunk boundaries and early cancellation in the existing runtime E2E suite.

## Add a schema attribute

1. **Parse and preserve syntax.** Add the field/model attribute shape to
   [ast/attributes.rs](crates/nautilus-schema/src/ast/attributes.rs) and parsing
   to `parser/fields.rs`; argument expressions belong to `parser/expressions.rs`.
   Keep spans and recovery behavior, review traversal in `visitor.rs`, and update
   `formatter.rs` and [GRAMMAR.md](crates/nautilus-schema/GRAMMAR.md).
2. **Validate and lower once.** Put the rule in the relevant `validator/` domain:
   field constraints in `fields.rs`, defaults in `defaults.rs`, relations in
   `relations.rs`, indexes in `index.rs`, with view/composite restrictions in
   their respective passes. Register a new pass once in
   `validator/mod.rs::run_validation_passes`. Add resolved data to the relevant
   `ir/` type and lower it in `validator/ir_builder/fields.rs` or `entities.rs`.
3. **Describe it to editor users.** Add syntax, snippet and documentation in
   [analysis/catalog/attributes.rs](crates/nautilus-schema/src/analysis/catalog/attributes.rs),
   which completion and hover share. Attribute argument suggestions belong to
   `analysis/completion/attribute_args.rs`. The LSP consumes this analysis;
   language rules remain in schema even for incomplete buffers.
4. **Update the consumers that give it meaning.** A storage attribute reaches
   migrate's DDL, live introspection, diff, reversal and `serializer/` output.
   A query attribute reaches engine metadata/planning or mutation input. A
   generated API property reaches codegen's `ModelView`, language contexts and
   the relevant templates. Follow these routes only for the semantics the
   attribute changes; its syntax alone does not require a wire method.

Use the relevant feature row of [TESTING.md](TESTING.md), with parser recovery
and spans, validation diagnostics/order, resolved IR, formatting and editor
analysis. Storage attributes need a real push/pull/diff check; generated API
changes need a compiled consumer as well as reviewed snapshots.

## Generated contexts, templates and runtimes

All paths in this table are relative to `crates/nautilus-codegen/`.

| Target | Schema-dependent contexts and templates | Shared runtime |
| --- | --- | --- |
| Rust | `src/generator/`; `templates/rust/model/`, `templates/rust/read/` and `templates/rust/delegate/`; register in `src/generator/templates.rs`, assemble in `templates/rust/model_file.tera` / `templates/rust/delegate.tera`, and list modular output in `src/generator/files.rs` | `templates/rust/runtime.rs.tpl`, `templates/rust/events.rs.tpl` |
| Python | `src/python/generator/`; `templates/python/model/`, `templates/python/input/` and `templates/python/delegate/`; register in `src/python/generator/templates.rs` and update `src/python/generator/files.rs` plus `templates/python/files/` for modular output | `templates/python/runtime/` |
| JS / TS | `src/js/generator/`; `templates/js/model/` for runtime and declarations from the same contexts; register in `src/js/generator/templates.rs` | Edit `templates/js/runtime/*.ts`, then derive `.js` / `.d.ts` with [tools/js-runtime](tools/js-runtime/README.md) |
| Java | `src/java/generator/`; `templates/java/delegate/` and `templates/java/dsl/`, assembled by `templates/java/delegate.java.tera` and `templates/java/dsl.java.tera`; register in `src/java/generator/templates.rs` | `templates/java/runtime/` and `templates/java/events/` |

See the [codegen layout guide](crates/nautilus-codegen/README.md#template-layout)
for the public single-file and modular output contracts. Layout belongs to
`src/writer/`, rendering to the backends, publication to `src/publish.rs`, and
installation to `src/install/`. Add a template to the registry that embeds it;
putting a file on disk alone does not make it part of a generated client.

## Verify the change

Run formatting, Clippy and the affected crate tests; use
[TESTING.md](TESTING.md#running-a-focused-check) for snapshot filters, real
consumers and prerequisites. Review intended baseline changes and rerun with
`INSTA_UPDATE=no`. The [CI workflow](.github/workflows/ci.yml) defines the full
workspace, MSRV, platform and database matrix for changes crossing those paths.
Keep these routes and the affected crate README current when moving an owner.
