//! What the language offers, described once for every reader.
//!
//! Completion and hover answer about the same scalar types, attributes and
//! configuration keys. Each of them is described here once — the label a
//! completion offers, the snippet it inserts, its one-line detail and the
//! documentation hover shows — so adding, renaming or documenting one touches
//! a single entry.
//!
//! These are descriptions of syntax and documentation. They carry no relation
//! rules and no SQL generation; where a capability is already decided
//! elsewhere, such as which provider supports a type, the catalog reads that
//! answer instead of restating it.

mod attributes;
mod config;
mod types;

pub(super) use attributes::{
    field_attribute, model_attribute, type_attribute_map, AttributeDoc, FIELD_ATTRIBUTES,
    MODEL_ATTRIBUTES,
};
pub(super) use config::{config_field, ConfigBlock, CONFIG_FIELDS};
pub(super) use types::{scalar_doc, SCALAR_TYPES};
