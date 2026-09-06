//! Embedded Python runtime files and public re-exports.

use crate::GeneratedFile;

/// Generate errors/__init__.py.
///
/// Content is static (no template variables needed).
pub fn generate_errors_init() -> &'static str {
    include_str!("../../../templates/python/errors_init.py.tera")
}

/// Generate _internal/__init__.py.
///
/// Content is static (no template variables needed).
pub fn generate_internal_init() -> &'static str {
    include_str!("../../../templates/python/internal_init.py.tera")
}

/// Generate transaction.py at the package root.
///
/// Content is static: re-exports `IsolationLevel` and `TransactionClient`
/// from the internal `_internal.transaction` module so users can write
/// `from nautilus.transaction import IsolationLevel`.
pub fn generate_transaction_init() -> &'static str {
    include_str!("../../../templates/python/transaction_init.py.tera")
}

/// Generate events.py at the package root.
pub fn generate_events_init() -> &'static str {
    include_str!("../../../templates/python/events.py.tera")
}

/// Returns static runtime Python files to be written alongside generated code.
/// These files implement the base client, engine process manager, protocol, and errors.
pub fn python_runtime_files() -> Vec<GeneratedFile> {
    let protocol_version = nautilus_protocol::PROTOCOL_VERSION.to_string();
    vec![
        (
            "_errors.py".to_string(),
            include_str!("../../../templates/python/runtime/_errors.py").to_string(),
        ),
        (
            "_protocol.py".to_string(),
            include_str!("../../../templates/python/runtime/_protocol.py")
                .replace("{{ protocol_version }}", &protocol_version),
        ),
        (
            "_engine.py".to_string(),
            include_str!("../../../templates/python/runtime/_engine.py").to_string(),
        ),
        (
            "_client.py".to_string(),
            include_str!("../../../templates/python/runtime/_client.py").to_string(),
        ),
        (
            "_descriptors.py".to_string(),
            include_str!("../../../templates/python/runtime/_descriptors.py").to_string(),
        ),
        (
            "_transaction.py".to_string(),
            include_str!("../../../templates/python/runtime/_transaction.py").to_string(),
        ),
        (
            "_events.py".to_string(),
            include_str!("../../../templates/python/runtime/_events.py").to_string(),
        ),
    ]
}
