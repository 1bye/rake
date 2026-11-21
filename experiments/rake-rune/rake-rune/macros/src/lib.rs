//! Experimental Rune lowering of Rake's renderer-independent parse tree.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use rake_view_syntax::{AttributeValue, Element, Node, View};
use syn::{Expr, Lit};

#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    syn::parse::<View>(input)
        .and_then(|view| {
            view.layout()?;
            element(view.root())
        })
        .unwrap_or_else(|error| error.into_compile_error())
        .into()
}

fn element(element: &Element) -> syn::Result<Tokens> {
    element_colored(element, &quote!(theme.text))
}

fn element_colored(element: &Element, inherited_color: &Tokens) -> syn::Result<Tokens> {
    let tag = element.tag.segments.last().unwrap().ident.to_string();
    if tag.starts_with(char::is_uppercase) {
        let path = &element.tag;
        let mut fields = Vec::new();
        let mut has_children = false;
        for attribute in &element.attributes {
            let name = syn::parse_str::<syn::Ident>(&attribute.name)?;
            has_children |= attribute.name == "children";
            let value = match &attribute.value {
                AttributeValue::Expression(value) => quote!(#value),
                AttributeValue::Markup(value) => self::element(value)?,
            };
            fields.push(quote!(#name: #value,));
        }
        if !element.children.is_empty() {
            if has_children {
                return Err(syn::Error::new_spanned(path, "duplicate children slot"));
            }
            let children = nodes(&element.children, inherited_color)?;
            fields
                .push(quote!(children: { let mut __children = Vec::new(); #children __children },));
        }
        return Ok(quote!(::rake_rune::Component::render(#path { #(#fields)* })));
    }
    let mut classes = String::new();
    for attribute in &element.attributes {
        if attribute.name == "class" {
            match &attribute.value {
                AttributeValue::Expression(value) => match value.as_ref() {
                    Expr::Lit(lit) => match &lit.lit {
                        Lit::Str(value) => classes = value.value(),
                        _ => {
                            return Err(syn::Error::new(
                                attribute.span,
                                "class must be a string literal",
                            ));
                        }
                    },
                    _ => {
                        return Err(syn::Error::new(
                            attribute.span,
                            "dynamic classes are not supported",
                        ));
                    }
                },
                _ => {
                    return Err(syn::Error::new(
                        attribute.span,
                        "class must be a string literal",
                    ));
                }
            }
        }
    }
    let mut output = match tag.as_str() {
        "p" | "span" | "h1" | "h2" => quote!(::rake_rune::rune::col()),
        "div" | "main" | "section" | "nav" | "button" => {
            if classes.split_whitespace().any(|class| class == "flex-row") {
                quote!(::rake_rune::rune::row())
            } else if classes
                .split_whitespace()
                .any(|class| class == "flex-col" || class == "flex")
            {
                quote!(::rake_rune::rune::col())
            } else {
                quote!(::rake_rune::rune::el())
            }
        }
        _ => {
            return Err(syn::Error::new_spanned(
                &element.tag,
                "unsupported Rune element",
            ));
        }
    };
    if tag == "button" {
        output = quote!(#output.role("button").focusable(true));
    }
    for class in classes.split_whitespace() {
        let method = utility(class)?;
        output = quote!(#output #method);
    }
    for attribute in &element.attributes {
        if attribute.name == "class" {
            continue;
        }
        let AttributeValue::Expression(value) = &attribute.value else {
            return Err(syn::Error::new(
                attribute.span,
                "markup is only supported on components",
            ));
        };
        let method = match attribute.name.as_str() {
            "id" => quote!(.key(#value)),
            "aria-label" => quote!(.label(#value)),
            "role" => quote!(.role(#value)),
            "on:click" => quote!(.on_press(#value)),
            _ => {
                return Err(syn::Error::new(
                    attribute.span,
                    "unsupported Rune attribute",
                ));
            }
        };
        output = quote!(#output #method);
    }
    let mut color = inherited_color.clone();
    for class in classes.split_whitespace() {
        if let Some(name) = class.strip_prefix("text-") {
            let field = match name {
                "foreground" => "text",
                "muted-foreground" => "text_secondary",
                "background" => "bg",
                "surface" => "card",
                "muted" => "control_track",
                "accent" => "hover",
                _ => unreachable!("utility validation rejects unknown colors"),
            };
            let field = format_ident!("{field}");
            color = quote!(theme.#field);
        }
    }
    let children = nodes(&element.children, &color)?;
    Ok(quote!({ let mut __children = Vec::new(); #children (#output).children(__children) }))
}

fn nodes(nodes: &[Node], color: &Tokens) -> syn::Result<Tokens> {
    let mut output = Tokens::new();
    for node in nodes {
        output.extend(match node {
            Node::Element(value) => {
                let value = element_colored(value, color)?;
                quote!(__children.push(#value);)
            }
            Node::Text(value) => quote!(__children.push(::rake_rune::rune::text(#value).font(::rake_rune::rune::tok::type_::BODY).color(#color));),
            Node::Expression(value) => {
                quote!(__children.push(::rake_rune::IntoEl::into_el(#value).color(#color));)
            }
            Node::For {
                pattern,
                iterator,
                children,
            } => {
                let body = self::nodes(children, color)?;
                quote!(for #pattern in #iterator { #body })
            }
            Node::If {
                condition,
                then_nodes,
                else_nodes,
            } => {
                let yes = self::nodes(then_nodes, color)?;
                let no = self::nodes(else_nodes, color)?;
                quote!(if #condition { #yes } else { #no })
            }
            Node::Match { expression, arms } => {
                let arms = arms
                    .iter()
                    .map(|arm| {
                        let pattern = &arm.pattern;
                        let guard = arm.guard.as_ref().map(|guard| quote!(if #guard));
                        let body = self::nodes(&arm.children, color)?;
                        Ok(quote!(#pattern #guard => { #body }))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                quote!(match #expression { #(#arms,)* })
            }
        });
    }
    Ok(output)
}

fn utility(class: &str) -> syn::Result<Tokens> {
    let simple = match class {
        "flex" | "flex-row" | "flex-col" => Some(quote!()),
        "w-full" => Some(quote!(.w_full())),
        "h-full" => Some(quote!(.h_full())),
        "size-full" => Some(quote!(.w_full().h_full())),
        "flex-1" => Some(quote!(.grow(1.0).min_w(0.0))),
        "items-center" => Some(quote!(.center_y())),
        "justify-between" => Some(quote!(.justify_between())),
        "rounded" | "rounded-md" => Some(quote!(.radius(::rake_rune::rune::tok::radius::SM))),
        "rounded-lg" => Some(quote!(.radius(::rake_rune::rune::tok::radius::MD))),
        _ => None,
    };
    if let Some(value) = simple {
        return Ok(value);
    }
    for (prefix, method) in [
        ("hover:bg-", "hover"),
        ("focus:bg-", "focus_fill"),
        ("focus-visible:bg-", "focus_fill"),
        ("bg-", "fill"),
        ("text-", "color"),
    ] {
        if let Some(color) = class.strip_prefix(prefix) {
            let field = match color {
                "background" => "bg",
                "surface" => "card",
                "foreground" => "text",
                "muted-foreground" => "text_secondary",
                "muted" => "control_track",
                "accent" => "hover",
                _ => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("unsupported Rune class: {class}"),
                    ));
                }
            };
            let field = format_ident!("{field}");
            let method = format_ident!("{method}");
            return Ok(quote!(.#method(theme.#field)));
        }
    }
    if let Some((prefix, number)) = class.split_once('-') {
        if let Ok(number) = number.parse::<f32>() {
            if number.is_finite() && number >= 0.0 {
                let value = number * 4.0;
                if prefix == "size" {
                    return Ok(quote!(.size(#value, #value)));
                }
                let method = match prefix {
                    "w" => Some("w"),
                    "h" => Some("h"),
                    "p" => Some("padding"),
                    "px" => Some("px"),
                    "gap" => Some("gap"),
                    _ => None,
                };
                if let Some(method) = method {
                    let method = format_ident!("{method}");
                    return Ok(quote!(.#method(#value)));
                }
            }
        }
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        format!("unsupported Rune class: {class}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_utilities_and_control_flow() {
        for invalid in ["unknown", "hover:p-4", "bg-red-500", "w-NaN"] {
            assert!(utility(invalid).is_err());
        }
        let view: View = syn::parse_str("<div class=\"flex flex-col\">for item in items { if item { <span>\"yes\"</span> } else { <span/> } } match mode { true => { <div/> } false => { <div/> } }</div>").unwrap();
        assert!(
            element(view.root())
                .unwrap()
                .to_string()
                .contains("match mode")
        );
        let empty: View = syn::parse_str("<div/>").unwrap();
        assert!(element(empty.root()).is_ok());
    }
}
