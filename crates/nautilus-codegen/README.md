# nautilus-codegen

`nautilus-codegen` turns a validated `SchemaIr` into generated clients.

It is used both by the standalone `nautilus-codegen` binary and by the main `nautilus generate` command.

## Supported generator providers

| Provider | Output | Install behavior |
| --- | --- | --- |
| `nautilus-client-rs` | Rust model files, delegates, runtime helpers | `install = true` adds the output directory to the nearest Cargo workspace; `--standalone` also emits a generated `Cargo.toml` |
| `nautilus-client-py` | Python package with generated models and runtime files | Default workflow: import the generated `output` package directly. `install = true` copies the same generated files into Python `site-packages/nautilus`; it is a local install convenience, not a PyPI publish step |
| `nautilus-client-js` | JavaScript runtime plus TypeScript declaration files | Default workflow: import from the generated `output` directory. `install = true` copies the same generated files into the nearest `node_modules/nautilus`; it is a local install convenience, not an npm publish step |
| `nautilus-client-java` | Java Maven module with generated client, models, DSL, and runtime helpers | Default workflow: import the generated `output` Maven module from your build. Set `mode = "jar"` to also build a plain Java bundle under `output/dist/`. `install = true` is ignored for Java v1 |

The provider string in the schema selects which generator runs. The runtime
package/module name comes from the generated output location, or from the local
install target above when `install = true`.

## Public entry points

| Symbol | Purpose |
| --- | --- |
| `resolve_schema_path` | Auto-detects the first `.nautilus` file in the current directory unless one is passed explicitly |
| `generate_command` | Read -> parse -> validate -> generate -> write |
| `validate_command` | Read -> parse -> validate without writing output |
| `GenerateOptions` | Controls install, verbosity, and Rust standalone generation |

## The output directory

The generator owns the configured `output` path outright. Every generation
writes the complete tree into a staging directory beside it and then swaps the
two, so:

- a template that fails to render, or a write that fails part-way, leaves the
  previously generated client exactly as it was;
- files a previous generation produced but the current one does not are gone
  after the swap, and so is anything else placed in that directory by hand —
  keep hand-written code outside `output`;
- two generations running at once never share a staging directory, including
  the temporary one used when no `output` is configured and the package is only
  built to be installed.

An `output` path that is empty, names a filesystem root, or names an existing
file is refused rather than replaced.

## Current target notes

- The generator target is chosen entirely from the schema's `generator` block.
- Python supports `interface = "sync"` and `interface = "async"`.
- `recursive_type_depth` is currently validated only for the Python target.
- Rust generation can still emit bare sources for embedding into an existing workspace instead of forcing a standalone crate.
- JS generation writes both runtime files and `.d.ts` declarations so the generated package works in plain JS and TS projects.
- Java generation writes a Maven module rooted at the configured `output` path and requires Java-specific generator fields: `package`, `group_id`, and `artifact_id`. Java also supports `mode = "jar"`, which compiles the generated sources, writes `output/dist/{artifact_id}.jar` plus `output/dist/lib/*.jar` for plain `javac` / `java` workflows, and removes its temporary `.nautilus-build` directory before returning.
- The checked-in examples show the intended consumption pattern today: JS imports from `./generated/...`, Python imports from the generated output package on `sys.path`, Java imports from the generated Maven module or from the generated jar bundle, and `install = true` is optional.

## Choosing `findMany` vs streaming APIs

Generated clients expose both buffered and streaming read paths where the
runtime can benefit from it:

| Runtime | Buffered API | Streaming API | Recommended use |
| --- | --- | --- | --- |
| Rust async | `find_many` | `stream_many` | Use `find_many` for small/medium result sets you want as a final `Vec`; use `stream_many` for exports and long forward scans |
| Python async | `find_many` | `stream_many` | Same tradeoff as Rust; sync Python clients only expose `find_many` |
| JS / TS | `findMany` | `streamMany` | Prefer `streamMany` for `for await` pipelines and large result sets |
| Java sync + async | `findMany` | `streamMany` | Prefer `streamMany` when you want pull-based iteration; close early with `try`-with-resources |

As a rule of thumb, prefer the buffered APIs when you need the full collection
in memory anyway, especially for small pages, relation-heavy includes, or code
that naturally works on `Vec` / `List`. Prefer the streaming APIs when you want
to process rows incrementally, reduce client-side memory growth, or stop
consuming early once you have enough rows. Streaming keeps one pooled
connection occupied until iteration finishes, so it should be chosen
intentionally rather than used as the default for every `findMany`.

## Java bundle mode

Use `mode = "jar"` when you want `nautilus generate` to leave behind a bundle
that can be consumed without Maven or Gradle:

```prisma
generator client {
  provider    = "nautilus-client-java"
  output      = "db"
  package     = "com.example.db"
  group_id    = "com.example"
  artifact_id = "nautilus-client"
  mode        = "jar"
}
```

After generation you can compile and run plain Java code directly against the
bundle:

```powershell
javac --release 21 -cp "db\dist\nautilus-client.jar;db\dist\lib\*" Main.java
java -cp ".;db\dist\nautilus-client.jar;db\dist\lib\*" Main
```

## Template layout

| Area | Location |
| --- | --- |
| Rust templates | `templates/rust/` |
| Python templates | `templates/python/` |
| JS / TS templates | `templates/js/` |
| Java templates | `templates/java/` |
| In-memory output layout | `src/writer/` |
| Rust generator contexts | `src/generator/` |
| Python generator contexts | `src/python/generator/` |
| JS generator contexts | `src/js/generator/` |
| Java generator contexts | `src/java/generator/` |

Each generator's `mod.rs` assembles the output and preserves the existing public
entry points. Its private `templates.rs` registers embedded templates. Contexts
stay with the code that constructs them:

- Rust separates scalar fields and read hints (`fields`), cursor/unique/vector
  metadata (`keys`), composite ordering (`ordering`), and relation hydration and
  nested writes (`relations`).
- Python and JS separate field/input/filter contexts (`fields`), relation and
  include contexts (`relations`), language type expressions and enum/composite
  declarations (`types`), client imports (`client`), and embedded runtime files
  (`runtime`). JS produces runtime code and declarations from the same contexts.
- Java separates package settings (`config`), enum/model/composite records
  (`records`), partial records (`projections`), JSON decoding expressions
  (`readers`), query and input builders (`dsl`), model operations (`delegate`),
  client/Maven output (`client`), and runtime/event files (`runtime`).

To add a semantic field property, define it in `src/model_view.rs` and consume it
from the relevant language contexts. It owns shared facts about required create
values, generated defaults, numeric aggregates, arithmetic updates, relations
and vector fields. Extension availability and wire representation belong to
`src/extension_types.rs` and its `ExtensionRegistry`. Names, imports, type
expressions, defaults rendered as literals, and template syntax belong to each
backend.

Input exposure remains a backend decision: Rust permits overriding `now()`;
Python exposes all create fields with requiredness metadata; JS omits generated
create fields; Java also excludes `updatedAt` fields. These distinctions must be
preserved when sharing a new field classification. Update the relevant context
and template, then verify `snapshot_tests` and a compiled/runtime consumer from
`writer_tests`, `path_equivalence_tests`, or `stream_runtime_e2e_tests`.

Rust's `templates/rust/delegate.tera` assembles partials under `delegate/` for
nested and scalar input, aggregate types, filters, reads, projections, streaming,
and individual write operations. `model/` separates imports and row decoding;
`read/` separates ordering, builder configuration, and execution. A write names
its operation, event args and payload and hands its body to
`events::run_with_crud_events`, which owns the before/after/error sequence, the
state carried between phases, and a before handler that stops propagation.

Command-based Rust generation uses `src/generator/files.rs` to render these
partials into `src/<model>/` alongside a short `src/<model>.rs` facade. Model
types and columns, input, decoding, aggregate types, delegate operations, and
query builders have separate files. The facade uses `include!` so these items
still belong to the original Rust module: public paths and private access
between builders and delegates stay compatible. Views omit write files, and
sync clients omit the streaming delegate file. To add an operation, update its
partial, the `delegate.tera` assembly, and the file layout in `files.rs`.

The public `generate_model` / `generate_all_models` and `write_rust_code` APIs
retain their complete-model strings and existing layout. Both output forms use
the same template partials; neither writer parses generated Rust to split it.

Python's `templates/python/model_file.py.tera` assembles `model/`, `input/`, and
`delegate/` partials. Command-based generation uses
`src/python/generator/files.rs` and the `files/` templates to keep
`models/<model>.py` as the public facade and emit private sibling modules for
inputs, event types, wire conversion, reads, writes, and aggregates. The facade
still defines the Pydantic model and public delegate, explicitly re-exports
input/event types and compatibility helpers, and retains its existing
`__all__`. Private operation classes supply inherited methods; the write class
inherits reads because deleting one row first looks it up. Views only inherit
reads and aggregates. Model-independent codec rules — wire serialization,
filter and select preparation, include nodes, row reading — live in the
runtime's `_internal/codec.py`; a model keeps only its own maps and serializers
and binds them through `ModelInputCodec`.

Import order is deliberate: the facade defines the model before loading event
types, codecs, and delegates that reference it. Relation codecs continue to
import other public model modules lazily, and `models/__init__.py` rebuilds
Pydantic forward references once every model has loaded. Keep these contracts
when adding a relation or a new input type. `generate_python_model`,
`generate_all_python_models`, and `write_python_code` keep their source-only
behavior, sharing the same operation bodies with the modular output.

Java's `templates/java/delegate.java.tera` and `dsl.java.tera` are assemblies of
partials. `delegate/` splits the class into its constructor, the async and sync
public surfaces, streaming, and the read, write and aggregate implementations
behind them; `dsl/` splits the nested builders into filters, selection, vector
search, nested writes, create/update input, operation arguments, and aggregates,
with `serializable_tail.java.tera` carrying the accessor every node-backed
builder ends with.

A Java class cannot be split across files, so the generated delegate stays one
class and gets shorter instead: `internal/AbstractDelegate` owns the request
envelope, the raw-statement envelope, the select guards, the stream chunk size,
projection decoding, and the three shapes a write follows around its CRUD events
— one record, many records, a count. Each shape takes the operation name, the
wire method, the event arguments, the request, and a decoder, so a delegate
declares what it writes instead of repeating the event plumbing six times.
`JsonSupport.entries` and `JsonSupport.batchEntries` do the same for the arrays
a nested write appends to. Adding an operation means updating its partial, the
`delegate.java.tera` assembly, and the base class when the new step is one every
model spells the same way.

JavaScript's `model.js.tera` and `model.d.ts.tera` assemble partials under
`templates/js/model/`. Runtime row/input codecs and delegate operations have
separate owners; declarations separate model/input types, events, and query
arguments/delegates. Both outputs consume the same field and relation contexts.
Only JavaScript and declarations are rendered: the unused parallel TypeScript
templates have been removed.

The static runtime in `templates/js/runtime/` has one TypeScript source per
module. Its JavaScript and declaration artifacts are generated during
development with the pinned compiler in [tools/js-runtime](../../tools/js-runtime/README.md)
and checked for drift in CI. Change runtime logic and signatures in the `.ts`
source, regenerate, and review both artifacts. Cargo embeds the results, so
building and using the Rust generator needs no Node toolchain. Output paths,
ES module imports, protocol substitution and the public client APIs remain the
same.

## Testing

```bash
cargo test -p nautilus-orm-codegen
```

`snapshot_tests` compares representative Rust, Python, Java and JS output
(including JS declarations) with the checked-in files in `tests/snapshots/` on
every run. Line endings are normalized to LF, file selection uses logical names,
and fixtures use fixed paths. The suite also checks individual API contracts;
`writer_tests` compiles a generated Rust client and an events macro consumer.

`path_equivalence_tests` generates an async Rust client, compiles its consumer,
and runs it against isolated SQLite databases using the engine's existing
`tests/common` fixture. It compares RPC, embedded dispatch, typed handlers and
generated client calls on the same schema. Run it without external toolchains:

```bash
cargo test --locked -p nautilus-orm-codegen --test path_equivalence_tests
```

The schema and consumer cases live in `tests/fixtures/path_equivalence/`.
The consumer uses command-based generation, exercising publication and the
included model files. The source-only writer remains covered by its compiled
consumer. A small layout snapshot protects the Rust facade and emitted paths;
the existing complete-model snapshots protect the shared partial contents.
The generated consumer inherits the workspace lockfile and builds offline;
its build cache lives in `target/path-equivalence/`.

| Contract | Runtime coverage |
| --- | --- |
| Mapped names, null, enum, decimal, datetime and JSON | RPC, embedded, typed and Rust Auto/Always/Never reads |
| Upsert insert/update, `returnData`, affected counts | Engine adapters and generated Rust client modes |
| Includes with ordering and pagination | Engine adapters, Rust Auto/Always; Never rejects includes |
| Error code, message and details | RPC, embedded and typed mutation adapters |
| Before/after events, priorities and stopped writes | Rust modes plus Python/JS/Java runtime E2E |
| Transaction rollback and reads on the transaction | Rust Auto/Always/Never |
| Streaming early break, follow-up read and cleanup | Python/JS/Java runtime E2E |

`Auto` remains the default. `Never` rejects count and include operations;
event hooks belong to generated clients and are tested at that boundary.
Typed and embedded mutation adapters return rows and require `returnData: true`;
the generated client also checks its `return_data: false` result (`None`).
Upserts can consume sequence values on conflict, so tests check generated keys
by reading the inserted record back, without requiring contiguous identifiers.
The equivalence fixture covers SQLite; provider-specific SQL and transaction
behavior remain covered by the connector and engine integration suites.

Verify baselines without writing files (also used in CI):

```bash
INSTA_UPDATE=no cargo test --locked -p nautilus-orm-codegen --test snapshot_tests
```

For an intentional output change, run the affected test with `INSTA_UPDATE=always`,
review the `.snap` diff, then rerun with `INSTA_UPDATE=no`. On PowerShell, set
`$env:INSTA_UPDATE = 'always'` or `'no'` before running Cargo. Without an explicit
update mode, mismatches fail and leave ignored `.snap.new` candidates for review;
accepted baselines are never updated by CI.

Runtime E2E tests exercise generated Python, JS and Java clients against SQLite.
Python's fixtures use command-based generation to cover the private module
imports, publication, streaming, and event dispatch together.
The import compatibility case compares the source-only and modular clients at
runtime: symbols, `__all__`, model annotations, and delegate signatures match
for relations, views, enums, composites, and extensions. It uses the existing
Pydantic stub and does not need a database; real Pydantic integration is also
exercised by the generated-client examples.
They require `sqlite3`, Python 3 (`python3` or `python`), Node, Java 21 or newer,
and the Jackson jars listed by `java_test_classpath` in
`tests/stream_runtime_e2e_tests.rs`. Put those jars in
`target/test-jars/jackson-<version>/` or set `NAUTILUS_JAVA_TEST_CLASSPATH`.
The TypeScript consumer test also requires `npm ci` in `tools/js-runtime`;
it uses that pinned compiler and Node type definitions rather than a global
installation. It checks the generated declarations in strict mode, including
event contexts, selected fields, streaming, transactions and metrics, and
ensures invalid field access and inputs are rejected.

```bash
NAUTILUS_REQUIRE_E2E=1 cargo test --locked -p nautilus-orm-codegen \
  --test stream_runtime_e2e_tests -- --nocapture
```

CI supplies these prerequisites and sets `NAUTILUS_REQUIRE_E2E=1`, so missing
tools or jars fail the job. When the variable is unset, missing prerequisites
skip the affected tests; `--nocapture` shows each reason. On PowerShell, set
`$env:NAUTILUS_REQUIRE_E2E = '1'` before running Cargo.
