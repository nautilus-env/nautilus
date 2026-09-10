# nautilus-events-macros

Attribute macros for CRUD event handlers in generated Rust clients. `#[events]`
collects handlers from an inline module and emits a `register` function.
`#[on_create]`, `#[on_create_many]`, `#[on_update]`, `#[on_update_many]`,
`#[on_delete]` and `#[on_delete_many]` identify hooks inside that module.

The crate expands code against the generated client's event API and has no
workspace dependencies. Runtime dispatch, before/after/error phases, priority
and stop propagation live in codegen's
[Rust event runtime](../nautilus-codegen/templates/rust/events.rs.tpl).
The [compiled consumers](tests/ui/) show accepted syntax and diagnostics.

## Implementation owners

| Module under `src/` | Responsibility |
| --- | --- |
| `lib.rs` | Procedural macro entry points and `proc_macro` boundary |
| `args.rs` | Typed parsing of module and hook arguments with source spans |
| `validate.rs` | Handler signature checks, registration collisions and rejected attributes |
| `operation.rs` | The six operations, attribute/registry names and stop-propagation result types |
| `expand.rs` | Registration code, handler `cfg` propagation and errors for hooks outside `#[events]` |

Change argument syntax in `args.rs`, rejection rules in `validate.rs`, and
emitted code in `expand.rs`. An operation must agree with the generated
context aliases and runtime registry. The
[contributor guide](../../CONTRIBUTING.md#add-a-query-operation) covers the
query and client changes around it.

## Testing

```bash
cargo test --locked -p nautilus-events-macros
cargo test --locked -p nautilus-orm-codegen --test writer_tests
```

[tests/ui](tests/ui/) contains accepted consumers and rejected forms with
diagnostic snapshots, run by trybuild. Their small client stubs isolate the
expansion contract; codegen's writer tests also compile an event consumer
against a real generated client. Runtime behavior is covered by the generated
client suites linked from [TESTING.md](../../TESTING.md).
