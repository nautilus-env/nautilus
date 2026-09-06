//! How a generated Python package is laid out.

use anyhow::Result;

use crate::package::GeneratedPackage;
use crate::python::generator::{
    generate_enums_init, generate_errors_init, generate_events_init, generate_internal_init,
    generate_models_init, generate_package_init, generate_transaction_init,
};
use crate::GeneratedFile;

/// Lay out the generated Python package.
///
/// Produces:
/// - `__init__.py`             — package init with exports
/// - `client.py`               — Nautilus client with model delegates
/// - `models/__init__.py`, `models/{model_snake}.py`
/// - `enums/__init__.py`, `enums/enums.py` (if any)
/// - `errors/__init__.py`, `errors/errors.py`
/// - `types/__init__.py`, `types/types.py` (if any)
/// - `extensions/` — one package per extension scalar (if any)
/// - `_internal/` — the runtime the models call into
/// - `py.typed`  — marker for mypy
pub(crate) fn package(
    models: &[GeneratedFile],
    enums_code: Option<String>,
    composite_types_code: Option<String>,
    extension_files: &[GeneratedFile],
    client_code: Option<String>,
    runtime_files: &[GeneratedFile],
) -> Result<GeneratedPackage> {
    let mut package = GeneratedPackage::default();

    if !extension_files.is_empty() {
        package.add(
            "extensions/__init__.py",
            "# Generated extension scalar packages.\n",
        );
        for (relative_path, code) in extension_files {
            let path = format!("extensions/{relative_path}");
            let directory = path.rsplit_once('/').map(|(dir, _)| dir).ok_or_else(|| {
                anyhow::anyhow!("Invalid Python extension file path: {relative_path}")
            })?;
            package.add(
                format!("{directory}/__init__.py"),
                "from .types import *  # noqa: F401, F403\n",
            );
            package.add(path, code);
        }
    }

    package.add_all("models", models);
    package.add("models/__init__.py", generate_models_init(models)?);

    if let Some(types_code) = composite_types_code {
        package.add("types/types.py", types_code);
        package.add(
            "types/__init__.py",
            "from .types import *  # noqa: F401, F403\n",
        );
    }

    let has_enums = enums_code.is_some();
    if let Some(enums_code) = enums_code {
        package.add("enums/enums.py", enums_code);
    }
    package.add("enums/__init__.py", generate_enums_init(has_enums)?);

    for (file_name, contents) in runtime_files {
        let path = match file_name.as_str() {
            "_errors.py" => "errors/errors.py".to_string(),
            _ => format!("_internal/{}", file_name.trim_start_matches('_')),
        };
        package.add(path, contents);
    }

    package.add("errors/__init__.py", generate_errors_init());
    package.add("_internal/__init__.py", generate_internal_init());

    if let Some(client_code) = client_code {
        package.add("client.py", client_code);
    }
    package.add("transaction.py", generate_transaction_init());
    package.add("events.py", generate_events_init());
    package.add("__init__.py", generate_package_init(has_enums)?);
    package.add("py.typed", "");

    Ok(package.sorted())
}
