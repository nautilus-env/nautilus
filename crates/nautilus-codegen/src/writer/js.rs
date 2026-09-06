//! How a generated JavaScript package is laid out.

use crate::package::GeneratedPackage;
use crate::GeneratedFile;

/// Everything the JavaScript backend produced, as [`package`] lays it out.
///
/// Each half of the client has its own field because the runtime and the
/// declarations are generated separately and can be absent independently — a
/// schema with no enums has neither `enums.js` nor `enums.d.ts`, while
/// composite types are types only and so have no runtime half at all.
#[derive(Default)]
pub struct JsOutput<'a> {
    pub js_models: &'a [GeneratedFile],
    pub dts_models: &'a [GeneratedFile],
    pub js_enums: Option<String>,
    pub dts_enums: Option<String>,
    pub dts_composite_types: Option<String>,
    pub js_extension_files: &'a [GeneratedFile],
    pub dts_extension_files: &'a [GeneratedFile],
    pub js_client: Option<String>,
    pub dts_client: Option<String>,
    pub js_models_index: Option<String>,
    pub dts_models_index: Option<String>,
    pub runtime_files: &'a [GeneratedFile],
}

/// Lay out the generated JavaScript client and its TypeScript declarations.
///
/// Produces:
/// - `index.js`, `index.d.ts`           — the generated `Nautilus` class
/// - `models/index.js`, `models/index.d.ts` — barrel re-exports for all models
/// - `models/{snake}.js`, `models/{snake}.d.ts` — per-model delegates and types
/// - `enums.js`, `enums.d.ts`           — enums (if any)
/// - `types.d.ts`                       — composite types (declarations only)
/// - `_internal/_*.js`, `_internal/_*.d.ts` — the runtime the models call into
/// - `package.json`                     — what makes the directory importable
pub(crate) fn package(output: JsOutput<'_>) -> GeneratedPackage {
    let JsOutput {
        js_models,
        dts_models,
        js_enums,
        dts_enums,
        dts_composite_types,
        js_extension_files,
        dts_extension_files,
        js_client,
        dts_client,
        js_models_index,
        dts_models_index,
        runtime_files,
    } = output;

    let mut package = GeneratedPackage::default();

    package.add_all("", js_extension_files);
    package.add_all("", dts_extension_files);
    package.add_all("models", js_models);
    package.add_all("models", dts_models);

    if let Some(index_js) = js_models_index {
        package.add("models/index.js", index_js);
    }
    if let Some(index_dts) = dts_models_index {
        package.add("models/index.d.ts", index_dts);
    }

    if let Some(enums_js) = js_enums {
        package.add("enums.js", enums_js);
    }
    if let Some(enums_dts) = dts_enums {
        package.add("enums.d.ts", enums_dts);
    }
    if let Some(types_dts) = dts_composite_types {
        package.add("types.d.ts", types_dts);
    }

    package.add_all("_internal", runtime_files);

    if let Some(client_js) = js_client {
        package.add("index.js", client_js);
    }
    if let Some(client_dts) = dts_client {
        package.add("index.d.ts", client_dts);
    }

    package.add("package.json", PACKAGE_JSON);
    package.sorted()
}

/// The `package.json` that makes the generated directory importable.
///
/// The client is pure ESM. Without a manifest of its own Node reads the
/// consuming project's `type`, so `import { Nautilus } from './db/index.js'`
/// fails in any project that has not opted into modules — a requirement the
/// consumer should not have to know about.
const PACKAGE_JSON: &str = r#"{
  "name": "nautilus",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "index.js",
  "types": "index.d.ts",
  "exports": {
    ".": {
      "types": "./index.d.ts",
      "default": "./index.js"
    },
    "./models": {
      "types": "./models/index.d.ts",
      "default": "./models/index.js"
    }
  }
}
"#;
