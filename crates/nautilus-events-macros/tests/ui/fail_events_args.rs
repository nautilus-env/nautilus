//! `#[events]` needs to be told where the generated client lives.

#[nautilus_events_macros::events]
mod without_args {}

#[nautilus_events_macros::events(crate_path = crate::client)]
mod wrong_key {}

fn main() {}
