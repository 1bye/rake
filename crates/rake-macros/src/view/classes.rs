//! Checked GPUI utilities shared by literal and conditional class expansion.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Expr, Lit};

pub(super) const FIXED: &[(&str, &str)] = &[
    ("flex", "flex"),
    ("block", "block"),
    ("grid", "grid"),
    ("hidden", "hidden"),
    ("flex-col", "flex_col"),
    ("flex-row", "flex_row"),
    ("flex-wrap", "flex_wrap"),
    ("flex-1", "flex_1"),
    ("items-start", "items_start"),
    ("items-center", "items_center"),
    ("items-end", "items_end"),
    ("justify-start", "justify_start"),
    ("justify-center", "justify_center"),
    ("justify-end", "justify_end"),
    ("justify-between", "justify_between"),
    ("w-full", "w_full"),
    ("h-full", "h_full"),
    ("size-full", "size_full"),
    ("min-w-0", "min_w_0"),
    ("min-h-0", "min_h_0"),
    ("overflow-hidden", "overflow_hidden"),
    ("overflow-scroll", "overflow_scroll"),
    ("overflow-x-scroll", "overflow_x_scroll"),
    ("overflow-y-scroll", "overflow_y_scroll"),
    ("rounded-none", "rounded_none"),
    ("rounded-sm", "rounded_sm"),
    ("rounded-md", "rounded_md"),
    ("rounded-lg", "rounded_lg"),
    ("rounded-xl", "rounded_xl"),
    ("rounded-full", "rounded_full"),
    ("border-0", "border_0"),
    ("border-1", "border_1"),
    ("border-2", "border_2"),
    ("shadow-sm", "shadow_sm"),
    ("shadow-md", "shadow_md"),
    ("shadow-lg", "shadow_lg"),
    ("cursor-pointer", "cursor_pointer"),
    ("text-xs", "text_xs"),
    ("text-sm", "text_sm"),
    ("text-base", "text_base"),
    ("text-lg", "text_lg"),
    ("text-xl", "text_xl"),
    ("text-2xl", "text_2xl"),
    ("text-3xl", "text_3xl"),
    ("text-left", "text_left"),
    ("text-center", "text_center"),
    ("text-right", "text_right"),
    ("rounded", "rounded_md"),
    ("border", "border_1"),
    ("shrink-0", "flex_shrink_0"),
    ("grow", "flex_grow_1"),
    ("grow-0", "flex_grow_0"),
    ("flex-none", "flex_none"),
    ("relative", "relative"),
    ("absolute", "absolute"),
    ("items-stretch", "items_stretch"),
    ("self-start", "self_start"),
    ("self-center", "self_center"),
    ("self-end", "self_end"),
    ("justify-around", "justify_around"),
    ("truncate", "truncate"),
    ("whitespace-nowrap", "whitespace_nowrap"),
    ("overflow-x-hidden", "overflow_x_hidden"),
    ("overflow-y-hidden", "overflow_y_hidden"),
    ("border-t", "border_t_1"),
    ("border-t-0", "border_t_0"),
    ("border-t-1", "border_t_1"),
    ("border-t-2", "border_t_2"),
    ("border-b", "border_b_1"),
    ("border-b-0", "border_b_0"),
    ("border-b-1", "border_b_1"),
    ("border-b-2", "border_b_2"),
    ("border-l", "border_l_1"),
    ("border-l-0", "border_l_0"),
    ("border-l-1", "border_l_1"),
    ("border-l-2", "border_l_2"),
    ("border-r", "border_r_1"),
    ("border-r-0", "border_r_0"),
    ("border-r-1", "border_r_1"),
    ("border-r-2", "border_r_2"),
];

pub(super) const SCALES: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32];

pub(super) const PREFIXES: &[&str] = &[
    "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "ml", "mr", "gap",
    "gap-x", "gap-y", "w", "h", "size",
];

pub(super) fn utility_methods(
    class: &str,
    span: Span,
    rake: &TokenStream,
) -> syn::Result<TokenStream> {
    if let Some(weight) = match class {
        "font-normal" => Some("NORMAL"),
        "font-medium" => Some("MEDIUM"),
        "font-semibold" => Some("SEMIBOLD"),
        "font-bold" => Some("BOLD"),
        _ => None,
    } {
        let weight = format_ident!("{weight}");

        return Ok(quote!(.font_weight(#rake::gpui::FontWeight::#weight)));
    }

    if let Some(columns) = class
        .strip_prefix("grid-cols-")
        .and_then(|value| value.parse::<u16>().ok())
        && (1..=12).contains(&columns)
    {
        return Ok(quote!(.grid_cols(#columns)));
    }

    let method = if let Some((_, method)) = FIXED.iter().find(|(utility, _)| *utility == class) {
        (*method).to_owned()
    } else if class.rsplit_once('-').is_some_and(|(prefix, value_text)| {
        PREFIXES.contains(&prefix)
            && value_text
                .parse::<usize>()
                .is_ok_and(|value| SCALES.contains(&value) && value.to_string() == value_text)
    }) {
        class.replace('-', "_")
    } else {
        return Err(syn::Error::new(
            span,
            format!("unsupported utility class `{class}`; use style={{|s| ...}} for GPUI styling"),
        ));
    };

    let method = format_ident!("{method}", span = span);

    Ok(quote!(.#method()))
}

pub(super) fn class_methods(value: &Expr, rake: &TokenStream) -> syn::Result<TokenStream> {
    let Expr::Lit(syn::ExprLit {
        lit: Lit::Str(classes),
        ..
    }) = value
    else {
        return Err(syn::Error::new_spanned(
            value,
            "class requires a literal utility string; use style={|s| ...} for dynamic styles",
        ));
    };

    let mut methods = TokenStream::new();
    let mut errors: Option<syn::Error> = None;

    for class in classes.value().split_whitespace() {
        match utility_methods(class, classes.span(), rake) {
            Ok(tokens) => methods.extend(tokens),
            Err(error) => {
                if let Some(errors) = &mut errors {
                    errors.combine(error);
                } else {
                    errors = Some(error);
                }
            }
        }
    }

    if let Some(errors) = errors {
        return Err(errors);
    }

    Ok(methods)
}
