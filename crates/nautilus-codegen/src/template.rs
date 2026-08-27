//! Shared helpers for rendering Tera templates across codegen backends.

use anyhow::{Context as _, Result};
use tera::{Context, Tera};

/// Render `template` from `tera` with `ctx`, normalizing CRLF line endings.
///
/// Tera on Windows can emit `\r\n` depending on how template strings are
/// embedded at build time. All codegen backends want LF-only output so
/// generated sources hash consistently across platforms.
///
/// Templates are embedded at compile time, so a render failure is a generator
/// bug rather than user error. It is still reported as an error carrying the
/// template name, so a regressed template surfaces as a CLI message naming it
/// instead of a panic backtrace.
pub(crate) fn render(tera: &Tera, template: &str, ctx: &Context) -> Result<String> {
    let rendered = tera
        .render(template, ctx)
        .with_context(|| format!("Failed to render template '{}'", template))?;
    Ok(rendered.replace("\r\n", "\n"))
}

/// Insert the current wire protocol version into a Tera context.
pub(crate) fn insert_protocol_version(ctx: &mut Context) {
    ctx.insert("protocol_version", &nautilus_protocol::PROTOCOL_VERSION);
}
