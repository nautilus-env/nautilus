//! Writing the registrations: the `register` function `#[events]` appends and
//! the tokens each hook contributes to it.
//!
//! The expansion lands inside the user's module, next to whatever that module
//! has imported, so every path it writes is absolute: a schema is free to have
//! a model called `Box` or `Option`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Item, ItemFn, ItemMod, LitStr, Path};

use crate::args::{take_hook_attrs, EventsArgs, HookAttr};
use crate::operation::Operation;
use crate::validate;

/// Strip the hooks off the functions of an `#[events]` module and append the
/// `register` that wires them into a client.
pub(crate) fn events_module(args: &EventsArgs, mut module: ItemMod) -> syn::Result<TokenStream> {
    let Some((brace, items)) = module.content.take() else {
        return Err(syn::Error::new_spanned(
            module,
            "#[events] requires an inline module",
        ));
    };

    validate::reject_register_collision(&items)?;

    let mut registrations = Vec::new();
    let mut stripped_items = Vec::with_capacity(items.len() + 1);

    for item in items {
        let Item::Fn(mut function) = item else {
            stripped_items.push(item);
            continue;
        };
        for hook in take_hook_attrs(&mut function.attrs)? {
            registrations.push(registration(&args.client_crate, &function, hook)?);
        }
        stripped_items.push(Item::Fn(function));
    }

    let client_crate = &args.client_crate;
    stripped_items.push(syn::parse_quote! {
        pub fn register<E>(client: &#client_crate::Client<E>)
        where
            E: #client_crate::Executor + 'static,
        {
            #(#registrations)*
        }
    });
    module.content = Some((brace, stripped_items));

    Ok(quote!(#module))
}

/// The tokens one hook contributes to the body of `register`.
fn registration(
    client_crate: &Path,
    function: &ItemFn,
    hook: HookAttr,
) -> syn::Result<TokenStream> {
    let fn_name = &function.sig.ident;
    let context_type = validate::handler_context_type(function)?;
    let model_name = model_name_literal(&hook.model)?;
    let method = hook.operation.registry_method(hook.span);
    let result_type = hook.operation.result_type(&hook.model);
    let priority = hook.priority;
    let phase = hook
        .phase
        .map(|phase| quote!(#phase))
        .unwrap_or_else(|| quote!(#client_crate::EventPhase::Before));
    let call = if function.sig.asyncness.is_some() {
        quote!(#fn_name(ctx).await)
    } else {
        quote!(#fn_name(ctx))
    };
    // The handler keeps its own `#[cfg]`s, so the registration has to disappear
    // with it rather than call a function that was not compiled.
    let cfgs = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"));

    Ok(quote! {
        #(#cfgs)*
        {
            client.events().#method::<#context_type, #result_type, _>(
                #model_name,
                #phase,
                #priority,
                |ctx| {
                    ::std::boxed::Box::pin(async move {
                        let output = #call;
                        #client_crate::IntoEventResult::<#result_type>::into_event_result(output)
                    })
                },
            );
        }
    })
}

/// The model a handler is registered for, as the name the registry keys on.
fn model_name_literal(model: &Path) -> syn::Result<LitStr> {
    let Some(segment) = model.segments.last() else {
        return Err(syn::Error::new_spanned(model, "model path cannot be empty"));
    };
    Ok(LitStr::new(
        &segment.ident.to_string(),
        segment.ident.span(),
    ))
}

/// An `on_*` attribute reached as a macro of its own.
///
/// Inside an `#[events]` module the attribute is consumed before it can expand,
/// so getting here means the handler would never be registered — including when
/// a `#[cfg_attr(..., on_create(..))]` hides it from `#[events]`. The function
/// is kept so the rest of the file still resolves.
pub(crate) fn hook_outside_events(operation: Operation, input: TokenStream) -> TokenStream {
    let name = operation.attr_name();
    let mut expanded = syn::Error::new(
        Span::call_site(),
        format!(
            "`#[{name}]` registers a handler only inside an `#[events]` module, which consumes the attribute; move the function into one"
        ),
    )
    .to_compile_error();
    expanded.extend(input);
    expanded
}
