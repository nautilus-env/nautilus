//! Models named after types the expansion itself writes.
//!
//! A schema is free to have a model called `Box`, `Option` or `Vec`, and the
//! `#[events]` module imports it next to the registrations. Every path the
//! expansion writes is absolute so the model never captures one: `Box::pin`
//! would otherwise resolve to the model here. The same rule covers the `u64`
//! an `UpdateMany` handler stops with, which this stub cannot show apart from
//! a model of that name because `IntoEventResult` is generic over the type.

mod fake_client {
    use std::any::Any;
    use std::future::Future;
    use std::marker::PhantomData;
    use std::pin::Pin;

    pub type Result<T> = std::result::Result<T, String>;
    pub type EventFuture<'a, T> = Pin<Box<dyn Future<Output = Result<EventControl<T>>> + Send + 'a>>;

    #[derive(Clone, Copy)]
    pub enum EventPhase {
        Before,
        After,
        Error,
    }

    pub enum EventControl<T> {
        Continue,
        StopPropagation(T),
    }

    pub trait IntoEventResult<T> {
        fn into_event_result(self) -> Result<EventControl<T>>;
    }

    impl<T> IntoEventResult<T> for () {
        fn into_event_result(self) -> Result<EventControl<T>> {
            Ok(EventControl::Continue)
        }
    }

    pub trait Executor {}

    pub struct Events;

    macro_rules! hook {
        ($name:ident) => {
            pub fn $name<C, T, F>(
                &self,
                _model: &'static str,
                _phase: EventPhase,
                _priority: u8,
                _handler: F,
            ) where
                C: Any + Send + 'static,
                T: Any + Send + 'static,
                F: for<'a> Fn(&'a mut C) -> EventFuture<'a, T> + Send + Sync + 'static,
            {
            }
        };
    }

    impl Events {
        hook!(on_create_with_priority);
        hook!(on_create_many_with_priority);
        hook!(on_update_with_priority);
        hook!(on_update_many_with_priority);
        hook!(on_delete_with_priority);
        hook!(on_delete_many_with_priority);
    }

    pub struct Client<E>(pub PhantomData<E>);

    impl<E> Client<E> {
        pub fn events(&self) -> Events {
            Events
        }
    }
}

pub struct Ctx;

pub struct Box;
pub struct Option;
pub struct Vec;

#[nautilus_events_macros::events(client_crate = crate::fake_client)]
mod hooks {
    use super::{Box, Ctx, Option, Vec};

    #[nautilus_events_macros::on_create(Box)]
    fn audit_create(_ctx: &mut Ctx) {}

    #[nautilus_events_macros::on_delete(Option)]
    fn audit_delete(_ctx: &mut Ctx) {}

    #[nautilus_events_macros::on_update(Vec)]
    async fn audit_update(_ctx: &mut Ctx) {}

    #[nautilus_events_macros::on_update_many(Vec)]
    fn audit_update_many(_ctx: &mut Ctx) {}
}

fn main() {
    struct Direct;
    impl fake_client::Executor for Direct {}

    let client: fake_client::Client<Direct> = fake_client::Client(std::marker::PhantomData);
    hooks::register(&client);
}
