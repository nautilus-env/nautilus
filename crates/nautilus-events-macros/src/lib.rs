use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Attribute, FnArg, Ident, Item, ItemFn, ItemMod, LitInt, LitStr, Path, Type,
};

#[proc_macro_attribute]
pub fn events(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as EventsArgs);
    let mut module = parse_macro_input!(input as ItemMod);

    let Some((brace, items)) = module.content.take() else {
        return syn::Error::new_spanned(module, "#[events] requires an inline module")
            .to_compile_error()
            .into();
    };

    let mut registrations = Vec::new();
    let mut stripped_items = Vec::with_capacity(items.len());

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

    module.content = Some((brace, stripped_items));

    let client_crate = &args.client_crate;
    let register_fn = quote! {
        pub fn register<E>(client: &#client_crate::Client<E>)
        where
            E: #client_crate::Executor + 'static,
        {
            #(#registrations)*
        }
    };

    let Some((brace, mut items)) = module.content.take() else {
        unreachable!("module content restored above");
    };
    items.push(syn::parse_quote! { #register_fn });
    module.content = Some((brace, items));

    quote!(#module).into()
}

#[proc_macro_attribute]
pub fn on_create(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_attribute]
pub fn on_create_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_attribute]
pub fn on_update(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_attribute]
pub fn on_delete(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_attribute]
pub fn on_delete_many(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

struct EventsArgs {
    client_crate: Path,
}

impl syn::parse::Parse for EventsArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
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
            retained.push(attr);
            continue;
        };

        hooks.push(parse_hook_attr(operation, method, &attr)?);
    }

    *attrs = retained;
    Ok(hooks)
}

fn hook_operation(attr: &Attribute) -> Option<(&'static str, Ident)> {
    let ident = attr.path().segments.last()?.ident.to_string();
    match ident.as_str() {
        "on_create" => Some(("Create", format_ident!("on_create"))),
        "on_create_many" => Some(("CreateMany", format_ident!("on_create_many"))),
        "on_update" => Some(("Update", format_ident!("on_update"))),
        "on_delete" => Some(("Delete", format_ident!("on_delete"))),
        "on_delete_many" => Some(("DeleteMany", format_ident!("on_delete_many"))),
        _ => None,
    }
}

fn parse_hook_attr(
    operation: &'static str,
    method: Ident,
    attr: &Attribute,
) -> syn::Result<HookAttr> {
    let mut model = None;
    let mut phase = None;
    let mut priority = 0;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("phase") {
            let value = meta.value()?;
            phase = Some(value.parse()?);
            return Ok(());
        }
        if meta.path.is_ident("priority") {
            let value = meta.value()?;
            let literal: LitInt = value.parse()?;
            priority = literal.base10_parse::<u8>()?;
            return Ok(());
        }
        if model.is_none() {
            model = Some(meta.path.clone());
            return Ok(());
        }
        Err(meta.error("unexpected event attribute argument"))
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
        priority,
    })
}

fn build_registration(
    client_crate: &Path,
    function: &ItemFn,
    hook: HookAttr,
) -> syn::Result<proc_macro2::TokenStream> {
    let fn_name = &function.sig.ident;
    let context_type = first_context_type(function)?;
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

    Ok(quote! {
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
    })
}

fn first_context_type(function: &ItemFn) -> syn::Result<Type> {
    let Some(first) = function.sig.inputs.first() else {
        return Err(syn::Error::new_spanned(
            function,
            "event handlers must accept a context argument",
        ));
    };
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
        _ => quote!(#client_crate::Never),
    }
}
