//! Named style functions use the same checked class plan as view attributes.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Expr, ForeignItemFn, LitStr, ReturnType};

use super::{classes, style_plan::Variant, theme};

pub(crate) fn expand(
    classes: LitStr,
    item: ForeignItemFn,
    rake: &TokenStream,
) -> syn::Result<TokenStream> {
    let signature = &item.sig;

    if !signature.inputs.is_empty()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
        || signature.asyncness.is_some()
        || signature.constness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
        || !matches!(signature.output, ReturnType::Default)
    {
        return Err(syn::Error::new_spanned(
            signature,
            "style preset marker must be `fn name();` without parameters, generics or qualifiers",
        ));
    }

    if classes.value().contains("class:") || classes.value().contains('{') {
        return Err(syn::Error::new_spanned(
            classes,
            "style presets accept one literal utility string; conditional attributes and branches require view!",
        ));
    }

    let value: Expr = syn::parse_quote!(#classes);
    let plan = classes::class_methods(&value, rake, None)?;

    let stateful = plan
        .variants
        .keys()
        .any(|variant| matches!(variant, Variant::Active | Variant::GroupActive(_)))
        || classes.value().split_whitespace().any(|class| {
            matches!(
                class,
                "overflow-scroll" | "overflow-x-scroll" | "overflow-y-scroll"
            )
        });

    let bound = if stateful {
        quote!(+ #rake::gpui::StatefulInteractiveElement)
    } else if plan.interactive {
        quote!(+ #rake::gpui::InteractiveElement)
    } else {
        TokenStream::new()
    };

    let theme = theme::expression_use(&value).map(|span| theme::binding(span, rake));
    let builder = format_ident!("__rake_style", span = Span::mixed_site());
    let body = plan.emit_refinement(&builder);
    let variants = plan.emit_variants(&builder);
    let name = &signature.ident;
    let visibility = &item.vis;
    let attributes = &item.attrs;

    Ok(quote! {
        #(#attributes)*
        #visibility fn #name<S: #rake::gpui::Styled #bound>(#builder: S) -> S {
            #theme
            #body
            #builder #variants
        }
    })
}
