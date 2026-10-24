//! Closed command metadata; argument/result codecs and schemas come from data types.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Ident, LitStr, Type};

pub(super) fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let rake = super::facade()?;

    expand(input, &rake)
}

fn expand(input: DeriveInput, rake: &TokenStream) -> syn::Result<TokenStream> {
    let name = &input.ident;

    if !matches!(input.data, Data::Struct(_))
        || !input.generics.params.is_empty()
        || input.generics.where_clause.is_some()
    {
        return Err(syn::Error::new_spanned(
            name,
            "Command requires a concrete, non-generic struct",
        ));
    }

    if let Data::Struct(data) = &input.data {
        for field in &data.fields {
            if let Some(attribute) = field
                .attrs
                .iter()
                .find(|attribute| attribute.path().is_ident("command"))
            {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "command metadata belongs on the struct",
                ));
            }
        }
    }

    let mut id = None;
    let mut output = None;
    let mut capability = None;

    for attribute in input
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("command"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") && id.is_none() {
                id = Some(meta.value()?.parse::<LitStr>()?);
            } else if meta.path.is_ident("output") && output.is_none() {
                output = Some(meta.value()?.parse::<Type>()?);
            } else if meta.path.is_ident("capability") && capability.is_none() {
                let value = meta.value()?.parse::<Ident>()?;

                if value != "Headless" && value != "RequiresWindow" {
                    return Err(syn::Error::new_spanned(
                        value,
                        "expected Headless or RequiresWindow",
                    ));
                }

                capability = Some(value);
            } else {
                return Err(meta
                    .error("expected one id, output and capability; unknown or duplicate option"));
            }

            Ok(())
        })?;
    }

    let id =
        id.ok_or_else(|| syn::Error::new_spanned(name, "missing #[command(id = \"stable.key\")]"))?;

    let value = id.value();

    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_whitespace) {
        return Err(syn::Error::new_spanned(
            id,
            "command id must be 1–128 bytes without whitespace",
        ));
    }

    let output = output.unwrap_or_else(|| syn::parse_quote!(()));
    // A command is only headless-capable when the author explicitly says so.
    let capability = capability.unwrap_or_else(|| syn::parse_quote!(RequiresWindow));

    Ok(quote! {
        impl #rake::control::ApplicationCommand for #name {
            type Output = #output;
            fn id() -> ::std::string::String { #id.into() }
            fn descriptor() -> #rake::control::CommandDescriptor {
                #rake::control::CommandDescriptor::for_type::<Self>(#id, #rake::control::ExecutionCapability::#capability)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::expand;
    use quote::quote;

    #[test]
    fn rejects_ambiguous_metadata_and_open_schemas() {
        for (input, expected) in [
            ("struct Missing {}", "missing"),
            (r#"#[command(id = "")] struct Empty;"#, "1–128"),
            (
                r#"#[command(id = "two words")] struct Space;"#,
                "whitespace",
            ),
            (
                r#"#[command(id = "x", id = "y")] struct Duplicate;"#,
                "duplicate",
            ),
            (
                r#"#[command(id = "x", capability = Desktop)] struct Platform;"#,
                "Headless or RequiresWindow",
            ),
            (
                r#"#[command(id = "x", outpt = String)] struct Typo;"#,
                "unknown",
            ),
            (
                r#"#[command(id = "x", output = (), output = ())] struct Repeated;"#,
                "duplicate",
            ),
            (
                r#"#[command(id = "x")] enum Enum { A }"#,
                "non-generic struct",
            ),
            (
                r#"#[command(id = "x")] struct Generic<T>(T);"#,
                "non-generic struct",
            ),
        ] {
            let error = expand(syn::parse_str(input).unwrap(), &quote!(::framework)).unwrap_err();

            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}
