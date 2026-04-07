# nautilus-lsp

`nautilus-lsp` is the Language Server Protocol server for `.nautilus` schema files.

It is intentionally thin: almost all schema intelligence lives in `nautilus-schema`, while this crate translates between LSP messages and Nautilus analysis results.

## Capabilities

| Capability | Current behavior |
| --- | --- |
| Diagnostics | Published after open/change/save |
| Completion | Triggered from schema-aware analysis |
| Hover | Uses resolved schema metadata |
| Go to definition | Jumps to model, enum, type, and field declarations |
| Document formatting | Whole-file canonical formatting |
| Semantic tokens | Models, enums, and composite types |
| Text sync | Full-document sync |

## Running from source

```bash
cargo run -p nautilus-orm-lsp
```

The server speaks stdio and is designed to be launched by an editor integration rather than directly by end users.

## Workspace usage

- The VS Code extension in `tools/vscode-nautilus-schema/` launches this server.
- The server depends on `nautilus-schema` for diagnostics, completion, hover, definitions, formatting, and semantic tokens.

## Internal layout

| Module | Purpose |
| --- | --- |
| `backend` | `tower-lsp` server implementation |
| `document` | Cached source + analysis per open document |
| `convert` | Offset/span conversions between Nautilus and LSP |
| `main` | stdio server bootstrap |

## Testing

```bash
cargo test -p nautilus-orm-lsp
```
