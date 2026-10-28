//! Procedural macros that resolve all runtime paths through the Rake facade.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{DeriveInput, ItemFn, Token, parenthesized, parse_macro_input};

mod command;
mod route;
mod view;

/// Builds a GPUI element tree from experimental HTML-like markup.
///
/// Exposed as `rake::view!` with Rake's `experimental-view` feature. Supports
/// unquoted/quoted text, `{expression}` children, `if`/`else`, `for`, and guarded `match`
/// children with braced arm bodies. Untaken branches are not evaluated.
/// `class` accepts checked GPUI utilities; `class:utility={bool}` applies one
/// conditionally in attribute order (layout tags only); `style={|s| ...}` applies
/// GPUI builder methods. Canonical events use `on:click`, `on:key-down`, mouse-button,
/// Utilities include half-step spacing, unit lengths, fractions, literal hex colors,
/// font families and opacity; negative lengths are limited to margins and inset.
/// Interaction variants merge into one GPUI call per kind: hover, active, focus,
/// focus-visible, in-focus and named group-hover/group-active. disabled is button-only.
/// Variants cannot stack or configure interactivity. Conflicting calls inside style
/// closures are detected heuristically; indirect calls remain GPUI's responsibility.
/// Utility strings are checked and lowered to ordered GPUI style calls at compile time.
/// outside-pointer, hover, wheel, drop and drag names with GPUI callbacks.
/// `tooltip` accepts a GPUI tooltip builder even on disabled buttons. Legacy
/// event aliases remain accepted with deprecation warnings. Drag takes a
/// `(payload, constructor)` tuple; drop callbacks need a typed first parameter.
/// Repeated element roots require explicit, stable `id` attributes.
/// `<view entity={&self.editor}/>` mounts a retained entity by borrowing its handle.
/// `focus` and `scroll` borrow stored GPUI handles; inline constructors are rejected.
/// `on:Action` registers a typed GPUI action handler; `key-context` scopes keymaps.
/// `tab-navigation={true}` enables unconsumed Tab/Shift+Tab navigation.
/// Buttons support `disabled`, retained focus, and keyboard activation; style their
/// focus-visible and disabled appearance with GPUI builders.
/// Uppercase component tags (`<ui::Card ...>...</ui::Card>`) construct typed Rust
/// structs; paired tags collect `children: Vec<gpui::AnyElement>`. Bare boolean
/// properties mean true; `{id}` is shorthand for `id={id}`. A final `{..expr}`
/// supplies component defaults through Rust struct update, after explicit fields
/// and paired children. Built-in bare booleans are limited to disabled, wrap,
/// ellipsis, tab-navigation and aria-expanded. Slots are ordinary
/// properties. Components own their root IDs and render through GPUI's `RenderOnce`.
/// Paragraphs and h1–h6 headings compose string-like values and inline
/// `span`/`strong`/`em`/`br` into one styled text layout, with `wrap`, `ellipsis`,
/// and `max-lines` controls.
/// Both reject layout/widget children; use a div with a heading role for rich content.
/// Headings expose the full composed text as their name; explicit aria-label wins.
/// Inline `style` takes `gpui::HighlightStyle`.
/// Semantic nav/header/footer/main/aside/article tags set AccessKit roles; hr is
/// a one-point rule without a separator role in the pinned AccessKit version.
/// SVG leaves use path, styling and optional image labels; they have no macro ID
/// and are exempt from loop IDs. Wrap SVGs for identity, focus and events.
/// Independent local diagnostics are combined. Attribute parsing recovers at the
/// next attribute/tag boundary, and valid builder contexts remain type-checked
/// beside macro errors. Malformed markup structure can still stop parsing.
/// Debug builds diagnose duplicate direct sibling IDs visible at construction.
/// Rustfmt does not format view! bodies; put longer attribute lists on separate
/// lines and extract long style/listener expressions into named bindings.
/// A dedicated macro formatter remains a follow-up.
/// See `docs/experimental-view.md` in the repository for the full contract.
///
#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as view::View);
    let expanded = facade().and_then(|rake| input.expand_resilient(&rake));

    expanded
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

/// Derives a closed destination codec from explicit `route(id, path)` attributes.
#[proc_macro_derive(Route, attributes(route))]
pub fn route(input: TokenStream) -> TokenStream {
    route::derive(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

/// Derives application command metadata from `#[command(id, output, capability)]`.
/// Pair with `#[rake::data(schema)]` for argument codecs and JSON schemas.
/// Output defaults to `()`, capability to `RequiresWindow`.
#[proc_macro_derive(Command, attributes(command))]
pub fn command(input: TokenStream) -> TokenStream {
    command::derive(parse_macro_input!(input as DeriveInput))
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
/// `#[rake::data(schema)]` also derives JSON Schema, respecting Serde annotations.
#[proc_macro_attribute]
pub fn data(args: TokenStream, input: TokenStream) -> TokenStream {
    let schema = args.to_string() == "schema";

    if !args.is_empty() && !schema {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "data accepts only the optional schema flag",
        )
        .to_compile_error()
        .into();
    }

    let item = parse_macro_input!(input as DeriveInput);

    let rake = match facade() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let serde_path = format!("{}::serde", rake.to_string().replace(' ', ""));

    let schema_path = format!("{}::schemars", rake.to_string().replace(' ', ""));

    let schema = schema.then(|| {
        quote! {
            #[derive(#rake::schemars::JsonSchema)]
            #[schemars(crate = #schema_path)]
        }
    });

    quote! {
        #schema
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
