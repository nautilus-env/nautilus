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
| Writers | `src/writer.rs` |
| Rust generator context | `src/generator.rs` |
| Python generator context | `src/python/` |
| JS generator context | `src/js/` |
| Java generator context | `src/java/` |

## Testing

```bash
cargo test -p nautilus-orm-codegen
```

`snapshot_tests` compares representative Rust, Python, Java and JS output
(including JS declarations) with the checked-in files in `tests/snapshots/` on
every run. Line endings are normalized to LF, file selection uses logical names,
and fixtures use fixed paths. The suite also checks individual API contracts;
`writer_tests` compiles a generated Rust client and an events macro consumer.

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
They require `sqlite3`, Python 3 (`python3` or `python`), Node, Java 21 or newer,
and the Jackson jars listed by `java_test_classpath` in
`tests/stream_runtime_e2e_tests.rs`. Put those jars in
`target/test-jars/jackson-<version>/` or set `NAUTILUS_JAVA_TEST_CLASSPATH`.

```bash
NAUTILUS_REQUIRE_E2E=1 cargo test --locked -p nautilus-orm-codegen \
  --test stream_runtime_e2e_tests -- --nocapture
```

CI supplies these prerequisites and sets `NAUTILUS_REQUIRE_E2E=1`, so missing
tools or jars fail the job. When the variable is unset, missing prerequisites
skip the affected tests; `--nocapture` shows each reason. On PowerShell, set
`$env:NAUTILUS_REQUIRE_E2E = '1'` before running Cargo.
