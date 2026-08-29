# Development instructions

## Comments

**Comments are docstrings.** `///` on public items, `//!` on modules — they say
what an item is and how it is used, never how it is implemented.

Do not write inline comments. In particular, never write a comment that
paraphrases the line below it, announces the block that follows, or marks a
section:

```rust
// increment the counter
count += 1;

// build the context
let mut ctx = Context::new();

// --- Section: enum handling ---
```

If a block is long enough to want a heading, the correct move is to **extract a
function with a name that says what it does**.

The one exception, already the standard in this tree, is a comment that explains
a *why* the code cannot express by itself: an external constraint, a deliberate
trade-off, a trap a future contributor would otherwise walk into. It has to
carry information the reader could not recover from the code:

```rust
// NOTE: do not add `panic = "abort"` — the engine transport converts handler
// panics into JSON-RPC errors via `catch_unwind` (see engine/src/transport.rs);
// aborting would kill the process instead of answering the client.
```

Anything short of that bar is not a comment, it is noise. New `#[allow(...)]`
attributes follow the same rule: none without a line saying why it is needed.

## Testing a feature

Unit and integration tests in `crates/**` are necessary but not sufficient.
**Every feature has to be exercised in `examples/` by a real scenario against a
real database** before it counts as done. SQLite alone is not enough for
anything that touches SQL generation: PostgreSQL and MySQL disagree on
`RETURNING`, aggregate types, `EXPLAIN` syntax, autoincrement and enums, and
those differences only surface against a live server.

Start the servers:

```bash
docker compose up -d
```

Then run the scenario:

```bash
NAUTILUS_SURFACE_PG_URL='postgresql://nautilus:nautilus@localhost:5432/nautilus_test' \
NAUTILUS_SURFACE_MYSQL_URL='mysql://nautilus:nautilus@localhost:3306/nautilus_test' \
  bash examples/<scenario>/run-all.sh
```

Pick the scenario shape that matches what changed:

- **Engine method or SQL rendering** — an engine-level scenario that drives
  `nautilus engine serve` over JSON-RPC, run against SQLite, PostgreSQL and
  MySQL. See `examples/engine-surface` and `examples/upsert-dialects`.
- **Generated client API** — a scenario with a smoke app per language under
  `rust/app`, `java/src`, `js/`, `py/`, so all four clients are shown to agree.
  See `examples/client-surface` and `examples/basic-crud`. The generated clients
  bake their datasource provider in, so each provider pass has to regenerate
  them from `schema.<language>.<provider>.nautilus`.

A scenario asserts real values — counts, decoded rows, error codes — and exits
non-zero on the first mismatch. "It ran without crashing" is not a test.

When a provider genuinely cannot support the feature, say so in the scenario's
`README.md` and state where that provider *is* covered, instead of quietly
dropping it from the matrix.

`examples/` is gitignored on purpose: regenerated clients, SQLite files and
local notes stay out of the repository.

## Before finishing

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked   # must be warning-free
cargo test --workspace --exclude nautilus-orm-connector --exclude nautilus-orm-codegen --locked
cargo test -p nautilus-orm-codegen --test snapshot_tests --test type_helpers_tests \
    --test writer_tests --test parse_schema_tests --test readme_contract_tests --locked
```

Any change to public behaviour needs a `CHANGELOG.md` entry under `Unreleased`,
in the existing `Added` / `Changed` / `Fixed` style.
