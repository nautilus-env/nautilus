//! Repeating `phase` or `priority`, or naming a second model, used to overwrite
//! silently.

struct Ctx;

#[nautilus_events_macros::events(client_crate = crate::client)]
mod repeated_phase {
    use super::Ctx;

    #[nautilus_events_macros::on_create(User, phase = A::Before, phase = A::After)]
    fn handler(_ctx: &mut Ctx) {}
}

#[nautilus_events_macros::events(client_crate = crate::client)]
mod repeated_priority {
    use super::Ctx;

    #[nautilus_events_macros::on_create(User, priority = 1, priority = 2)]
    fn handler(_ctx: &mut Ctx) {}
}

#[nautilus_events_macros::events(client_crate = crate::client)]
mod second_model {
    use super::Ctx;

    #[nautilus_events_macros::on_create(User, Post)]
    fn handler(_ctx: &mut Ctx) {}
}

#[nautilus_events_macros::events(client_crate = crate::client)]
mod no_model {
    use super::Ctx;

    #[nautilus_events_macros::on_create(priority = 1)]
    fn handler(_ctx: &mut Ctx) {}
}

fn main() {}
