//! An `on_*` attribute outside an `#[events]` module registers nothing, and a
//! `cfg_attr` hides it from `#[events]` entirely.

struct Ctx;

#[nautilus_events_macros::on_create(User)]
fn loose(_ctx: &mut Ctx) {}

#[nautilus_events_macros::events(client_crate = crate::client)]
mod hooks {
    use super::Ctx;

    #[cfg_attr(all(), nautilus_events_macros::on_update(User))]
    fn hidden(_ctx: &mut Ctx) {}
}

fn main() {}
