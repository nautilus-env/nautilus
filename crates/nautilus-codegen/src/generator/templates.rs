//! Embedded Rust templates and rendering.

use anyhow::Result;
use tera::{Context, Tera};

pub static TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "columns_struct.tera",
            include_str!("../../templates/rust/columns_struct.tera"),
        ),
        (
            "column_impl.tera",
            include_str!("../../templates/rust/column_impl.tera"),
        ),
        (
            "create.tera",
            include_str!("../../templates/rust/create.tera"),
        ),
        (
            "create_many.tera",
            include_str!("../../templates/rust/create_many.tera"),
        ),
        (
            "delegate.tera",
            include_str!("../../templates/rust/delegate.tera"),
        ),
        (
            "delete.tera",
            include_str!("../../templates/rust/delete.tera"),
        ),
        ("enum.tera", include_str!("../../templates/rust/enum.tera")),
        (
            "find_many.tera",
            include_str!("../../templates/rust/find_many.tera"),
        ),
        (
            "from_row_impl.tera",
            include_str!("../../templates/rust/from_row_impl.tera"),
        ),
        (
            "model_file.tera",
            include_str!("../../templates/rust/model_file.tera"),
        ),
        (
            "lib_rs.tera",
            include_str!("../../templates/rust/lib_rs.tera"),
        ),
        (
            "model_struct.tera",
            include_str!("../../templates/rust/model_struct.tera"),
        ),
        (
            "update.tera",
            include_str!("../../templates/rust/update.tera"),
        ),
        (
            "composite_type.tera",
            include_str!("../../templates/rust/composite_type.tera"),
        ),
    ])
    .expect("embedded Rust templates must parse");
    tera
});

pub(super) fn render(template: &str, ctx: &Context) -> Result<String> {
    crate::template::render(&TEMPLATES, template, ctx)
}
