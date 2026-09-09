//! Snapshot tests for the code generator: parse a schema, generate code, and
//! compare representative output against versioned baselines on every run.
//!
//! Baselines live in `tests/snapshots/`. Use `INSTA_UPDATE=no` to verify without
//! writing files, or `INSTA_UPDATE=always` to update after reviewing an intended
//! change. CRLF is normalized to LF; files are selected by logical name and
//! fixture paths are fixed, so machine paths and map iteration order stay out.

mod snapshot;
