//! Embedded Python templates and rendering.

use anyhow::Result;
use tera::{Context, Tera};

/// Python template registry — loaded once at first use.
pub static PYTHON_TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "composite_types.py.tera",
            include_str!("../../../templates/python/composite_types.py.tera"),
        ),
        (
            "model_file.py.tera",
            include_str!("../../../templates/python/model_file.py.tera"),
        ),
        (
            "input_types.py.tera",
            include_str!("../../../templates/python/input_types.py.tera"),
        ),
        (
            "enums.py.tera",
            include_str!("../../../templates/python/enums.py.tera"),
        ),
        (
            "client.py.tera",
            include_str!("../../../templates/python/client.py.tera"),
        ),
        (
            "package_init.py.tera",
            include_str!("../../../templates/python/package_init.py.tera"),
        ),
        (
            "models_init.py.tera",
            include_str!("../../../templates/python/models_init.py.tera"),
        ),
        (
            "enums_init.py.tera",
            include_str!("../../../templates/python/enums_init.py.tera"),
        ),
        (
            "errors_init.py.tera",
            include_str!("../../../templates/python/errors_init.py.tera"),
        ),
        (
            "internal_init.py.tera",
            include_str!("../../../templates/python/internal_init.py.tera"),
        ),
        (
            "transaction_init.py.tera",
            include_str!("../../../templates/python/transaction_init.py.tera"),
        ),
        (
            "events.py.tera",
            include_str!("../../../templates/python/events.py.tera"),
        ),
    ])
    .expect("embedded Python templates must parse");
    tera
});

pub(super) fn render(template: &str, ctx: &Context) -> Result<String> {
    crate::template::render(&PYTHON_TEMPLATES, template, ctx)
}
