//! The generated closure passes exactly one context argument, so anything else
//! has to be rejected where the handler is written.

struct Ctx;

#[nautilus_events_macros::events(client_crate = crate::client)]
mod no_argument {
    #[nautilus_events_macros::on_create(User)]
    fn handler() {}
}

#[nautilus_events_macros::events(client_crate = crate::client)]
mod extra_argument {
    use super::Ctx;

    #[nautilus_events_macros::on_create(User)]
    fn handler(_ctx: &mut Ctx, _extra: u8) {}
}

#[nautilus_events_macros::events(client_crate = crate::client)]
mod self_receiver {
    #[nautilus_events_macros::on_create(User)]
    fn handler(self) {}
}

fn main() {}
