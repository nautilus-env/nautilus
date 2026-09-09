//! Attribute macros that turn annotated functions into event-handler
//! registrations for a generated Nautilus client.
//!
//! The entry points here only parse their input and hand it on: `args` reads
//! the attributes into typed data, `validate` refuses the uses that would drop
//! a handler in silence, `expand` writes the registrations, and `operation`
//! holds what differs between the six events.

mod args;
mod expand;
mod operation;
mod validate;

use proc_macro::TokenStream;
use syn::{parse_macro_input, ItemMod};

use crate::args::EventsArgs;
use crate::operation::Operation;

/// Collect the `on_*` handlers of an inline module and append a `register`
/// function that wires each of them into a client's event registry.
#[proc_macro_attribute]
pub fn events(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as EventsArgs);
    let module = parse_macro_input!(input as ItemMod);

    expand::events_module(&args, module)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Register a `Create` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_create(_args: TokenStream, input: TokenStream) -> TokenStream {
    expand::hook_outside_events(Operation::Create, input.into()).into()
}

/// Register a `CreateMany` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_create_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    expand::hook_outside_events(Operation::CreateMany, input.into()).into()
}

/// Register an `Update` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_update(_args: TokenStream, input: TokenStream) -> TokenStream {
    expand::hook_outside_events(Operation::Update, input.into()).into()
}

/// Register an `UpdateMany` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_update_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    expand::hook_outside_events(Operation::UpdateMany, input.into()).into()
}

/// Register a `Delete` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_delete(_args: TokenStream, input: TokenStream) -> TokenStream {
    expand::hook_outside_events(Operation::Delete, input.into()).into()
}

/// Register a `DeleteMany` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_delete_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    expand::hook_outside_events(Operation::DeleteMany, input.into()).into()
}
