//! Checked GPUI utilities shared by literal and conditional class expansion.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Expr, Lit};

use super::style_plan::{StylePlan, Variant};

pub(super) mod value;

const LENGTH_PREFIXES: &[(&str, &str)] = &[
    ("min-w", "min_w"),
    ("max-w", "max_w"),
    ("min-h", "min_h"),
    ("max-h", "max_h"),
    ("border-t", "border_t"),
    ("border-b", "border_b"),
    ("border-l", "border_l"),
    ("border-r", "border_r"),
    ("rounded", "rounded"),
    ("border", "border"),
    ("inset", "inset"),
    ("top", "top"),
    ("right", "right"),
    ("bottom", "bottom"),
    ("left", "left"),
    ("basis", "flex_basis"),
    ("leading", "line_height"),
    ("text", "text_size"),
];

fn length_tokens(length: value::Length, rake: &TokenStream) -> TokenStream {
    let (method, number) = match length {
        value::Length::Pixels(n) => ("px", n),
        value::Length::Rems(n) => ("rems", n),
        value::Length::Relative(n) => ("relative", n),
    };

    let method = format_ident!("{method}");
    let number = proc_macro2::Literal::f32_suffixed(number);

    quote!(#rake::gpui::#method(#number))
}

pub(super) fn literal_color(
    value: &str,
    span: Span,
    rake: &TokenStream,
) -> syn::Result<TokenStream> {
    let (value, alpha) = value::color(value).map_err(|message| syn::Error::new(span, message))?;

    Ok(if alpha {
        quote!(#rake::gpui::rgba(#value))
    } else {
        quote!(#rake::gpui::rgb(#value))
    })
}

fn extended_methods(
    class: &str,
    span: Span,
    rake: &TokenStream,
) -> syn::Result<Option<TokenStream>> {
    let error = |message| syn::Error::new(span, message);

    if let Some(raw) = class.strip_prefix("opacity-") {
        let n = value::opacity(raw).map_err(error)?;
        let n = proc_macro2::Literal::f32_suffixed(n);

        return Ok(Some(quote!(.opacity(#n))));
    }

    if class == "leading-none" {
        let n = length_tokens(value::Length::Relative(1.), rake);

        return Ok(Some(quote!(.line_height(#n))));
    }

    for (prefix, method) in [
        ("text", "text_color"),
        ("bg", "bg"),
        ("border", "border_color"),
    ] {
        if let Some(hex) = class
            .strip_prefix(&format!("{prefix}-["))
            .and_then(|v| v.strip_suffix(']'))
            .filter(|v| v.starts_with('#'))
        {
            let color = literal_color(hex, span, rake)?;
            let method = format_ident!("{method}");

            return Ok(Some(quote!(.#method(#color))));
        }
    }

    if let Some(font) = class
        .strip_prefix("font-[")
        .and_then(|v| v.strip_suffix(']'))
    {
        if font.is_empty() {
            return Err(error("font family cannot be empty"));
        }

        let font = font.replace('_', " ");

        return Ok(Some(quote!(.font_family(#font))));
    }

    let negative = class.starts_with('-');
    let class = class.strip_prefix('-').unwrap_or(class);

    let found = LENGTH_PREFIXES
        .iter()
        .find_map(|(prefix, method)| {
            class
                .strip_prefix(&format!("{prefix}-"))
                .map(|raw| (*prefix, (*method).to_owned(), raw))
        })
        .or_else(|| {
            PREFIXES.iter().rev().find_map(|prefix| {
                class
                    .strip_prefix(&format!("{prefix}-"))
                    .map(|raw| (*prefix, prefix.replace('-', "_"), raw))
            })
        });

    let Some((prefix, method, raw)) = found else {
        return Ok(None);
    };

    let arbitrary = raw.strip_prefix('[').and_then(|v| v.strip_suffix(']'));

    // Named values remain unknown until a theme is declared by the later grammar.
    if arbitrary.is_none() && !raw.starts_with(|c: char| c.is_ascii_digit()) && !negative {
        return Ok(None);
    }

    let permits_negative = matches!(
        prefix,
        "m" | "mx"
            | "my"
            | "mt"
            | "mb"
            | "ml"
            | "mr"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "inset"
    );

    if negative && !permits_negative {
        return Err(error(
            "negative values are supported only for margins and inset",
        ));
    }

    let mut length = if let Some(value) = arbitrary {
        value::length(value).map_err(error)?
    } else if raw.contains('/') {
        if !matches!(prefix, "w" | "h" | "basis") {
            return Err(error("fractions are supported only for w, h and basis"));
        }

        value::fraction(raw).map_err(error)?
    } else {
        value::scale(raw).map_err(error)?
    };

    let n = match &mut length {
        value::Length::Pixels(n) | value::Length::Rems(n) | value::Length::Relative(n) => n,
    };

    if *n < 0. && !permits_negative {
        return Err(error(
            "negative values are supported only for margins and inset",
        ));
    }

    if negative {
        if *n < 0. {
            return Err(error("use only one negative sign"));
        }

        *n = -*n;
    }

    if matches!(length, value::Length::Relative(_))
        && (prefix.starts_with("border") || matches!(prefix, "rounded" | "text"))
    {
        return Err(error("this utility requires px or rem, not %"));
    }

    let value = length_tokens(length, rake);
    let method = format_ident!("{method}", span = span);

    Ok(Some(quote!(.#method(#value))))
}

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
    if let Some(name) = class.strip_prefix("group/") {
        if name.is_empty() {
            return Err(syn::Error::new(span, "group utility requires a name"));
        }

        return Ok(quote!(.group(#name)));
    }

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
    } else if let Some(methods) = extended_methods(class, span, rake)? {
        return Ok(methods);
    } else {
        return Err(syn::Error::new(
            span,
            format!("unsupported utility class `{class}`; use style={{|s| ...}} for GPUI styling"),
        ));
    };

    let method = format_ident!("{method}", span = span);

    Ok(quote!(.#method()))
}

pub(super) fn class_methods(
    value: &Expr,
    rake: &TokenStream,
    disabled: Option<&proc_macro2::Ident>,
) -> syn::Result<StylePlan> {
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

    let mut plan = StylePlan::default();
    let mut errors = super::Diagnostics::default();

    for class in classes.value().split_whitespace() {
        let contribution = utility_plan(class, classes.span(), rake, disabled)
            .and_then(|contribution| plan.extend(contribution, classes.span()));

        if let Err(error) = contribution {
            errors.push(error);
        }
    }

    if let Some(errors) = errors.0 {
        return Err(errors);
    }

    Ok(plan)
}

fn split_variant(class: &str) -> Option<(&str, &str)> {
    let mut bracket = false;

    for (index, character) in class.char_indices() {
        match character {
            '[' => bracket = true,
            ']' => bracket = false,
            ':' if !bracket => return Some((&class[..index], &class[index + 1..])),
            _ => {}
        }
    }

    None
}

pub(super) fn utility_plan(
    class: &str,
    span: Span,
    rake: &TokenStream,
    disabled: Option<&proc_macro2::Ident>,
) -> syn::Result<StylePlan> {
    let Some((name, utility)) = split_variant(class) else {
        let mut plan = StylePlan::always(utility_methods(class, span, rake)?);

        plan.interactive = class.starts_with("group/");

        return Ok(plan);
    };

    if split_variant(utility).is_some() {
        return Err(syn::Error::new(
            span,
            "variants can't be stacked; GPUI applies one interaction state per style block",
        ));
    }

    if utility.starts_with("group/")
        || matches!(
            utility,
            "overflow-scroll" | "overflow-x-scroll" | "overflow-y-scroll"
        )
    {
        return Err(syn::Error::new(
            span,
            "this utility sets interactivity and cannot be used inside variants",
        ));
    }

    let methods = utility_methods(utility, span, rake)?;

    if name == "disabled" {
        let condition = disabled.ok_or_else(|| {
            syn::Error::new(span, "disabled: classes are supported only on buttons")
        })?;

        return Ok(StylePlan::conditional(condition.clone(), methods));
    }

    let variant = Variant::parse(name, span)?;
    let mut plan = StylePlan::default();

    plan.variants
        .insert(variant, vec![super::style_plan::Segment::Always(methods)]);
    plan.interactive = true;

    Ok(plan)
}
