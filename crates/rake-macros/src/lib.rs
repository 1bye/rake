//! Procedural macros that resolve all runtime paths through the Rake facade.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{DeriveInput, ItemFn, Token, parenthesized, parse_macro_input};

mod route;

/// Derives a closed destination codec from explicit `route(id, path)` attributes.
#[proc_macro_derive(Route, attributes(route))]
pub fn route(input: TokenStream) -> TokenStream {
    route::derive(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

fn facade() -> syn::Result<Tokens> {
    match crate_name("rake") {
        Ok(FoundCrate::Itself) => Ok(quote!(::rake)),
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");

            Ok(quote!(::#name))
        }
        Err(error) => Err(syn::Error::new(proc_macro2::Span::call_site(), error)),
    }
}

/// Derives Serde serialization and deserialization through the resolved facade.
#[proc_macro_attribute]
pub fn data(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(proc_macro2::Span::call_site(), "data takes no arguments")
            .to_compile_error()
            .into();
    }

    let item = parse_macro_input!(input as DeriveInput);

    let rake = match facade() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let serde_path = format!("{}::serde", rake.to_string().replace(' ', ""));

    quote! {
        #[derive(#rake::serde::Serialize, #rake::serde::Deserialize)]
        #[serde(crate = #serde_path)]
        #item
    }
    .into()
}

#[derive(Default)]
struct InstrumentArgs {
    fields: Tokens,
}

impl Parse for InstrumentArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let option: syn::Ident = input.parse()?;

            match option.to_string().as_str() {
                "skip_all" => {}
                "fields" => {
                    let content;

                    parenthesized!(content in input);
                    args.fields = content.parse()?;
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        option,
                        "supported: skip_all, fields(...)",
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(args)
    }
}

/// Instruments sync or async functions, skipping arguments and recording explicit fields.
///
/// Supports `skip_all` and `fields(name = %value, other = ?value)`. An async
/// span is entered only while its future is polled, never across suspension.
#[proc_macro_attribute]
pub fn instrument(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as InstrumentArgs);
    let mut item = parse_macro_input!(input as ItemFn);

    let rake = match facade() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let name = item.sig.ident.to_string();
    let fields = args.fields;
    let body = item.block;

    if item.sig.constness.is_some() {
        return syn::Error::new_spanned(item.sig, "instrument does not support const functions")
            .to_compile_error()
            .into();
    }

    item.block = if item.sig.asyncness.is_some() {
        syn::parse_quote!({
            let __rake_span = #rake::__private::tracing::info_span!(#name, #fields);

            #rake::__private::tracing::Instrument::instrument(async move #body, __rake_span).await
        })
    } else {
        syn::parse_quote!({
            let __rake_span = #rake::__private::tracing::info_span!(#name, #fields);

            __rake_span.in_scope(|| #body)
        })
    };

    quote!(#item).into()
}

/// Generates a native entry point and browser Wasm start export from one factory.
#[proc_macro_attribute]
pub fn app(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemFn);

    if !args.is_empty()
        || !item.sig.inputs.is_empty()
        || item.sig.asyncness.is_some()
        || !item.sig.generics.params.is_empty()
        || item.sig.ident == "main"
    {
        return syn::Error::new_spanned(
            item.sig,
            "app requires a synchronous, non-generic, zero-argument factory named other than main",
        )
        .to_compile_error()
        .into();
    }

    let rake = match facade() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let factory = &item.sig.ident;

    quote! {
        #item

        #[cfg(not(target_family = "wasm"))]
        /// Runs the application on the native host.
        pub fn main() {
            if let Err(error) = #factory().run() {
                eprintln!("Rake startup failed: {error}");
                ::std::process::exit(1);
            }
        }

        #[cfg(target_family = "wasm")]
        /// Starts the application from the generated Wasm loader.
        #[#rake::__private::wasm_bindgen::prelude::wasm_bindgen(
            start, wasm_bindgen = #rake::__private::wasm_bindgen
        )]
        pub fn rake_start() -> ::std::result::Result<(), #rake::__private::wasm_bindgen::JsValue> {
            #factory().run().map_err(|error| {
                #rake::__private::wasm_bindgen::JsValue::from_str(&error.to_string())
            })
        }
    }
    .into()
}
