//! Lowers paragraph children to one text buffer and inherited inline styles.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Ident, Lit};

use super::tag::{PARAGRAPH_CONTENT, Tag};
use super::{Element, Node};

impl Element {
    pub(super) fn expand_paragraph(
        &self,
        builder: TokenStream,
        rake: &TokenStream,
        next: &mut usize,
    ) -> syn::Result<TokenStream> {
        let index = *next;

        *next += 1;
        let content = format_ident!(
            "__rake_paragraph_content_{index}",
            span = Span::mixed_site()
        );

        let style = format_ident!("__rake_paragraph_style_{index}", span = Span::mixed_site());

        let node = format_ident!(
            "__rake_paragraph_element_{index}",
            span = Span::mixed_site()
        );

        let children = inline_children(&self.children, &content, &style, rake, next)?;

        let heading_text = format_ident!("__rake_heading_text_{index}", span = Span::mixed_site());

        let finish =
            if matches!(self.resolved, Tag::Heading(_)) && self.attribute("aria-label").is_none() {
                quote!({
                    let #heading_text = #content.finish();
                    #node.aria_label(#heading_text.full_text()).child(#heading_text)
                })
            } else {
                quote!(#node.child(#content.finish()))
            };

        Ok(quote!({
            let #node = #builder;
            let mut #content = #rake::__private::ViewParagraphContent::default();
            let #style = #rake::__private::ViewRunStyle::default();
            #children
            #finish
        }))
    }
}

fn inline_children(
    nodes: &[Node],
    content: &Ident,
    inherited: &Ident,
    rake: &TokenStream,
    next: &mut usize,
) -> syn::Result<TokenStream> {
    let mut output = TokenStream::new();

    for node in nodes {
        output.extend(match node {
            Node::Text(text) => quote_spanned!(text.span()=> #content.push(#text, &#inherited);),
            Node::Expression(value) => {
                quote_spanned!(value.span()=> #content.push(#value, &#inherited);)
            }
            Node::If {
                condition,
                then_nodes,
                else_nodes,
            } => {
                let then_nodes = inline_children(then_nodes, content, inherited, rake, next)?;
                let else_nodes = inline_children(else_nodes, content, inherited, rake, next)?;

                quote!(if #condition { #then_nodes } else { #else_nodes })
            }
            Node::Match { expression, arms } => {
                let arms = arms
                    .iter()
                    .map(|arm| {
                        let pattern = &arm.pattern;
                        let guard = arm.guard.as_ref().map(|guard| quote!(if #guard));
                        let body = inline_children(&arm.children, content, inherited, rake, next)?;

                        Ok(quote!(#pattern #guard => { #body },))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;

                quote!(match #expression { #(#arms)* })
            }
            Node::For {
                pattern,
                iterator,
                children,
            } => {
                let children = inline_children(children, content, inherited, rake, next)?;

                quote!(for #pattern in #iterator { #children })
            }
            Node::Element(element) => inline_element(element, content, inherited, rake, next)?,
        });
    }

    Ok(output)
}

pub(super) fn inline_element(
    element: &Element,
    content: &Ident,
    inherited: &Ident,
    rake: &TokenStream,
    next: &mut usize,
) -> syn::Result<TokenStream> {
    if !Tag::is_inline_name(&element.tag) {
        return Err(syn::Error::new(element.tag.span(), PARAGRAPH_CONTENT));
    }

    if element.tag.is_ident("br") {
        if !element.attributes.is_empty() || !element.children.is_empty() {
            return Err(syn::Error::new(
                element.tag.span(),
                "br accepts no attributes or children",
            ));
        }

        return Ok(quote!(#content.push("\n", &#inherited);));
    }

    let base = if element.tag.is_ident("span") {
        quote!(#inherited.clone())
    } else if element.tag.is_ident("strong") {
        quote!(#inherited.clone().highlight(#rake::gpui::FontWeight::BOLD.into()))
    } else if element.tag.is_ident("em") {
        quote!(#inherited.clone().highlight(#rake::gpui::FontStyle::Italic.into()))
    } else {
        unreachable!("Tag::is_inline_name admitted an inline tag without a style")
    };

    let mut style = base;

    for attribute in &element.attributes {
        let value = attribute.expression()?;

        let method = match attribute.name.as_str() {
            "class" => inline_classes(value, rake)?,
            "style" => quote!(.highlight(#value)),
            "font-family" => quote!(.font_family(#value)),
            name if name.starts_with("class:") => {
                return Err(syn::Error::new(
                    attribute.span,
                    "conditional inline text classes are unsupported; select a HighlightStyle with style={...}",
                ));
            }
            _ => {
                return Err(syn::Error::new(
                    attribute.span,
                    "inline text supports only class, style={gpui::HighlightStyle}, and font-family; inline IDs, layout and events are unsupported",
                ));
            }
        };

        style.extend(quote_spanned!(attribute.span=> #method));
    }

    let index = *next;

    *next += 1;
    let local = format_ident!("__rake_inline_style_{index}", span = Span::mixed_site());
    let children = inline_children(&element.children, content, &local, rake, next)?;

    Ok(quote!({
        let #local = #style;
        #children
    }))
}

fn inline_classes(value: &Expr, rake: &TokenStream) -> syn::Result<TokenStream> {
    let Expr::Lit(syn::ExprLit {
        lit: Lit::Str(classes),
        ..
    }) = value
    else {
        return Err(syn::Error::new_spanned(
            value,
            "inline class requires a literal text utility string",
        ));
    };

    let mut methods = TokenStream::new();
    let mut errors: Option<syn::Error> = None;

    for class in classes.value().split_whitespace() {
        let style = match class {
            "font-normal" => quote!(#rake::gpui::FontWeight::NORMAL.into()),
            "font-medium" => quote!(#rake::gpui::FontWeight::MEDIUM.into()),
            "font-semibold" => quote!(#rake::gpui::FontWeight::SEMIBOLD.into()),
            "font-bold" => quote!(#rake::gpui::FontWeight::BOLD.into()),
            "italic" => quote!(#rake::gpui::FontStyle::Italic.into()),
            "not-italic" => quote!(#rake::gpui::FontStyle::Normal.into()),
            "underline" => quote!(#rake::gpui::HighlightStyle {
                underline: Some(#rake::gpui::UnderlineStyle {
                    thickness: #rake::gpui::px(1.), color: None, wavy: false,
                }), ..Default::default()
            }),
            _ => {
                let error = syn::Error::new(
                    classes.span(),
                    format!(
                        "unsupported inline text utility `{class}`; font size, line height, alignment and layout belong on p"
                    ),
                );

                if let Some(errors) = &mut errors {
                    errors.combine(error);
                } else {
                    errors = Some(error);
                }

                continue;
            }
        };

        methods.extend(quote!(.highlight(#style)));
    }

    if let Some(errors) = errors {
        return Err(errors);
    }

    Ok(methods)
}
