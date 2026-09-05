//! A handler behind a `#[cfg]` takes its registration with it, so the generated
//! `register` never calls a function that was not compiled.

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

    pub struct Never;

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

pub struct User;
pub struct Ctx;

#[nautilus_events_macros::events(client_crate = crate::fake_client)]
mod hooks {
    use super::{Ctx, User};

    #[cfg(any())]
    #[nautilus_events_macros::on_create(User)]
    fn never_compiled(_ctx: &mut Ctx) {}

    #[cfg(all())]
    #[nautilus_events_macros::on_update(User)]
    fn always_compiled(_ctx: &mut Ctx) {}
}

fn main() {
    struct Direct;
    impl fake_client::Executor for Direct {}

    let client: fake_client::Client<Direct> = fake_client::Client(std::marker::PhantomData);
    hooks::register(&client);
}
