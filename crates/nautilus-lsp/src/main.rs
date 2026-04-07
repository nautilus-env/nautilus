//! `nautilus-lsp` — LSP server for `.nautilus` schema files.
//!
//! Communicates over stdin/stdout using the Language Server Protocol.
//! Run the binary and configure your editor to launch it as a language server
//! for files matching `*.nautilus`.
//!
//! # Quick start (Neovim — nvim-lspconfig)
//!
//! ```lua
//! require('lspconfig.configs').nautilus_lsp = {
//!   default_config = {
//!     cmd = { 'nautilus-lsp' },
//!     filetypes = { 'nautilus' },
//!     root_dir = require('lspconfig.util').root_pattern('schema.nautilus', '.git'),
//!   },
//! }
//! require('lspconfig').nautilus_lsp.setup {}
//! ```
//!
//! # Quick start (Helix — languages.toml)
//!
//! ```toml
//! [[language]]
//! name = "nautilus"
//! language-servers = ["nautilus-lsp"]
//!
//! [language-server.nautilus-lsp]
//! command = "nautilus-lsp"
//! ```

#![forbid(unsafe_code)]

mod backend;
mod convert;
mod document;

use backend::Backend;
use dashmap::DashMap;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: DashMap::new(),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
