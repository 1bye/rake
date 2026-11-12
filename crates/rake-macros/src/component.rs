use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, ReturnType, Stmt, Type, parse_quote};

pub(super) fn expand(args: TokenStream, mut function: ItemFn) -> syn::Result<TokenStream> {
    let mut force_view = false;
    let mut cache = None;
    let mut cache_disabled = false;
    let options = syn::parse::Parser::parse2(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        args,
    )?;
    for option in options {
        match option {
            syn::Meta::Path(path) if path.is_ident("view") => force_view = true,
            syn::Meta::NameValue(value) if value.path.is_ident("cached") => match value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(value),
                    ..
                }) => cache = Some(value),
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Bool(value),
                    ..
                }) if !value.value => cache_disabled = true,
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "cached expects false or a class string with definite dimensions",
                    ));
                }
            },
            other => return Err(syn::Error::new_spanned(other, "expected view or cached")),
        }
    }
    if function.sig.asyncness.is_some()
        || function.sig.constness.is_some()
        || function.sig.unsafety.is_some()
        || function.sig.abi.is_some()
    {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "components must be ordinary synchronous functions",
        ));
    }
    if matches!(function.sig.output, ReturnType::Default) {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "components must return an element",
        ));
    }
    let output = &function.sig.output;
    let rake = crate::facade()?;
    let name = &function.sig.ident;
    let vis = &function.vis;
    let attrs = &function.attrs;
    let generics = &function.sig.generics;
    let mut fields = Vec::new();
    let mut names = Vec::new();
    let mut types = Vec::new();
    let mut rest = None;
    let mut tracked = Vec::new();
    let mut tracked_types = Vec::new();
    let mut context = None;
    let mut context_type = None;
    for argument in &function.sig.inputs {
        let FnArg::Typed(argument) = argument else {
            return Err(syn::Error::new_spanned(
                argument,
                "component methods are not supported",
            ));
        };
        let Pat::Ident(pattern) = &*argument.pat else {
            return Err(syn::Error::new_spanned(
                &argument.pat,
                "use a named component property",
            ));
        };
        let ident = &pattern.ident;
        let ty = &argument.ty;
        if is_setup(ty) {
            if let Type::Reference(reference) = &**ty
                && let Type::Path(path) = &*reference.elem
            {
                context_type = Some(path.path.clone());
            }
            if context.replace(ident.clone()).is_some() {
                return Err(syn::Error::new_spanned(
                    argument,
                    "a component accepts one Setup parameter",
                ));
            }
            continue;
        }
        let attrs = &argument.attrs;
        if attrs.iter().any(|attr| attr.path().is_ident("rest")) && rest.replace(ident).is_some() {
            return Err(syn::Error::new_spanned(
                argument,
                "a component accepts one rest property",
            ));
        }
        if attrs.iter().any(|attr| {
            attr.path().is_ident("prop")
                && attr.meta.require_list().is_ok_and(|meta| {
                    meta.tokens
                        .to_string()
                        .split(',')
                        .any(|value| value.trim() == "track")
                })
        }) {
            tracked.push(ident);
            tracked_types.push(ty);
        }
        let ty = if attrs.iter().any(|attr| attr.path().is_ident("event")) {
            quote!(#rake::reactive::Event<#ty>)
        } else {
            quote!(#ty)
        };
        fields.push(quote!(#(#attrs)* #[doc = "Component input."] pub #ident: #ty));
        names.push(ident);
        types.push(ty);
    }
    let named_props = names.len() == 1 && names[0] == "props";
    let stateful = force_view || context.is_some();
    if context.is_some()
        && !function.sig.inputs.last().is_some_and(
            |argument| matches!(argument, FnArg::Typed(argument) if is_setup(&argument.ty)),
        )
    {
        return Err(syn::Error::new_spanned(
            &function.sig.inputs,
            "Setup must be the final component parameter",
        ));
    }
    if !stateful && !tracked.is_empty() {
        return Err(syn::Error::new_spanned(
            tracked[0],
            "tracked props require a Setup parameter",
        ));
    }
    if stateful && names.iter().any(|name| *name == "id") {
        return Err(syn::Error::new_spanned(
            name,
            "id is reserved for stateful component identity",
        ));
    }
    let mut implementation = generics.clone();
    if stateful {
        for ty in &tracked_types {
            implementation
                .make_where_clause()
                .predicates
                .push(parse_quote!(#ty: PartialEq));
        }
        for ty in &types {
            implementation
                .make_where_clause()
                .predicates
                .push(parse_quote!(#ty: Clone + 'static));
        }
    }
    let (impl_generics, _, where_clause) = implementation.split_for_impl();
    let (_, type_generics, _) = generics.split_for_impl();
    let identity = stateful.then(|| quote!(#[doc = "Stable component identity within its parent."] pub id: #rake::gpui::ElementId,));
    let clone = stateful.then(|| quote!(#[derive(Clone)] #[props(stateful)]));
    let body = if stateful {
        let mut tail = match function.block.stmts.pop() {
            Some(Stmt::Expr(tail, None)) => tail,
            Some(Stmt::Macro(statement)) if statement.semi_token.is_none() => {
                syn::Expr::Macro(syn::ExprMacro {
                    attrs: statement.attrs,
                    mac: statement.mac,
                })
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    &function.block,
                    "the final expression must render the component",
                ));
            }
        };
        let style = if cache_disabled || (rest.is_some() && cache.is_none()) {
            None
        } else if let Some(classes) = cache {
            let markup: rake_view_syntax::View = syn::parse2(quote!(<div class=#classes/>))?;
            Some(markup.cache_style(&rake)?.ok_or_else(|| {
                syn::Error::new_spanned(classes, "cached requires explicit width and height")
            })?)
        } else if let syn::Expr::Macro(expression) = &tail {
            if expression
                .mac
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "view")
            {
                syn::parse2::<rake_view_syntax::View>(expression.mac.tokens.clone())?
                    .cache_style(&rake)?
            } else {
                None
            }
        } else {
            None
        };
        if let Some(rest) = rest {
            tail = forward_rest(tail, rest, &rake)?;
        }
        let mounted = style.map_or_else(|| quote!(#rake::gpui::IntoElement::into_any_element(__view)), |style| quote!(if __changed { #rake::gpui::IntoElement::into_any_element(__view) } else { #rake::gpui::IntoElement::into_any_element(__view.cached(#style)) }));
        let setup = &function.block.stmts;
        crate::setup_reads::validate(setup, &tracked)?;
        let tracked_storage: Vec<_> = tracked
            .iter()
            .map(|name| format_ident!("__rake_tracked_{name}"))
            .collect();
        let context_type = context_type
            .map(|mut path| {
                path.segments.last_mut().unwrap().arguments =
                    syn::PathArguments::AngleBracketed(parse_quote!(<'_, '_, _>));
                quote!(#path)
            })
            .unwrap_or_else(|| quote!(#rake::reactive::Setup<'_, '_, _>));
        let context = context.unwrap_or_else(|| syn::Ident::new("cx", name.span()));
        let instrumented = setup.iter().map(|statement| {
            let mut tokens = quote!(#statement);
            if let Stmt::Local(local) = statement
                && let Pat::Ident(binding) = &local.pat
                && let Some(init) = &local.init
                && let syn::Expr::MethodCall(call) = &*init.expr
                && matches!(
                    call.method.to_string().as_str(),
                    "signal" | "memo" | "resource"
                )
            {
                let method = format_ident!("inspect_{}", call.method);
                let binding = &binding.ident;
                tokens.extend(quote!(#context.#method(stringify!(#binding), #binding);));
            }
            tokens
        });
        let setup_hash = quote!(#(#tracked)* #(#setup)*)
            .to_string()
            .bytes()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        quote! {
            let __id = self.id.clone();
            let mut __props = Some(self);
            let __view = window.with_id(#setup_hash, |window| {
                window.use_keyed_state(__id, cx, |window, __component_cx| {
                    let __props = __props.take().expect("mount consumes its props once");
                    #rake::reactive::ComponentView::observe_release(__component_cx);
                    let mut __setup = #rake::reactive::Setup::new(window, __component_cx);
                    let #context: &mut #context_type = &mut __setup;
                    #(let #names = __props.#names.clone();)*
                    #(let #tracked_storage = #context.signal(#tracked.clone());
                      let #tracked = #rake::reactive::Memo::from_signal(#tracked_storage);)*
                    #(#instrumented)*
                    let __render = move |__props: &Self, window: &mut #rake::gpui::Window, #context: &mut #rake::gpui::App| {
                        #(let #names = __props.#names.clone();)*
                        #rake::gpui::IntoElement::into_any_element(#tail)
                    };
                    __setup.finish_with(__props, __render, move |__props, cx| {
                        #(#tracked_storage.set_prop(cx, __props.#tracked.clone());)*
                    })
                })
            });
            let __changed = __props.is_some_and(|props| __view.update(cx, |view, cx| view.swap_props(props, cx, |previous, next| {
                use #rake::__private::EquivalentProp as _;
                true #(&& (&#rake::__private::PropComparison(&previous.#names, &next.#names)).equivalent())*
            })));
            #mounted
        }
    } else {
        if let Some(rest) = rest {
            let last = function
                .block
                .stmts
                .last_mut()
                .ok_or_else(|| syn::Error::new_spanned(name, "rest requires a view! root"))?;
            let expression = match last {
                Stmt::Expr(expression, None) => expression.clone(),
                Stmt::Macro(statement) if statement.semi_token.is_none() => {
                    syn::Expr::Macro(syn::ExprMacro {
                        attrs: statement.attrs.clone(),
                        mac: statement.mac.clone(),
                    })
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        last,
                        "rest requires a final view! expression",
                    ));
                }
            };
            *last = Stmt::Expr(forward_rest(expression, rest, &rake)?, None);
        }
        let statements = &function.block.stmts;
        quote! {
            let Self { #(#names,)* } = self;
            #(#statements)*
        }
    };
    let derive_props = if named_props {
        None
    } else {
        Some(quote!(#[derive(#rake::Props)]))
    };
    let delegate = if named_props && !stateful {
        let ty = &types[0];
        quote! {
            impl #impl_generics From<#ty> for #name #type_generics #where_clause {
                fn from(props: #ty) -> Self { Self { props } }
            }
            impl #impl_generics From<#name #type_generics> for #ty #where_clause {
                fn from(component: #name #type_generics) -> Self { component.props }
            }
            impl #impl_generics #name #type_generics #where_clause {
                #[doc(hidden)]
                pub fn into_props(self) -> #ty { self.props }
                #[doc(hidden)]
                pub fn builder_empty() -> <#ty as #rake::__private::PropsBuilder>::EmptyBuilder { <#ty as #rake::__private::PropsBuilder>::builder_empty() }
                #[doc = "Builds this component using its named props struct."]
                pub fn builder() -> <#ty as #rake::__private::PropsBuilder>::Builder { <#ty as #rake::__private::PropsBuilder>::builder() }
                #[doc = "Builds from an existing component's properties."]
                pub fn builder_from(base: Self) -> <#ty as #rake::__private::PropsBuilder>::FilledBuilder { <#ty as #rake::__private::PropsBuilder>::builder_from(base.props) }
            }
        }
    } else {
        quote!()
    };
    let derive_props = if stateful {
        Some(quote!(#[derive(#rake::Props)]))
    } else {
        derive_props
    };
    Ok(quote! {
        #(#attrs)*
        #derive_props
        #clone
        #[doc = "Properties for a function component."]
        #vis struct #name #generics {
            #identity
            #(#fields,)*
        }
        #delegate
        impl #impl_generics #rake::gpui::RenderOnce for #name #type_generics #where_clause {
            #[allow(unused_variables, reason = "Setup and render may use different props and GPUI context arguments.")]
            fn render(self, window: &mut #rake::gpui::Window, cx: &mut #rake::gpui::App) #output {
                #body
            }
        }
        impl #impl_generics #rake::gpui::IntoElement for #name #type_generics #where_clause {
            type Element = #rake::gpui::ViewElement<Self>;
            fn into_element(self) -> Self::Element {
                #rake::gpui::ViewElement::new(self)
            }
        }
    })
}

fn is_setup(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference) if reference.mutability.is_some() && matches!(&*reference.elem, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Setup")))
}

fn forward_rest(
    expression: syn::Expr,
    rest: &syn::Ident,
    rake: &TokenStream,
) -> syn::Result<syn::Expr> {
    let syn::Expr::Macro(expression) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "rest requires a final view! expression",
        ));
    };
    let mut view: rake_view_syntax::View = syn::parse2(expression.mac.tokens)?;
    view.forward_rest(rest)?;
    syn::parse2(view.expand_resilient(rake)?)
}
