//! Embedded JavaScript runtime and TypeScript declarations.

use crate::GeneratedFile;

/// Static JavaScript + declaration runtime files embedded at compile time.
/// Returns `Vec<(filename, content)>` containing both `.js` and `.d.ts` pairs.
pub fn js_runtime_files() -> Vec<GeneratedFile> {
    let protocol_version = nautilus_protocol::PROTOCOL_VERSION.to_string();
    vec![
        (
            "_errors.js".to_string(),
            include_str!("../../../templates/js/runtime/_errors.js").to_string(),
        ),
        (
            "_errors.d.ts".to_string(),
            include_str!("../../../templates/js/runtime/_errors.d.ts").to_string(),
        ),
        (
            "_protocol.js".to_string(),
            include_str!("../../../templates/js/runtime/_protocol.js")
                .replace("{{ protocol_version }}", &protocol_version),
        ),
        (
            "_protocol.d.ts".to_string(),
            include_str!("../../../templates/js/runtime/_protocol.d.ts")
                .replace("{{ protocol_version }}", &protocol_version),
        ),
        (
            "_engine.js".to_string(),
            include_str!("../../../templates/js/runtime/_engine.js").to_string(),
        ),
        (
            "_engine.d.ts".to_string(),
            include_str!("../../../templates/js/runtime/_engine.d.ts").to_string(),
        ),
        (
            "_client.js".to_string(),
            include_str!("../../../templates/js/runtime/_client.js").to_string(),
        ),
        (
            "_client.d.ts".to_string(),
            include_str!("../../../templates/js/runtime/_client.d.ts").to_string(),
        ),
        (
            "_transaction.js".to_string(),
            include_str!("../../../templates/js/runtime/_transaction.js").to_string(),
        ),
        (
            "_transaction.d.ts".to_string(),
            include_str!("../../../templates/js/runtime/_transaction.d.ts").to_string(),
        ),
        (
            "_events.js".to_string(),
            include_str!("../../../templates/js/runtime/_events.js").to_string(),
        ),
        (
            "_events.d.ts".to_string(),
            include_str!("../../../templates/js/runtime/_events.d.ts").to_string(),
        ),
    ]
}
