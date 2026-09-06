//! Turning what a backend generated into the files of a client.
//!
//! Each language module lays its output out as a
//! [`GeneratedPackage`](crate::package::GeneratedPackage) — the paths and
//! contents of every file, and nothing else — so a client can be generated and
//! inspected without a directory to put it in. The `write_*_code` entry points
//! here are that layout followed by one publish, which is the only step that
//! touches the disk.

pub(crate) mod java;
pub(crate) mod js;
pub(crate) mod python;
pub(crate) mod rust;

use anyhow::Result;
use std::collections::HashMap;

use crate::GeneratedFile;

pub use js::JsOutput;

/// Write the generated Rust client to `output_path`. See [`rust::package`].
pub fn write_rust_code(
    output_path: &str,
    models: &HashMap<String, String>,
    enums_code: Option<String>,
    composite_types_code: Option<String>,
    extension_files: &[GeneratedFile],
    schema_source: &str,
    standalone: bool,
) -> Result<()> {
    rust::package(
        output_path,
        models,
        enums_code,
        composite_types_code,
        extension_files,
        schema_source,
        standalone,
    )?
    .publish(output_path)
}

/// Write the generated Python package to `output_path`. See [`python::package`].
pub fn write_python_code(
    output_path: &str,
    models: &[GeneratedFile],
    enums_code: Option<String>,
    composite_types_code: Option<String>,
    extension_files: &[GeneratedFile],
    client_code: Option<String>,
    runtime_files: &[GeneratedFile],
) -> Result<()> {
    python::package(
        models,
        enums_code,
        composite_types_code,
        extension_files,
        client_code,
        runtime_files,
    )?
    .publish(output_path)
}

/// Write the generated JavaScript package to `output_path`. See [`js::package`].
pub fn write_js_code(output_path: &str, output: JsOutput<'_>) -> Result<()> {
    js::package(output).publish(output_path)
}

/// Write the generated Java module to `output_path`. See [`java::package`].
pub fn write_java_code(output_path: &str, files: &[GeneratedFile]) -> Result<()> {
    java::package(files).publish(output_path)
}
