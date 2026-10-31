//! Theme-name classification, declaration validation and invocation discovery.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::{DeriveInput, Expr, Lit};

use super::classes::{FIXED, LENGTH_PREFIXES};
use super::{AttributeValue, Element, Node};

const PREFIXES: &[(&str, &str)] = &[
    ("text-", "ThemeText"),
    ("bg-", "ThemeBackground"),
    ("border-", "ThemeBorder"),
    ("font-", "ThemeFont"),
    ("rounded-", "ThemeRadius"),
];

const WEIGHTS: &[&str] = &["normal", "medium", "semibold", "bold"];

pub(super) fn local() -> Ident {
    format_ident!("__rake_theme", span = Span::mixed_site())
}

pub(super) fn field(class: &str) -> Option<(&'static str, String)> {
    if FIXED.iter().any(|(name, _)| *name == class)
        || LENGTH_PREFIXES.iter().any(|(prefix, _)| {
            prefix.starts_with("border-")
                && class
                    .strip_prefix(&format!("{prefix}-"))
                    .is_some_and(|value| value.starts_with(|c: char| c.is_ascii_digit()))
        })
    {
        return None;
    }

    let (prefix, kind, value) = PREFIXES.iter().find_map(|(prefix, kind)| {
        class
            .strip_prefix(prefix)
            .map(|value| (*prefix, *kind, value))
    })?;

    if value.is_empty()
        || value.starts_with('[')
        || value.starts_with(|c: char| c.is_ascii_digit() || c == '-')
        || (prefix == "font-" && WEIGHTS.contains(&value))
    {
        return None;
    }

    let field = value.replace('-', "_");

    syn::parse_str::<Ident>(&field).ok()?;

    Some((kind, field))
}

pub(super) fn methods(class: &str, span: Span, rake: &TokenStream) -> Option<TokenStream> {
    let (kind, field) = field(class)?;
    let kind = format_ident!("{kind}", span = span);
    let field = format_ident!("{field}", span = span);
    let theme = local();

    Some(quote_spanned!(span=> .map(|s| #rake::__private::#kind::apply(#theme.#field.clone(), s))))
}

pub(super) fn binding(span: Span, rake: &TokenStream) -> TokenStream {
    let theme = local();
    let declaration = quote_spanned!(span=> crate::__RakeTheme);

    quote!(let #theme = #rake::theme::current::<#declaration>();)
}

fn literal_use(value: &str, span: Span) -> Option<Span> {
    value.split_whitespace().find_map(|class| {
        let utility = super::classes::split_variant(class).map_or(class, |(_, utility)| utility);

        field(utility).map(|_| span)
    })
}

pub(super) fn expression_use(value: &Expr) -> Option<Span> {
    match value {
        Expr::Lit(syn::ExprLit {
            lit: Lit::Str(value),
            ..
        }) => literal_use(&value.value(), value.span()),
        Expr::Paren(value) => expression_use(&value.expr),
        Expr::Group(value) => expression_use(&value.expr),
        Expr::Block(value) => block_use(&value.block),
        Expr::If(value) => block_use(&value.then_branch).or_else(|| {
            value
                .else_branch
                .as_ref()
                .and_then(|(_, value)| expression_use(value))
        }),
        Expr::Match(value) => value.arms.iter().find_map(|arm| expression_use(&arm.body)),
        _ => None,
    }
}

fn block_use(block: &syn::Block) -> Option<Span> {
    if let [syn::Stmt::Expr(value, None)] = block.stmts.as_slice() {
        expression_use(value)
    } else {
        None
    }
}

pub(super) fn used(element: &Element) -> Option<Span> {
    element
        .attributes
        .iter()
        .find_map(|attribute| match &attribute.value {
            AttributeValue::Markup(element) => used(element),
            AttributeValue::Expression(value) if attribute.name == "class" => expression_use(value),
            _ => attribute
                .name
                .strip_prefix("class:")
                .and_then(|class| literal_use(class, attribute.span)),
        })
        .or_else(|| nodes_use(&element.children))
}

fn nodes_use(nodes: &[Node]) -> Option<Span> {
    nodes.iter().find_map(|node| match node {
        Node::Element(element) => used(element),
        Node::If {
            then_nodes,
            else_nodes,
            ..
        } => nodes_use(then_nodes).or_else(|| nodes_use(else_nodes)),
        Node::For { children, .. } => nodes_use(children),
        Node::Match { arms, .. } => arms.iter().find_map(|arm| nodes_use(&arm.children)),
        _ => None,
    })
}

pub(crate) fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Theme requires a struct with named fields",
        ));
    };

    let syn::Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Theme requires named fields",
        ));
    };

    let mut errors = super::Diagnostics::default();

    for field in &fields.named {
        let name = field.ident.as_ref().expect("named field");

        if reserved(&name.to_string()) {
            errors.push(syn::Error::new_spanned(
                name,
                "theme field collides with a built-in utility value; choose a descriptive name",
            ));
        }
    }

    if let Some(error) = errors.0 {
        return Err(error);
    }

    let name = &input.ident;
    let (generics, arguments, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            #[allow(dead_code)]
            fn assert_theme_clone #generics () #where_clause {
                fn assert_clone<T: Clone>() {}
                assert_clone::<#name #arguments>();
            }
        };
    })
}

fn reserved(name: &str) -> bool {
    WEIGHTS.contains(&name)
        || LENGTH_PREFIXES
            .iter()
            .filter_map(|(prefix, _)| prefix.strip_prefix("border-"))
            .any(|side| {
                name.strip_prefix(&format!("{side}_"))
                    .is_some_and(|value| value.parse::<usize>().is_ok())
            })
        || FIXED.iter().any(|(class, _)| {
            PREFIXES.iter().any(|(prefix, _)| {
                class
                    .strip_prefix(prefix)
                    .is_some_and(|value| value.replace('-', "_") == name)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_names_follow_the_checked_class_table() {
        for (class, _) in FIXED {
            for (prefix, _) in PREFIXES {
                if let Some(value) = class.strip_prefix(prefix) {
                    assert!(reserved(&value.replace('-', "_")));
                    assert!(field(class).is_none());
                }
            }
        }

        assert!(reserved("semibold"));
        assert!(reserved("t_13"));
        assert!(reserved("r_999"));
        assert!(reserved("l_01"));
        assert!(!reserved("t_custom"));
        assert!(!reserved("hover"));
        assert_eq!(field("bg-surface-raised").unwrap().1, "surface_raised");
        assert!(field("text-[12px]").is_none());
        assert!(field("border-t-4").is_none());
        assert_eq!(field("border-t-custom").unwrap().1, "t_custom");
    }
}
