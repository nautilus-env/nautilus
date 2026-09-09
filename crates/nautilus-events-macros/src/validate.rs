//! The uses `#[events]` refuses, each reported on the token that shows it.
//!
//! Every rule here exists because the alternative is a handler that compiles
//! and never runs, or a registration that cannot name what it calls.

use syn::{Attribute, FnArg, Item, ItemFn, Type};

use crate::operation::Operation;

/// Refuse a module that already has a `register`, which the expansion appends.
pub(crate) fn reject_register_collision(items: &[Item]) -> syn::Result<()> {
    let Some(existing) = items.iter().find(|item| defines_register(item)) else {
        return Ok(());
    };
    Err(syn::Error::new_spanned(
        existing,
        "#[events] appends its own `register` to the module, so the module cannot define one",
    ))
}

/// Whether `item` would clash with the `register` function `#[events]` appends.
fn defines_register(item: &Item) -> bool {
    let name = match item {
        Item::Fn(item) => Some(&item.sig.ident),
        Item::Const(item) => Some(&item.ident),
        Item::Static(item) => Some(&item.ident),
        Item::Struct(item) => Some(&item.ident),
        Item::Enum(item) => Some(&item.ident),
        Item::Union(item) => Some(&item.ident),
        Item::Type(item) => Some(&item.ident),
        Item::Mod(item) => Some(&item.ident),
        _ => None,
    };
    name.is_some_and(|name| name == "register")
}

/// Refuse a hook wrapped in `cfg_attr`.
///
/// `cfg_attr` is expanded after `#[events]` has already read the module, so the
/// hook inside it is invisible here and the handler would never be registered.
/// Gate the handler with `#[cfg]` instead and let the registration follow it.
pub(crate) fn reject_conditional_hook(attr: &Attribute) -> syn::Result<()> {
    if !attr.path().is_ident("cfg_attr") {
        return Ok(());
    }
    let Ok(nested) = attr.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    ) else {
        return Ok(());
    };
    for meta in nested.iter().skip(1) {
        if let Some(segment) = meta.path().segments.last() {
            if Operation::from_attr_name(&segment.ident.to_string()).is_some() {
                return Err(syn::Error::new_spanned(
                    meta,
                    "#[events] cannot see a hook inside `cfg_attr`, so the handler would never be registered; put the hook attribute on the function and gate the function itself with `#[cfg(...)]`",
                ));
            }
        }
    }
    Ok(())
}

/// The context type a handler takes, which is also the type the generated
/// closure is instantiated with.
pub(crate) fn handler_context_type(function: &ItemFn) -> syn::Result<Type> {
    let mut inputs = function.sig.inputs.iter();
    let Some(first) = inputs.next() else {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "event handlers must accept a context argument",
        ));
    };
    if let Some(extra) = inputs.next() {
        return Err(syn::Error::new_spanned(
            extra,
            "event handlers take the context argument and nothing else",
        ));
    }
    let FnArg::Typed(arg) = first else {
        return Err(syn::Error::new_spanned(
            first,
            "event handlers cannot use a self receiver",
        ));
    };
    match arg.ty.as_ref() {
        Type::Reference(reference) => Ok((*reference.elem).clone()),
        ty => Ok(ty.clone()),
    }
}
