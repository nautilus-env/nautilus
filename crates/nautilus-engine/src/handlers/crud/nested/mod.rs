//! Nested writes: relation-shaped entries inside the `data` of `query.create`
//! and `query.update`.
//!
//! A relation field named in `data` carries an object of operations rather than
//! a column value — `{ "posts": { "create": [...] } }`. [`plan`] splits such a
//! payload into the columns of the written model and the operations that run
//! around its statement, and [`binding`] answers the question that decides when
//! each one can run: which model holds the foreign key.
//!
//! - **Owning side** — the written model holds it, so [`owning`] resolves those
//!   operations before the parent statement and hands it the foreign-key
//!   columns they produced.
//! - **Inverse side** — the related model holds it, so [`inverse`] runs those
//!   operations after the parent statement, scoped to the key it produced.
//! - **Many-to-many** — neither model does, so [`many_to_many`] writes the join
//!   table Nautilus owns instead of a foreign key.
//!
//! Under the three sides sit the two things they share: [`payload`] reads the
//! raw JSON of an operation, and [`execute`] reaches the database through the
//! handler a top-level request would use, on the caller's transaction, so value
//! conversion, defaults, `RETURNING` handling and one more level of nesting are
//! shared with the flat paths instead of reimplemented here.

mod binding;
mod execute;
mod inverse;
mod many_to_many;
mod owning;
mod payload;
mod plan;

pub(in crate::handlers::crud) use inverse::apply_children;
pub(in crate::handlers::crud) use owning::prepare_parent_data;
pub(in crate::handlers::crud) use plan::split;
