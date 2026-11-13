use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Expr, Fields, GenericParam, Type, parse_quote};

pub(super) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let mut runtime = None;
    let mut stateful = false;

    for attr in &input.attrs {
        if attr.path().is_ident("props") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("stateful") {
                    stateful = true;

                    return Ok(());
                }

                if !meta.path.is_ident("runtime") {
                    return Err(meta.error("expected runtime or stateful"));
                }

                runtime = Some(meta.value()?.parse::<syn::Path>()?);

                Ok(())
            })?;
        }
    }

    let rake = match runtime {
        Some(path) => quote!(#path),
        None => crate::facade()?,
    };

    let name = &input.ident;
    let visibility = &input.vis;
    let builder = format_ident!("{name}Builder");

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            name,
            "Props requires a struct with named fields",
        ));
    };

    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(name, "Props requires named fields"));
    };

    let names: Vec<_> = fields
        .named
        .iter()
        .map(|field| field.ident.as_ref().unwrap())
        .collect();

    let types: Vec<_> = fields.named.iter().map(|field| &field.ty).collect();

    let states: Vec<_> = names
        .iter()
        .map(|name| format_ident!("__Prop_{name}"))
        .collect();

    let mut defaults = Vec::new();
    let mut no_into = Vec::new();

    for field in &fields.named {
        let mut default = None;
        let mut direct = matches!(&field.ty, Type::Path(path) if path.path.get_ident().is_some_and(|name| input.generics.type_params().any(|param| param.ident == *name)));

        for attr in &field.attrs {
            if attr.path().is_ident("prop") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("default") {
                        default = Some(if meta.input.peek(syn::Token![=]) {
                            meta.value()?.parse::<Expr>()?
                        } else {
                            parse_quote!(::core::default::Default::default())
                        });
                    } else if meta.path.is_ident("no_into") {
                        direct = true;
                    } else if !meta.path.is_ident("track") {
                        return Err(meta.error("expected default, no_into, or track"));
                    }

                    Ok(())
                })?;
            }
        }

        if default.is_none()
            && (field.ident.as_ref().unwrap() == "children"
                || optional(&field.ty)
                || field
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("event") || attr.path().is_ident("rest")))
        {
            default = Some(parse_quote!(::core::default::Default::default()));
        }

        defaults.push(default);
        no_into.push(direct);
    }

    let initial: Vec<_> = defaults
        .iter()
        .map(|default| {
            if default.is_some() {
                quote!(#rake::__private::PropSet)
            } else {
                quote!(#rake::__private::PropMissing)
            }
        })
        .collect();

    let all_set: Vec<_> = states
        .iter()
        .map(|_| quote!(#rake::__private::PropSet))
        .collect();

    let arguments: Vec<TokenStream> = input
        .generics
        .params
        .iter()
        .map(|param| match param {
            GenericParam::Type(value) => {
                let name = &value.ident;

                quote!(#name)
            }
            GenericParam::Lifetime(value) => {
                let name = &value.lifetime;

                quote!(#name)
            }
            GenericParam::Const(value) => {
                let name = &value.ident;

                quote!(#name)
            }
        })
        .collect();

    let mut builder_generics = input.generics.clone();

    for param in &mut builder_generics.params {
        match param {
            GenericParam::Type(param) => param.default = None,
            GenericParam::Const(param) => param.default = None,
            GenericParam::Lifetime(_) => {}
        }
    }

    for state in &states {
        builder_generics.params.push(parse_quote!(#state));
    }

    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let (builder_impl, _, builder_where) = builder_generics.split_for_impl();

    let empty: Vec<_> = states
        .iter()
        .map(|_| quote!(#rake::__private::PropMissing))
        .collect();

    let build_values = names
        .iter()
        .zip(&defaults)
        .map(|(name, default)| match default {
            Some(Expr::Call(call)) if call.args.is_empty() => {
                let function = &call.func;

                quote!(self.#name.unwrap_or_else(#function))
            }
            Some(default) => quote!(self.#name.unwrap_or_else(|| #default)),
            None => quote!(self.#name.expect("property checked by the builder's type state")),
        });

    let methods = names.iter().enumerate().map(|(index, field)| {
        let ty = types[index];
        let field_attributes = &fields.named[index].attrs;
        let method = if field_attributes.iter().any(|attr| attr.path().is_ident("event")) { format_ident!("on_{field}") } else { (*field).clone() };
        let event = field_attributes.iter().any(|attr| attr.path().is_ident("event"));

        let event_type = match ty {
            Type::Path(path) if event => match &path.path.segments.last().unwrap().arguments {
                syn::PathArguments::AngleBracketed(args) => args.args.first().map(|ty| quote!(#ty)),
                _ => None,
            },
            _ => None,
        };

        let value = if let Some(event) = &event_type { quote!(impl Fn(&#event, &mut #rake::gpui::Window, &mut #rake::gpui::App) + 'static) } else if no_into[index] { quote!(#ty) } else { quote!(impl ::core::convert::Into<#ty>) };
        let converted = if event { quote!(#rake::reactive::Event::new(value)) } else if no_into[index] { quote!(value) } else { quote!(value.into()) };
        let output_states: Vec<_> = states.iter().enumerate().map(|(i,state)| if i==index { quote!(#rake::__private::PropSet) } else { quote!(#state) }).collect();
        let values = names.iter().map(|name| if *name==*field { quote!(#name: Some(#converted)) } else { quote!(#name: self.#name) });
        let factory = format_ident!("{field}_with");

        let factory_method = if event { quote!() } else if stateful {
            let children = matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Children"));

            if children {
                let (output, wrap) = if *field == "children" {
                    (quote!(Vec<#rake::gpui::AnyElement>), quote!({
                        use #rake::gpui::{IntoElement as _, ParentElement as _};
                        #rake::gpui::div().children(render(window, cx)).into_any_element()
                    }))
                } else { (quote!(#rake::gpui::AnyElement), quote!(render(window, cx))) };

                quote! {
                    #[doc(hidden)]
                    pub fn #factory(self, _: &mut #rake::gpui::Window, _: &mut #rake::gpui::App, render: impl Fn(&mut #rake::gpui::Window, &mut #rake::gpui::App) -> #output + 'static) -> #builder<#(#arguments,)* #(#output_states),*> {
                        self.#method(#rake::reactive::Children::new(move |window, cx| #wrap))
                    }
                }
            } else { quote!() }
        } else {
            quote! {
                #[doc(hidden)]
                pub fn #factory<C>(self, window: &mut #rake::gpui::Window, cx: &mut C, render: impl FnOnce(&mut #rake::gpui::Window, &mut C) -> #ty) -> #builder<#(#arguments,)* #(#output_states),*> {
                    self.#method(render(window, cx))
                }
            }
        };

        quote! {
            #factory_method
            #[doc = "Sets this component property."]
            pub fn #method(self, value: #value) -> #builder<#(#arguments,)* #(#output_states),*> {
                #builder { #(#values,)* __state: ::core::marker::PhantomData }
            }
        }
    });

    let rest_methods = fields.named.iter().find(|field| field.attrs.iter().any(|attr| attr.path().is_ident("rest"))).map(|field| {
        let rest = field.ident.as_ref().unwrap();
        let declared = |name: &str| names.iter().any(|field| *field == name);
        let mut methods = TokenStream::new();

        for (method, ty, property, converted) in [
            ("class", quote!(#rake::gpui::StyleRefinement), "style", quote!(value)),
            ("aria_label", quote!(impl Into<#rake::gpui::SharedString>), "aria_label", quote!(Some(value.into()))),
            ("aria_description", quote!(impl Into<#rake::gpui::SharedString>), "aria_description", quote!(Some(value.into()))),
            ("aria_expanded", quote!(bool), "aria_expanded", quote!(Some(value))),
            ("disabled", quote!(bool), "disabled", quote!(value)),
        ] {
            if declared(method) { continue; }

            let method = format_ident!("{method}");
            let property = format_ident!("{property}");

            methods.extend(quote! {
                #[doc = "Forwards an attribute to the component root."]
                pub fn #method(mut self, value: #ty) -> Self {
                    self.#rest.get_or_insert_with(Default::default).#property = #converted;
                    self
                }
            });
        }

        if !declared("click") {
            methods.extend(quote! {
                #[doc = "Forwards a click handler to the component root."]
                pub fn on_click(mut self, callback: impl Fn(&#rake::gpui::ClickEvent, &mut #rake::gpui::Window, &mut #rake::gpui::App) + 'static) -> Self {
                    self.#rest.get_or_insert_with(Default::default).on_click(callback);
                    self
                }
            });
        }

        methods
    });

    let mut build_generics = builder_generics.clone();

    for (field, state) in names.iter().zip(&states) {
        let marker = format_ident!("__{name}_{field}");

        build_generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(#state: #rake::__private::RequiredProp<#marker>));
    }

    let (_, _, build_where) = build_generics.split_for_impl();

    let markers = names.iter().map(|field| { let marker=format_ident!("__{name}_{field}");

        quote!(#[doc(hidden)] #[allow(non_camel_case_types, reason = "Generated prop names retain the authored field spelling.")] #visibility struct #marker;) });

    Ok(quote! {
        #(#markers)*
        #[doc(hidden)]
        #[allow(non_camel_case_types, reason = "Generated prop states retain the authored field spelling.")]
        #visibility struct #builder #builder_generics #builder_where {
            #(#names: ::core::option::Option<#types>,)*
            __state: ::core::marker::PhantomData<(#(#states,)*)>,
        }
        impl #impl_generics #rake::__private::PropsBuilder for #name #type_generics #where_clause {
            type Builder = #builder<#(#arguments,)* #(#initial),*>;
            type EmptyBuilder = #builder<#(#arguments,)* #(#empty),*>;
            fn builder_empty() -> Self::EmptyBuilder { Self::builder_empty() }
            type FilledBuilder = #builder<#(#arguments,)* #(#all_set),*>;
            fn builder() -> Self::Builder { Self::builder() }
            fn builder_from(props: Self) -> Self::FilledBuilder { Self::builder_from(props) }
        }
        impl #impl_generics #name #type_generics #where_clause {
            #[doc(hidden)]
            pub fn into_props(self) -> Self { self }
            #[doc = "Starts a builder with declared defaults and optional properties."]
            pub fn builder() -> #builder<#(#arguments,)* #(#initial),*> {
                #builder { #(#names: None,)* __state: ::core::marker::PhantomData }
            }
            #[doc(hidden)]
            pub fn builder_empty() -> #builder<#(#arguments,)* #(#empty),*> {
                #builder { #(#names: None,)* __state: ::core::marker::PhantomData }
            }
            #[doc = "Starts a builder from existing properties, preserving struct-update behavior."]
            pub fn builder_from(base: Self) -> #builder<#(#arguments,)* #(#all_set),*> {
                #builder { #(#names: Some(base.#names),)* __state: ::core::marker::PhantomData }
            }
        }
        #[allow(non_camel_case_types, reason = "Generated prop states retain the authored field spelling.")]
        impl #builder_impl #builder<#(#arguments,)* #(#states),*> #builder_where {
            #(#methods)*
            #rest_methods
            #[doc(hidden)]
            pub fn build_from(self, base: #name #type_generics) -> #name #type_generics {
                #name { #(#names: self.#names.unwrap_or(base.#names),)* }
            }
        }
        #[allow(non_camel_case_types, reason = "Generated prop states retain the authored field spelling.")]
        impl #builder_impl #builder<#(#arguments,)* #(#states),*> #builder_where {
            #[doc = "Builds the component once every required property has been supplied."]
            pub fn build(self) -> #name #type_generics #build_where {
                #name { #(#names: #build_values,)* }
            }
        }
    })
}

fn optional(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Option" || segment.ident == "bool"))
}
