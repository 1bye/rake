use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, LitStr, Type};

pub(super) fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let rake = super::facade()?;
    let name = input.ident;

    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            name,
            "route enums must have a concrete, closed schema",
        ));
    }

    let Data::Enum(data) = input.data else {
        return Err(syn::Error::new_spanned(name, "Route requires an enum"));
    };

    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut patterns: Vec<Vec<Option<String>>> = Vec::new();
    let mut descriptors = Vec::new();
    let mut encoders = Vec::new();
    let mut decoders = Vec::new();

    for variant in data.variants {
        let mut id = None;
        let mut path = None;

        for attr in variant
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("route"))
        {
            attr.parse_nested_meta(|meta| {
                let value: LitStr = meta.value()?.parse()?;

                if meta.path.is_ident("id") && id.is_none() {
                    id = Some(value);
                } else if meta.path.is_ident("path") && path.is_none() {
                    path = Some(value);
                } else {
                    return Err(meta.error("expected one id and one path"));
                }

                Ok(())
            })?;
        }

        let id = id.ok_or_else(|| syn::Error::new_spanned(&variant.ident, "missing route id"))?;

        let path =
            path.ok_or_else(|| syn::Error::new_spanned(&variant.ident, "missing route path"))?;

        let path_value = path.value();

        if id.value().is_empty() || !ids.insert(id.value()) {
            return Err(syn::Error::new_spanned(
                id,
                "route ids must be nonempty and unique",
            ));
        }

        if !path_value.starts_with('/')
            || (path_value != "/" && path_value.split('/').skip(1).any(str::is_empty))
        {
            return Err(syn::Error::new_spanned(
                path,
                "expected / or an absolute path without empty segments",
            ));
        }

        let mut placeholders = Vec::new();
        let mut shape = Vec::new();
        let mut pattern = Vec::new();

        for segment in path_value.split('/') {
            if let Some(parameter) = segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
            {
                if parameter.is_empty() || placeholders.contains(&parameter.to_string()) {
                    return Err(syn::Error::new_spanned(
                        &path,
                        "path placeholders must be nonempty and unique",
                    ));
                }

                placeholders.push(parameter.to_string());
                shape.push("{}");
                pattern.push(None);
            } else {
                if matches!(segment, "." | "..")
                    || !segment
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte))
                {
                    return Err(syn::Error::new_spanned(
                        &path,
                        "static path segments must use URL-safe characters",
                    ));
                }

                shape.push(segment);
                pattern.push(Some(segment.to_owned()));
            }
        }

        if !paths.insert(shape.join("/")) {
            return Err(syn::Error::new_spanned(
                path,
                "ambiguous route path pattern",
            ));
        }

        let parameter_count = pattern.iter().filter(|segment| segment.is_none()).count();

        if patterns.iter().any(|previous| {
            previous.len() == pattern.len()
                && previous.iter().filter(|segment| segment.is_none()).count() == parameter_count
                && previous.iter().zip(&pattern).all(|(previous, current)| {
                    match (previous, current) {
                        (Some(previous), Some(current)) => previous == current,
                        _ => true,
                    }
                })
        }) {
            return Err(syn::Error::new_spanned(
                path,
                "route path patterns overlap at equal specificity; static segments must disambiguate routes",
            ));
        }

        patterns.push(pattern);

        let variant_name = variant.ident;
        let mut field_names = Vec::new();
        let mut bindings = Vec::new();
        let mut names = Vec::new();
        let mut parameters = Vec::new();
        let mut inserts = Vec::new();
        let mut parses = Vec::new();

        for (index, field) in variant.fields.iter().enumerate() {
            let binding = format_ident!("__rake_parameter_{index}", span = Span::mixed_site());

            if let Some(name) = &field.ident {
                field_names.push(name.clone());
            }

            let parameter = match &field.ident {
                Some(name) => name.to_string(),
                None if variant.fields.len() == 1 && placeholders.len() == 1 => {
                    placeholders[0].clone()
                }
                None => {
                    return Err(syn::Error::new_spanned(
                        &variant_name,
                        "tuple routes require exactly one field and path placeholder; use named fields otherwise",
                    ));
                }
            };

            let optional = option_inner(&field.ty);
            let ty = optional.unwrap_or(&field.ty);

            if optional.is_some() && placeholders.contains(&parameter) {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "path parameters cannot be optional",
                ));
            }

            let required = optional.is_none();

            parameters.push(quote!(#rake::router::ParameterDescriptor { name: #parameter, value_type: stringify!(#ty), required: #required }));

            if optional.is_some() {
                inserts.push(quote!(if let Some(value) = #binding { params.insert(#parameter.into(), #rake::router::RouteParam::format_param(value)); }));
                parses.push(quote!(address.params.get(#parameter).map(|_| address.parameter::<#ty>(#parameter)).transpose()?));
            } else {
                inserts.push(quote!(params.insert(#parameter.into(), #rake::router::RouteParam::format_param(#binding));));
                parses.push(quote!(address.parameter::<#ty>(#parameter)?));
            }

            names.push(parameter);
            bindings.push(binding);
        }

        for placeholder in placeholders {
            if !names.contains(&placeholder) {
                return Err(syn::Error::new_spanned(
                    &path,
                    "path placeholder has no matching field",
                ));
            }
        }

        let (pattern, constructor) = match variant.fields {
            Fields::Unit => (quote!(Self::#variant_name), quote!(Self::#variant_name)),
            Fields::Unnamed(_) => (
                quote!(Self::#variant_name(#(#bindings),*)),
                quote!(Self::#variant_name(#(#parses),*)),
            ),
            Fields::Named(_) => (
                quote!(Self::#variant_name { #(#field_names: #bindings),* }),
                quote!(Self::#variant_name { #(#field_names: #parses),* }),
            ),
        };

        descriptors.push(quote!(#rake::router::RouteDescriptor { id: #id, path: #path, params: &[#(#parameters),*] }));
        encoders.push(quote!(#pattern => { let mut params = ::std::collections::BTreeMap::new(); #(#inserts)* #rake::router::RouteAddress { route: #id.into(), params } }));
        decoders.push(quote!(#id => { let descriptor = Self::descriptors().iter().find(|descriptor| descriptor.id == #id).expect("generated route descriptor"); address.validate(descriptor)?; Ok(#constructor) }));
    }

    Ok(quote! {
        impl #rake::router::Route for #name {
            fn encode(&self) -> #rake::router::RouteAddress { match self { #(#encoders),* } }
            fn decode(address: &#rake::router::RouteAddress) -> ::std::result::Result<Self, #rake::router::RouteDecodeError> {
                match address.route.as_str() { #(#decoders),*, _ => Err(#rake::router::RouteDecodeError::UnknownRoute(address.route.clone())) }
            }
            fn descriptors() -> &'static [#rake::router::RouteDescriptor] { &[#(#descriptors),*] }
        }
    })
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };

    let segment = path.path.segments.last()?;

    if segment.ident != "Option" {
        return None;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
        return None;
    };

    Some(inner)
}
