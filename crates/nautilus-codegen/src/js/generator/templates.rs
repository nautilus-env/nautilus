//! Embedded JavaScript and TypeScript templates and rendering.

use anyhow::Result;
use tera::{Context, Tera};

/// JS/TS template registry — loaded once at first use.
pub static JS_TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "model/codec.js.tera",
            include_str!("../../../templates/js/model/codec.js.tera"),
        ),
        (
            "model/delegate.js.tera",
            include_str!("../../../templates/js/model/delegate.js.tera"),
        ),
        (
            "model/input.d.ts.tera",
            include_str!("../../../templates/js/model/input.d.ts.tera"),
        ),
        (
            "model/events.d.ts.tera",
            include_str!("../../../templates/js/model/events.d.ts.tera"),
        ),
        (
            "model/delegate.d.ts.tera",
            include_str!("../../../templates/js/model/delegate.d.ts.tera"),
        ),
        (
            "model.js.tera",
            include_str!("../../../templates/js/model.js.tera"),
        ),
        (
            "model.d.ts.tera",
            include_str!("../../../templates/js/model.d.ts.tera"),
        ),
        (
            "enums.js.tera",
            include_str!("../../../templates/js/enums.js.tera"),
        ),
        (
            "enums.d.ts.tera",
            include_str!("../../../templates/js/enums.d.ts.tera"),
        ),
        (
            "client.js.tera",
            include_str!("../../../templates/js/client.js.tera"),
        ),
        (
            "client.d.ts.tera",
            include_str!("../../../templates/js/client.d.ts.tera"),
        ),
        (
            "models_index.js.tera",
            include_str!("../../../templates/js/models_index.js.tera"),
        ),
        (
            "models_index.d.ts.tera",
            include_str!("../../../templates/js/models_index.d.ts.tera"),
        ),
        (
            "composite_types.d.ts.tera",
            include_str!("../../../templates/js/composite_types.d.ts.tera"),
        ),
    ])
    .expect("embedded JS templates must parse");
    tera
});

pub(super) fn render(template: &str, ctx: &Context) -> Result<String> {
    crate::template::render(&JS_TEMPLATES, template, ctx)
}
