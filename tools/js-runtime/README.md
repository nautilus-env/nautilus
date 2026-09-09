# JavaScript runtime maintenance

The six `.ts` files in
[`templates/js/runtime`](../../crates/nautilus-codegen/templates/js/runtime)
are the source for the static client runtime. Edit those files, then regenerate
the adjacent `.js` and `.d.ts` files with the pinned TypeScript compiler:

```sh
cd tools/js-runtime
npm ci --ignore-scripts --no-audit --no-fund
npm run build
npm run check
```

On Windows PowerShell, use `npm.cmd` if script execution is disabled. Node 20
or newer is needed only for this maintenance workflow and the JavaScript tests.
Cargo embeds the checked-in artifacts; building or running the Rust generator
does not invoke Node, npm or TypeScript.

`build` type-checks the sources in strict mode, compiles in memory, then writes
the artifacts. `check` repeats compilation and compares every artifact without
writing; missing, changed or orphaned artifacts fail. The lockfile fixes the
compiler and Node type definitions, and output uses LF on every platform.
CI runs `check` in the codegen E2E job before executing the client tests.

The compiler's [declaration output](https://www.typescriptlang.org/tsconfig/declaration.html)
keeps method signatures beside their implementation. Private members carry
`@internal` so `stripInternal` preserves the existing public declaration surface.
Fields use `declare` and explicit constructor assignments to retain runtime
initialization order. The protocol constant is a typed zero placeholder:
the build restores `{{ protocol_version }}` in both artifacts, and
`js_runtime_files` substitutes the Rust protocol version when generating a client.

To change a static method, update its TypeScript implementation and types,
regenerate, and review both artifacts. To add a runtime module, also register
its artifacts in
[`src/js/generator/runtime.rs`](../../crates/nautilus-codegen/src/js/generator/runtime.rs).
Schema-dependent types and operations use the contexts and template partials
described in the [codegen README](../../crates/nautilus-codegen/README.md#template-layout).
They are rendered directly by Rust and need no TypeScript compilation.

After a runtime change, run from the repository root:

```sh
cargo test --locked -p nautilus-orm-codegen --test snapshot_tests
cargo test --locked -p nautilus-orm-codegen --test stream_runtime_e2e_tests -- --nocapture
```

The E2E suite compiles a TypeScript consumer against the generated declarations,
including events, projections, streaming, transactions, metrics and rejected
inputs. Its JavaScript runtime cases exercise events and streaming cleanup
against SQLite. See the codegen README for the other E2E prerequisites; CI sets
`NAUTILUS_REQUIRE_E2E=1` so missing prerequisites fail instead of skipping tests.
