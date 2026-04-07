//! JavaScript/TypeScript code generator module.

pub mod backend;
pub mod generator;
pub mod type_mapper;

pub use backend::JsBackend;
pub use generator::{
    generate_all_js_models, generate_js_client, generate_js_composite_types, generate_js_enums,
    generate_js_models_index, js_runtime_files,
};
