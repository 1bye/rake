//! Parses HTML-like tokens and lowers them directly to GPUI builders.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Pat, Token, braced};

pub(super) struct View(Element);

struct Element {
    tag: Ident,
    attributes: Vec<Attribute>,
    children: Vec<Node>,
}

struct Attribute {
    name: String,
    span: Span,
    value: Expr,
}

enum Node {
    Element(Element),
    Text(LitStr),
    Expression(Expr),
    If {
        condition: Expr,
        then_nodes: Vec<Node>,
        else_nodes: Vec<Node>,
    },
    For {
        pattern: Pat,
        iterator: Expr,
        children: Vec<Node>,
    },
}

impl Parse for View {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let root = Element::parse(input)?;

        if !input.is_empty() {
            return Err(input.error("view! requires exactly one root element"));
        }

        Ok(Self(root))
    }
}

impl Parse for Element {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![<]>()?;
        let tag = input.call(Ident::parse_any)?;
        let mut attributes = Vec::new();
        let mut names = BTreeSet::new();

        while !input.peek(Token![>]) && !input.peek(Token![/]) {
            let name = input.call(Ident::parse_any)?;
            let span = name.span();
            let mut name = name.to_string();

            while input.peek(Token![-]) {
                input.parse::<Token![-]>()?;
                name.push('-');
                name.push_str(&input.call(Ident::parse_any)?.to_string());
            }

            if !names.insert(name.clone()) {
                return Err(syn::Error::new(
                    span,
                    format!("duplicate attribute `{name}`"),
                ));
            }

            input.parse::<Token![=]>()?;
            let value = if input.peek(LitStr) {
                Expr::Lit(syn::ExprLit {
                    attrs: Vec::new(),
                    lit: syn::Lit::Str(input.parse()?),
                })
            } else if input.peek(syn::token::Brace) {
                parse_expression_block(input)?
            } else {
                return Err(
                    input.error("attribute values must be quoted strings or {Rust expressions}")
                );
            };

            attributes.push(Attribute { name, span, value });
        }

        let self_closing = input.peek(Token![/]);

        if self_closing {
            input.parse::<Token![/]>()?;
        }

        input.parse::<Token![>]>()?;
        let mut children = Vec::new();

        if !self_closing {
            while !(input.peek(Token![<]) && input.peek2(Token![/])) {
                if input.is_empty() {
                    return Err(syn::Error::new(
                        tag.span(),
                        format!("missing closing </{tag}>"),
                    ));
                }

                children.push(input.parse()?);
            }

            input.parse::<Token![<]>()?;
            input.parse::<Token![/]>()?;
            let closing = input.call(Ident::parse_any)?;

            if closing != tag {
                return Err(syn::Error::new(
                    closing.span(),
                    format!("expected closing </{tag}>"),
                ));
            }

            input.parse::<Token![>]>()?;
        }

        Ok(Self {
            tag,
            attributes,
            children,
        })
    }
}

impl Parse for Node {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![<]) {
            return input.parse().map(Self::Element);
        }

        if input.peek(LitStr) {
            return input.parse().map(Self::Text);
        }

        if input.peek(syn::token::Brace) {
            return parse_expression_block(input).map(Self::Expression);
        }

        if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;
            let condition = input.call(Expr::parse_without_eager_brace)?;
            let then_nodes = parse_nodes(input)?;
            let mut else_nodes = Vec::new();

            if input.peek(Token![else]) {
                input.parse::<Token![else]>()?;

                if input.peek(Token![if]) {
                    else_nodes.push(input.parse()?);
                } else {
                    else_nodes = parse_nodes(input)?;
                }
            }

            return Ok(Self::If {
                condition,
                then_nodes,
                else_nodes,
            });
        }

        if input.peek(Token![for]) {
            input.parse::<Token![for]>()?;
            let pattern = input.call(Pat::parse_single)?;

            input.parse::<Token![in]>()?;
            let iterator = input.call(Expr::parse_without_eager_brace)?;
            let children = parse_nodes(input)?;

            require_loop_ids(&children)?;

            return Ok(Self::For {
                pattern,
                iterator,
                children,
            });
        }

        Err(input.error("expected an element, quoted text, {expression}, if, or for"))
    }
}

fn parse_nodes(input: ParseStream<'_>) -> syn::Result<Vec<Node>> {
    let content;

    braced!(content in input);
    let mut nodes = Vec::new();

    while !content.is_empty() {
        nodes.push(content.parse()?);
    }

    Ok(nodes)
}

fn parse_expression_block(input: ParseStream<'_>) -> syn::Result<Expr> {
    let block: syn::Block = input.parse()?;

    if let [syn::Stmt::Expr(expression, None)] = block.stmts.as_slice() {
        return Ok(expression.clone());
    }

    Ok(Expr::Block(syn::ExprBlock {
        attrs: Vec::new(),
        label: None,
        block,
    }))
}

fn require_loop_ids(nodes: &[Node]) -> syn::Result<()> {
    for node in nodes {
        match node {
            Node::Element(element) if element.attribute("id").is_none() => {
                return Err(syn::Error::new(
                    element.tag.span(),
                    "element roots in a for loop require an explicit stable id",
                ));
            }
            Node::If {
                then_nodes,
                else_nodes,
                ..
            } => {
                require_loop_ids(then_nodes)?;
                require_loop_ids(else_nodes)?;
            }
            Node::For { children, .. } => require_loop_ids(children)?,
            Node::Text(_) | Node::Expression(_) => {
                return Err(syn::Error::new(
                    match node {
                        Node::Text(text) => text.span(),
                        Node::Expression(expression) => syn::spanned::Spanned::span(expression),
                        _ => unreachable!(),
                    },
                    "text and expressions in a for loop require an element wrapper with an explicit stable id",
                ));
            }
            _ => {}
        }
    }

    Ok(())
}

impl View {
    pub(super) fn expand(self, rake: &TokenStream) -> syn::Result<TokenStream> {
        let root = self.0.expand(rake, &mut 0)?;

        Ok(quote!({
            use #rake::gpui::prelude::{
                FluentBuilder as _, InteractiveElement as _, ParentElement as _,
                StatefulInteractiveElement as _, Styled as _, StyledImage as _,
            };
            #root
        }))
    }
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }

    fn expand(&self, rake: &TokenStream, next: &mut usize) -> syn::Result<TokenStream> {
        let index = *next;

        *next += 1;
        let node = format_ident!("__rake_view_element_{index}", span = Span::mixed_site());
        let tag = self.tag.to_string();
        let id = self.attribute("id").map(|attribute| &attribute.value);

        let id = id.map_or_else(
            || {
                quote_spanned!(self.tag.span()=> concat!(
                    "rake::view:", file!(), ":", line!(), ":", column!(), ":", #index
                ))
            },
            |value| quote!(#value),
        );

        let mut builder = match tag.as_str() {
            "div" | "section" | "span" => quote!(#rake::gpui::div().id(#id).flex().flex_col()),
            "p" => quote!(#rake::gpui::div().id(#id).flex().flex_col()
                .role(#rake::gpui::accesskit::Role::Paragraph)),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = tag.as_bytes()[1] as usize - b'0' as usize;

                let size = match level {
                    1 => 30.0_f32,
                    2 => 24.0,
                    3 => 20.0,
                    _ => 16.0,
                };

                quote!(#rake::gpui::div().id(#id).flex().flex_col()
                    .role(#rake::gpui::accesskit::Role::Heading).aria_level(#level)
                    .text_size(#rake::gpui::px(#size)).font_weight(#rake::gpui::FontWeight::BOLD))
            }
            "ul" | "ol" => quote!(#rake::gpui::div().id(#id).flex().flex_col()
                .role(#rake::gpui::accesskit::Role::List)),
            "li" => quote!(#rake::gpui::div().id(#id).flex().flex_col()
                .role(#rake::gpui::accesskit::Role::ListItem)),
            "button" => quote!(#rake::gpui::div().id(#id).flex().items_center().justify_center()
                .role(#rake::gpui::accesskit::Role::Button).tab_index(0).cursor_pointer()),
            "img" => {
                let source = self
                    .attribute("src")
                    .ok_or_else(|| syn::Error::new(self.tag.span(), "img requires src"))?;

                if self.attribute("alt").is_none() {
                    return Err(syn::Error::new(self.tag.span(), "img requires alt text"));
                }

                if !self.children.is_empty() {
                    return Err(syn::Error::new(self.tag.span(), "img cannot have children"));
                }

                let source = &source.value;

                quote!(#rake::__private::view_image(#source, #id))
            }
            _ => {
                return Err(syn::Error::new(
                    self.tag.span(),
                    format!("unsupported view element <{tag}>"),
                ));
            }
        };

        if matches!(
            tag.as_str(),
            "button" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) && self.attribute("aria-label").is_none()
            && !self.children.is_empty()
        {
            let labels: Option<Vec<_>> = self
                .children
                .iter()
                .map(|child| match child {
                    Node::Text(text) => Some(text.value()),
                    _ => None,
                })
                .collect();

            if let Some(labels) = labels {
                let label = labels.join(" ");

                builder.extend(quote!(.aria_label(#label)));
            }
        }

        for attribute in &self.attributes {
            let value = &attribute.value;

            let method = match attribute.name.as_str() {
                "id" => continue,
                "src" if tag == "img" => continue,
                "alt" if tag == "img" => quote!(.aria_label(#value)),
                "aria-label" => quote!(.aria_label(#value)),
                "aria-description" => quote!(.aria_description(#value)),
                "tabindex" => quote!(.tab_index(#value)),
                "class" => class_methods(value, rake)?,
                "style" => quote!(.map(#value)),
                "onclick" => quote!(.on_click(#value)),
                "onkeydown" => quote!(.on_key_down(#value)),
                "onkeyup" => quote!(.on_key_up(#value)),
                "onmousemove" => quote!(.on_mouse_move(#value)),
                "onmousedown" => quote!(.on_mouse_down(#rake::gpui::MouseButton::Left, #value)),
                "onmouseup" => quote!(.on_mouse_up(#rake::gpui::MouseButton::Left, #value)),
                _ => {
                    return Err(syn::Error::new(
                        attribute.span,
                        format!("unsupported attribute `{}` on <{tag}>", attribute.name),
                    ));
                }
            };

            builder.extend(quote_spanned!(attribute.span=> #method));
        }

        let children = expand_children(&self.children, &node, rake, next)?;

        // Img has no ParentElement implementation, including for an empty child list.
        if self.children.is_empty() {
            return Ok(quote!({ #builder }));
        }

        Ok(quote!({
            let mut #node = #builder;
            #children
            #node
        }))
    }
}

fn expand_children(
    nodes: &[Node],
    parent: &Ident,
    rake: &TokenStream,
    next: &mut usize,
) -> syn::Result<TokenStream> {
    let mut output = TokenStream::new();

    for node in nodes {
        output.extend(match node {
            Node::Element(element) => {
                let element = element.expand(rake, next)?;

                quote!(#parent = #parent.child(#element);)
            }
            Node::Text(_) | Node::Expression(_) => {
                let value = match node {
                    Node::Text(text) => quote!(#text),
                    Node::Expression(expression) => quote!(#expression),
                    _ => unreachable!(),
                };

                let index = *next;

                *next += 1;

                quote!(#parent.extend([#rake::__private::view_child(
                    #value, #rake::gpui::ElementId::from(("rake::view:child", #index))
                )]);)
            }
            Node::If {
                condition,
                then_nodes,
                else_nodes,
            } => {
                let then_nodes = expand_children(then_nodes, parent, rake, next)?;
                let else_nodes = expand_children(else_nodes, parent, rake, next)?;

                if else_nodes.is_empty() {
                    quote!(if #condition { #then_nodes })
                } else {
                    quote!(if #condition { #then_nodes } else { #else_nodes })
                }
            }
            Node::For {
                pattern,
                iterator,
                children,
            } => {
                let children = expand_children(children, parent, rake, next)?;

                quote!(for #pattern in #iterator { #children })
            }
        });
    }

    Ok(output)
}

fn class_methods(value: &Expr, rake: &TokenStream) -> syn::Result<TokenStream> {
    let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(classes),
        ..
    }) = value
    else {
        return Err(syn::Error::new_spanned(
            value,
            "class requires a literal utility string; use style={|s| ...} for dynamic styles",
        ));
    };

    let mut methods = TokenStream::new();

    for class in classes.value().split_whitespace() {
        if let Some(weight) = match class {
            "font-normal" => Some("NORMAL"),
            "font-medium" => Some("MEDIUM"),
            "font-semibold" => Some("SEMIBOLD"),
            "font-bold" => Some("BOLD"),
            _ => None,
        } {
            let weight = format_ident!("{weight}");

            methods.extend(quote!(.font_weight(#rake::gpui::FontWeight::#weight)));
            continue;
        }

        if let Some(columns) = class
            .strip_prefix("grid-cols-")
            .and_then(|value| value.parse::<u16>().ok())
            && (1..=12).contains(&columns)
        {
            methods.extend(quote!(.grid_cols(#columns)));
            continue;
        }

        let fixed = matches!(
            class,
            "flex"
                | "block"
                | "grid"
                | "hidden"
                | "flex-col"
                | "flex-row"
                | "flex-wrap"
                | "flex-1"
                | "items-start"
                | "items-center"
                | "items-end"
                | "justify-start"
                | "justify-center"
                | "justify-end"
                | "justify-between"
                | "w-full"
                | "h-full"
                | "size-full"
                | "min-w-0"
                | "min-h-0"
                | "overflow-hidden"
                | "overflow-scroll"
                | "overflow-x-scroll"
                | "overflow-y-scroll"
                | "rounded-none"
                | "rounded-sm"
                | "rounded-md"
                | "rounded-lg"
                | "rounded-xl"
                | "rounded-full"
                | "border-0"
                | "border-1"
                | "border-2"
                | "shadow-sm"
                | "shadow-md"
                | "shadow-lg"
                | "cursor-pointer"
                | "text-xs"
                | "text-sm"
                | "text-base"
                | "text-lg"
                | "text-xl"
                | "text-2xl"
                | "text-3xl"
                | "text-left"
                | "text-center"
                | "text-right"
        );

        let spacing = class.rsplit_once('-').is_some_and(|(prefix, value)| {
            matches!(
                prefix,
                "p" | "px"
                    | "py"
                    | "pt"
                    | "pb"
                    | "pl"
                    | "pr"
                    | "m"
                    | "mx"
                    | "my"
                    | "mt"
                    | "mb"
                    | "ml"
                    | "mr"
                    | "gap"
            ) && matches!(
                value,
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "12"
                    | "16"
                    | "20"
                    | "24"
                    | "32"
            )
        });

        let method = match class {
            "rounded" => "rounded_md".to_string(),
            "border" => "border_1".to_string(),
            _ if fixed || spacing => class.replace('-', "_"),
            _ => {
                return Err(syn::Error::new(
                    classes.span(),
                    format!(
                        "unsupported utility class `{class}`; use style={{|s| ...}} for GPUI styling"
                    ),
                ));
            }
        };

        let method = format_ident!("{method}", span = classes.span());

        methods.extend(quote!(.#method()));
    }

    Ok(methods)
}

#[cfg(test)]
mod tests;
