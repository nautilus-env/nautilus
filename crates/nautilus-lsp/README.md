# nautilus-lsp

`nautilus-lsp` is the Language Server Protocol server for `.nautilus` schema files.

It is intentionally thin: almost all schema intelligence lives in `nautilus-schema`, while this crate translates between LSP messages and Nautilus analysis results.

## Capabilities

| Capability | Current behavior |
| --- | --- |
| Diagnostics | Published after open/change/save, to the file each one belongs to |
| Completion | Schema-aware suggestions, imported declarations, and filesystem paths inside `import` |
| Hover | Uses resolved schema metadata |
| Go to definition | Jumps to model, enum, type, and field declarations, across imported files |
| Document formatting | Whole-file canonical formatting |
| Semantic tokens | Models, enums, and composite types |
| Text sync | Full-document sync |
| Watched files | `**/*.nautilus`, so editing an imported file outside the editor refreshes analysis |

## Multi-file schemas

A document that declares `import "…"` is analysed together with the files it
imports, transitively. The assembled schema is what resolves names, so a model
referring to an enum in another file is not an error, and each diagnostic is
published to the file that holds it rather than to the open document.

Inside an import string, completion lists directories as import targets or for
navigation, plus `.nautilus` files. The import itself must resolve to a
`.nautilus` file or a directory containing schema files; a missing path,
different file extension, or directory without schema files is reported
directly on the declaration.

Open buffers win over disk: an imported file being edited in another tab is
assembled from the editor's text, and editing any file re-analyses the other
open documents that share its schema. Opening an imported file reads it inside
the schema that imports it, so a model it references from elsewhere still
resolves.

A file nothing imports is analysed on its own: the server never joins sibling
files it was not told about, which is what keeps a directory of alternative
schemas — one per provider, say — from being merged into duplicate
declarations.

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
| `import_completion` | Filesystem-backed completion for import paths |
| `workspace` | The open file assembled with the files it imports |
| `convert` | Offset/span conversions between Nautilus and LSP |
| `main` | stdio server bootstrap |

## Testing

```bash
cargo test -p nautilus-orm-lsp
```
