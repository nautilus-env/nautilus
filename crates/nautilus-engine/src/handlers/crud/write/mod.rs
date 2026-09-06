//! Write operations: `create`, `createMany`, `update`, `upsert`, `delete` and
//! the `*Many` variants.
//!
//! One module per operation, over two shared foundations. [`input`] owns the
//! rules that read a `data` object — accepted keys, atomic operators, server
//! defaults, `updatedAt` — so every path writes a row the same way; and
//! [`read_back`] owns finding the written rows again where the dialect has no
//! `RETURNING`, which is the only reason those paths need a transaction.
//!
//! Each module holds one operation's statement plus its three entry points —
//! RPC, embedded and typed — which differ only in how they parse parameters
//! and shape the answer. Nested writes stay in [`nested`](super::nested) and
//! reach the database through `execute_create`, `execute_update` and
//! `execute_delete` here.

mod create;
mod create_many;
mod delete;
mod input;
mod read_back;
mod update;
mod upsert;

pub(in crate::handlers) use create::{handle_create, handle_create_embedded, handle_create_typed};
pub(in crate::handlers) use create_many::{
    handle_create_many, handle_create_many_embedded, handle_create_many_typed,
};
pub(in crate::handlers) use delete::{handle_delete, handle_delete_many, handle_delete_many_typed};
pub(in crate::handlers) use update::{
    handle_update, handle_update_embedded, handle_update_many, handle_update_many_typed,
    handle_update_typed,
};
pub(in crate::handlers) use upsert::{handle_upsert, handle_upsert_embedded, handle_upsert_typed};

pub(in crate::handlers::crud) use create::execute_create;
pub(in crate::handlers::crud) use delete::execute_delete;
pub(in crate::handlers::crud) use update::execute_update;
