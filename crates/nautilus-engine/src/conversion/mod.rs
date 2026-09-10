//! Conversion between the wire JSON and the internal [`nautilus_core::Value`].
//!
//! Each direction has its own module: `input` builds query values from the
//! request, `normalize` applies the schema hints to a decoded row,
//! `serialize` writes rows back out as JSON, and `composite` owns the
//! PostgreSQL record literal on both sides. This file is only the facade.

mod composite;
mod extension;
mod input;
mod normalize;
mod serialize;

pub use composite::json_to_value_composite;
pub use input::{ensure_scalar_input, holds_structured_json, json_to_value, json_to_value_field};
pub use normalize::{normalize_row_with_hints, normalize_rows_with_hints, ValueHint};
pub use serialize::rows_to_raw_json;

pub use crate::metadata::to_snake_case;
pub use nautilus_protocol::check_protocol_version;
