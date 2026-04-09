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
mod ddl;
mod error;
mod executor;
mod file_store;
mod migration;
mod provider;
mod serializer;
mod tracker;
mod utils;

pub mod diff;
pub mod inspector;
pub mod live;

pub use applier::DiffApplier;
pub use ddl::{DatabaseProvider, DdlGenerator};
pub use diff::{change_risk, order_changes_for_apply, Change, ChangeRisk, SchemaDiff};
pub use error::{MigrationError, Result};
pub use executor::MigrationExecutor;
pub use file_store::MigrationFileStore;
pub use inspector::SchemaInspector;
pub use live::{
    LiveColumn, LiveCompositeField, LiveCompositeType, LiveIndex, LiveSchema, LiveTable,
};
pub use migration::{Migration, MigrationDirection, MigrationStatus};
pub use serializer::serialize_live_schema;
pub use tracker::MigrationTracker;
