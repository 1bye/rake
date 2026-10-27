//! Canonical built-in events and source-spanned legacy alias warnings.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

use super::Attribute;

pub(super) const EVENTS: &[&str] = &[
    "click",
    "key-down",
    "key-up",
    "mouse-move",
    "mouse-down",
    "mouse-up",
    "right-mouse-down",
    "right-mouse-up",
    "middle-mouse-down",
    "middle-mouse-up",
    "any-mouse-down",
    "mouse-down-out",
    "mouse-up-out",
    "hover",
    "scroll-wheel",
    "drop",
    "drag",
];

pub(super) const LEGACY: &[(&str, &str)] = &[
    ("onclick", "click"),
    ("onkeydown", "key-down"),
    ("onkeyup", "key-up"),
    ("onmousemove", "mouse-move"),
    ("onmousedown", "mouse-down"),
    ("onmouseup", "mouse-up"),
];

pub(super) fn is_event(name: &str) -> bool {
    name.starts_with("on:") || LEGACY.iter().any(|(legacy, _)| *legacy == name)
}

pub(super) fn expand(
    attribute: &Attribute,
    rake: &TokenStream,
    index: usize,
) -> syn::Result<TokenStream> {
    let attribute_value = &attribute.value;
    let mut value = quote!(#attribute_value);

    let name = if let Some((_, canonical)) =
        LEGACY.iter().find(|(legacy, _)| *legacy == attribute.name)
    {
        let legacy = format_ident!("{}", attribute.name.to_uppercase(), span = attribute.span);

        value = quote_spanned!(attribute.span=> { let _ = #rake::__private::view_legacy::#legacy; #value });

        *canonical
    } else {
        attribute.name.strip_prefix("on:").expect("event namespace")
    };

    if name == "mouse-down-out" {
        return Ok(quote!(.on_mouse_down_out(#value)));
    }

    let mouse = match name {
        "mouse-down" | "mouse-up" | "mouse-up-out" => Some("Left"),
        "right-mouse-down" | "right-mouse-up" => Some("Right"),
        "middle-mouse-down" | "middle-mouse-up" => Some("Middle"),
        _ => None,
    };

    if let Some(button) = mouse {
        let button = format_ident!("{button}");

        let method = if name.ends_with("down-out") {
            "on_mouse_down_out"
        } else if name.ends_with("up-out") {
            "on_mouse_up_out"
        } else if name.ends_with("down") {
            "on_mouse_down"
        } else {
            "on_mouse_up"
        };

        let method = format_ident!("{method}");

        return Ok(quote!(.#method(#rake::gpui::MouseButton::#button, #value)));
    }

    if name == "drag" {
        let payload = format_ident!("__rake_drag_payload_{index}", span = Span::mixed_site());

        let constructor =
            format_ident!("__rake_drag_constructor_{index}", span = Span::mixed_site());

        let builder = format_ident!("__rake_drag_builder_{index}", span = Span::mixed_site());

        return Ok(quote!(.map(|#builder| {
            let (#payload, #constructor) = #value;
            #builder.on_drag(#payload, #constructor)
        })));
    }

    if EVENTS.contains(&name) {
        let method = format_ident!("on_{}", name.replace('-', "_"));

        return Ok(quote!(.#method(#value)));
    }

    Err(syn::Error::new(
        attribute.span,
        format!(
            "unknown built-in event `{name}`; supported events: {}",
            EVENTS.join(", ")
        ),
    ))
}
