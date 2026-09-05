//! Compilation and diagnostic tests for the event attribute macros.
//!
//! The passing cases pin the expansion the generated client has to satisfy; the
//! failing ones pin the message and the span of every unsupported use, so a hook
//! is never dropped without saying so.
//!
//! Each passing case declares its own `fake_client` module mirroring the surface
//! the generated Rust client exposes — `Client`, `Executor`, `EventPhase`,
//! `IntoEventResult`, `EventFuture` and the `on_*_with_priority` registrations.
//! Keeping it inline is what lets every case compile on its own, and it puts the
//! contract next to the expansion that has to satisfy it.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_*.rs");
    t.compile_fail("tests/ui/fail_*.rs");
}
