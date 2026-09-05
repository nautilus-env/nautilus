//! # Nautilus Migrate
//!
//! Schema migrations for Nautilus ORM.
//!
//! This crate provides tools for:
//! - Converting schema IR to SQL DDL
//! - Tracking applied migrations
//! - Applying and rolling back migrations
//! - Generating migration files

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod applier;
mod apply;
mod change;
mod ddl;
mod error;
mod executor;
mod file_store;
mod migration;
mod normalize;
mod provider;
mod serializer;
mod tracker;
mod utils;

pub mod diff;
pub mod inspector;
pub mod live;

pub use applier::DiffApplier;
pub use apply::{plan_apply_phases, ApplyFailure, ApplyOutcome, ApplyPhase, GroupStatus};
pub use ddl::{DatabaseProvider, DdlGenerator};
pub use diff::{
    change_risk, order_changes_for_apply, Change, ChangeDescription, ChangeRisk, SchemaDiff,
};
pub use error::{MigrationError, Result};
pub use executor::MigrationExecutor;
pub use file_store::MigrationFileStore;
pub use inspector::SchemaInspector;
pub use live::{
    model_table, LiveColumn, LiveCompositeField, LiveCompositeType, LiveExtension, LiveIndex,
    LiveSchema, LiveTable,
};
pub use migration::{Migration, MigrationDirection, MigrationStatus};
pub use serializer::{
    serialize_live_schema, serialize_live_schema_with_options, PullNameCase, PullNamingOptions,
};
pub use tracker::MigrationTracker;
pub use utils::requires_own_transaction;
