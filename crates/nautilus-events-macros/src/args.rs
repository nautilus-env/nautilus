//! Reading the attributes into typed data: the arguments of `#[events]` and
//! the hooks a handler carries.

use proc_macro2::Span;
use syn::{Attribute, Ident, LitInt, Path};

use crate::operation::Operation;
use crate::validate;

/// The arguments of `#[events]`.
pub(crate) struct EventsArgs {
    /// The path the expansion reaches the client's own types through.
    pub(crate) client_crate: Path,
}

impl syn::parse::Parse for EventsArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "#[events] requires `client_crate = path`",
            ));
        }
        let ident: Ident = input.parse()?;
        if ident != "client_crate" {
            return Err(syn::Error::new_spanned(
                ident,
                "expected `client_crate = path`",
            ));
        }
        input.parse::<syn::Token![=]>()?;
        let client_crate = input.parse()?;
        Ok(Self { client_crate })
    }
}

/// One `on_*` attribute, read off a handler.
pub(crate) struct HookAttr {
    pub(crate) operation: Operation,
    /// Span of the attribute name that asked for the registration, carried so
    /// the tokens generated for it report against it.
    pub(crate) span: Span,
    pub(crate) model: Path,
    pub(crate) phase: Option<Path>,
    pub(crate) priority: u8,
}

/// Take the `on_*` attributes off a handler and read them.
///
/// Consuming them here is what makes reaching the standalone `on_*` macros an
/// error: it means the handler is outside an `#[events]` module.
pub(crate) fn take_hook_attrs(attrs: &mut Vec<Attribute>) -> syn::Result<Vec<HookAttr>> {
    let mut hooks = Vec::new();
    let mut retained = Vec::new();

    for attr in attrs.drain(..) {
        let Some((operation, span)) = hook_operation(&attr) else {
            validate::reject_conditional_hook(&attr)?;
            retained.push(attr);
            continue;
        };

        hooks.push(parse_hook_attr(operation, span, &attr)?);
    }

    *attrs = retained;
    Ok(hooks)
}

/// The operation an attribute asks for, with the span of the name itself.
fn hook_operation(attr: &Attribute) -> Option<(Operation, Span)> {
    let segment = attr.path().segments.last()?;
    let operation = Operation::from_attr_name(&segment.ident.to_string())?;
    Some((operation, segment.ident.span()))
}

fn parse_hook_attr(operation: Operation, span: Span, attr: &Attribute) -> syn::Result<HookAttr> {
    let mut model: Option<Path> = None;
    let mut phase: Option<Path> = None;
    let mut priority: Option<u8> = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("phase") {
            if phase.is_some() {
                return Err(meta.error("`phase` is given more than once"));
            }
            phase = Some(meta.value()?.parse()?);
            return Ok(());
        }
        if meta.path.is_ident("priority") {
            if priority.is_some() {
                return Err(meta.error("`priority` is given more than once"));
            }
            let literal: LitInt = meta.value()?.parse()?;
            priority = Some(literal.base10_parse::<u8>()?);
            return Ok(());
        }
        if model.is_some() {
            return Err(meta.error(
                "an event attribute takes one model, plus optional `phase` and `priority`",
            ));
        }
        model = Some(meta.path.clone());
        Ok(())
    })?;

    let model = model.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            "event attributes require a model, e.g. #[on_create(User)]",
        )
    })?;

    Ok(HookAttr {
        operation,
        span,
        model,
        phase,
        priority: priority.unwrap_or(0),
    })
}
