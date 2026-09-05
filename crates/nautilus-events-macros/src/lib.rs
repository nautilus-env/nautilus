//! Attribute macros that turn annotated functions into event-handler
//! registrations for a generated Nautilus client.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Attribute, FnArg, Ident, Item, ItemFn, ItemMod, LitInt, LitStr, Path, Type,
};

/// Collect the `on_*` handlers of an inline module and append a `register`
/// function that wires each of them into a client's event registry.
#[proc_macro_attribute]
pub fn events(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as EventsArgs);
    let mut module = parse_macro_input!(input as ItemMod);

    let Some((brace, items)) = module.content.take() else {
        return syn::Error::new_spanned(module, "#[events] requires an inline module")
            .to_compile_error()
            .into();
    };

    if let Some(existing) = items.iter().find(|item| defines_register(item)) {
        return syn::Error::new_spanned(
            existing,
            "#[events] appends its own `register` to the module, so the module cannot define one",
        )
        .to_compile_error()
        .into();
    }

    let mut registrations = Vec::new();
    let mut stripped_items = Vec::with_capacity(items.len() + 1);

    for item in items {
        if let Item::Fn(mut function) = item {
            let hooks = match take_hook_attrs(&mut function.attrs) {
                Ok(hooks) => hooks,
                Err(error) => return error.to_compile_error().into(),
            };
            for hook in hooks {
                match build_registration(&args.client_crate, &function, hook) {
                    Ok(tokens) => registrations.push(tokens),
                    Err(error) => return error.to_compile_error().into(),
                }
            }
            stripped_items.push(Item::Fn(function));
        } else {
            stripped_items.push(item);
        }
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

    quote!(#module).into()
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

/// An `on_*` attribute reached as a macro of its own.
///
/// Inside an `#[events]` module the attribute is consumed before it can expand,
/// so getting here means the handler would never be registered — including when
/// a `#[cfg_attr(..., on_create(..))]` hides it from `#[events]`.
fn hook_outside_events(name: &str, input: TokenStream) -> TokenStream {
    let mut expanded = syn::Error::new(
        proc_macro2::Span::call_site(),
        format!(
            "`#[{name}]` registers a handler only inside an `#[events]` module, which consumes the attribute; move the function into one"
        ),
    )
    .to_compile_error();
    expanded.extend(proc_macro2::TokenStream::from(input));
    expanded.into()
}

/// Register a `Create` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_create(_args: TokenStream, input: TokenStream) -> TokenStream {
    hook_outside_events("on_create", input)
}

/// Register a `CreateMany` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_create_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    hook_outside_events("on_create_many", input)
}

/// Register an `Update` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_update(_args: TokenStream, input: TokenStream) -> TokenStream {
    hook_outside_events("on_update", input)
}

/// Register an `UpdateMany` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_update_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    hook_outside_events("on_update_many", input)
}

/// Register a `Delete` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_delete(_args: TokenStream, input: TokenStream) -> TokenStream {
    hook_outside_events("on_delete", input)
}

/// Register a `DeleteMany` handler. Only meaningful inside an `#[events]` module.
#[proc_macro_attribute]
pub fn on_delete_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    hook_outside_events("on_delete_many", input)
}

struct EventsArgs {
    client_crate: Path,
}

impl syn::parse::Parse for EventsArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
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

struct HookAttr {
    operation: &'static str,
    method: Ident,
    model: Path,
    phase: Option<Path>,
    priority: u8,
}

fn take_hook_attrs(attrs: &mut Vec<Attribute>) -> syn::Result<Vec<HookAttr>> {
    let mut hooks = Vec::new();
    let mut retained = Vec::new();

    for attr in attrs.drain(..) {
        let Some((operation, method)) = hook_operation(&attr) else {
            reject_conditional_hook(&attr)?;
            retained.push(attr);
            continue;
        };

        hooks.push(parse_hook_attr(operation, method, &attr)?);
    }

    *attrs = retained;
    Ok(hooks)
}

/// Refuse a hook wrapped in `cfg_attr`.
///
/// `cfg_attr` is expanded after `#[events]` has already read the module, so the
/// hook inside it is invisible here and the handler would never be registered.
/// Gate the handler with `#[cfg]` instead and let the registration follow it.
fn reject_conditional_hook(attr: &Attribute) -> syn::Result<()> {
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
            if hook_method_name(&segment.ident.to_string()).is_some() {
                return Err(syn::Error::new_spanned(
                    meta,
                    "#[events] cannot see a hook inside `cfg_attr`, so the handler would never be registered; put the hook attribute on the function and gate the function itself with `#[cfg(...)]`",
                ));
            }
        }
    }
    Ok(())
}

fn hook_operation(attr: &Attribute) -> Option<(&'static str, Ident)> {
    let ident = attr.path().segments.last()?.ident.to_string();
    hook_method_name(&ident).map(|operation| (operation, format_ident!("{}", ident)))
}

/// The CRUD operation an `on_*` attribute name stands for.
fn hook_method_name(ident: &str) -> Option<&'static str> {
    match ident {
        "on_create" => Some("Create"),
        "on_create_many" => Some("CreateMany"),
        "on_update" => Some("Update"),
        "on_update_many" => Some("UpdateMany"),
        "on_delete" => Some("Delete"),
        "on_delete_many" => Some("DeleteMany"),
        _ => None,
    }
}

fn parse_hook_attr(
    operation: &'static str,
    method: Ident,
    attr: &Attribute,
) -> syn::Result<HookAttr> {
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
        method,
        model,
        phase,
        priority: priority.unwrap_or(0),
    })
}

fn build_registration(
    client_crate: &Path,
    function: &ItemFn,
    hook: HookAttr,
) -> syn::Result<proc_macro2::TokenStream> {
    let fn_name = &function.sig.ident;
    let context_type = context_type(function)?;
    let model_name = model_name_literal(&hook.model)?;
    let phase = hook
        .phase
        .map(|phase| quote!(#phase))
        .unwrap_or_else(|| quote!(#client_crate::EventPhase::Before));
    let method = format_ident!("{}_with_priority", hook.method);
    let priority = hook.priority;
    let result_type = result_type_for_operation(client_crate, &hook.model, hook.operation);
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
                    Box::pin(async move {
                        let output = #call;
                        #client_crate::IntoEventResult::<#result_type>::into_event_result(output)
                    })
                },
            );
        }
    })
}

/// The context type a handler takes, which is also the type the generated
/// closure is instantiated with.
fn context_type(function: &ItemFn) -> syn::Result<Type> {
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

fn model_name_literal(model: &Path) -> syn::Result<LitStr> {
    let Some(segment) = model.segments.last() else {
        return Err(syn::Error::new_spanned(model, "model path cannot be empty"));
    };
    Ok(LitStr::new(
        &segment.ident.to_string(),
        segment.ident.span(),
    ))
}

fn result_type_for_operation(
    client_crate: &Path,
    model: &Path,
    operation: &str,
) -> proc_macro2::TokenStream {
    match operation {
        "Create" => quote!(#model),
        "CreateMany" | "Update" | "DeleteMany" => quote!(std::vec::Vec<#model>),
        "Delete" => quote!(std::option::Option<#model>),
        "UpdateMany" => quote!(u64),
        _ => quote!(#client_crate::Never),
    }
}
