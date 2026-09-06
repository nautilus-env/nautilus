//! Java runtime and event source files.

use crate::GeneratedFile;
use anyhow::Result;
use tera::Context;

use super::java_source_path;
use super::templates::{render, render_pkg};

/// Returns the rendered Java runtime files (the `internal` package) for the
/// given root package. Each tuple is `(maven-relative path, file content)`.
///
/// This is the Java equivalent of `python_runtime_files()` / `js_runtime_files()`:
/// the source content lives in `templates/java/runtime/*.java.tera` and is
/// embedded at compile time; only `package_name` (and `version`) are substituted
/// at generation time.
pub fn java_runtime_files(package_name: &str) -> Result<Vec<GeneratedFile>> {
    let mut ctx_pkg = Context::new();
    crate::template::insert_protocol_version(&mut ctx_pkg);
    ctx_pkg.insert("package_name", package_name);

    let mut ctx_ver = Context::new();
    crate::template::insert_protocol_version(&mut ctx_ver);
    ctx_ver.insert("package_name", package_name);
    ctx_ver.insert("version", env!("CARGO_PKG_VERSION"));

    let pkg = package_name;
    Ok(vec![
        (
            java_source_path(pkg, "internal", "WireSerializable.java"),
            render("java_rt_wire_serializable.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "RpcCaller.java"),
            render("java_rt_rpc_caller.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "GlobalNautilusRegistry.java"),
            render("java_rt_global_registry.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "NautilusException.java"),
            render("java_rt_nautilus_exception.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "ProtocolException.java"),
            render("java_rt_protocol_exception.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "HandshakeException.java"),
            render("java_rt_handshake_exception.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "TransactionException.java"),
            render("java_rt_transaction_exception.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "NotFoundException.java"),
            render("java_rt_not_found_exception.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "JsonSupport.java"),
            render("java_rt_json_support.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "EngineProcess.java"),
            render("java_rt_engine_process.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "BaseNautilusClient.java"),
            render("java_rt_base_client.tera", &ctx_ver)?,
        ),
        (
            java_source_path(pkg, "internal", "BaseTransactionClient.java"),
            render("java_rt_base_tx_client.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "AbstractDelegate.java"),
            render("java_rt_abstract_delegate.tera", &ctx_pkg)?,
        ),
        (
            java_source_path(pkg, "internal", "EventRegistry.java"),
            render("java_rt_event_registry.tera", &ctx_pkg)?,
        ),
    ])
}

pub(super) fn java_event_files(package_name: &str) -> Result<Vec<GeneratedFile>> {
    let pkg = package_name;
    Ok(vec![
        (
            java_source_path(pkg, "events", "EventPhase.java"),
            render_pkg("java_event_phase.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "CrudEventContext.java"),
            render_pkg("java_event_context.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "StopPropagation.java"),
            render_pkg("java_stop_propagation.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "OnCreate.java"),
            render_pkg("java_on_create.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "OnCreateMany.java"),
            render_pkg("java_on_create_many.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "OnUpdate.java"),
            render_pkg("java_on_update.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "OnUpdateMany.java"),
            render_pkg("java_on_update_many.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "OnDelete.java"),
            render_pkg("java_on_delete.tera", pkg)?,
        ),
        (
            java_source_path(pkg, "events", "OnDeleteMany.java"),
            render_pkg("java_on_delete_many.tera", pkg)?,
        ),
    ])
}
