//! `#[events]` has to see the handlers, so the module cannot be a file module.

#[nautilus_events_macros::events(client_crate = crate::client)]
mod hooks;

fn main() {}
