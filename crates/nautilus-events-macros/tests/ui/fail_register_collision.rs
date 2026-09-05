//! `#[events]` appends a `register` of its own.

#[nautilus_events_macros::events(client_crate = crate::client)]
mod hooks {
    pub fn register() {}
}

fn main() {}
