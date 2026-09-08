//! Convert typed Rust query arguments into the JSON shape consumed by the engine.
//!
//! One module per layer of the payload: `args` writes the request object,
//! `filters` writes the `where` object inside it, and `expressions` writes the
//! leaves both of them reach — a column reference and a value operand.

mod args;
mod expressions;
mod filters;

pub use args::{find_many_args_to_protocol_json, find_many_args_to_protocol_object};
pub use filters::where_expr_to_protocol_json;
