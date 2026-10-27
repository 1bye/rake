//! Parses HTML-like tokens and lowers them directly to GPUI builders.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Expr, Ident, LitStr, Pat, Token, braced};

use self::tag::{Tag, display_name};

pub(super) struct View(Element);

struct Element {
    tag: syn::Path,
    resolved: Tag,
    display_name: String,
    self_closing: bool,
    attributes: Vec<Attribute>,
    children: Vec<Node>,
}

struct Attribute {
    name: String,
    action: Option<syn::Path>,
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

        root.check_tags()?;

        Ok(Self(root))
    }
}

impl Parse for Element {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![<]>()?;
        let tag = input.call(syn::Path::parse_mod_style)?;
        let tag_name = display_name(&tag);
        let resolved = Tag::resolve(&tag);
        let mut attributes = Vec::new();
        let mut names = BTreeSet::new();

        while !input.peek(Token![>]) && !input.peek(Token![/]) {
            let name = input.call(Ident::parse_any)?;
            let span = name.span();
            let mut name = name.to_string();
            let mut action = None;

            if input.peek(Token![:]) {
                if name != "on" {
                    return Err(syn::Error::new(
                        span,
                        "typed actions use on:Action={handler}",
                    ));
                }

                input.parse::<Token![:]>()?;
                let path = input.call(syn::Path::parse_mod_style)?;

                name = format!("on:{}", quote!(#path).to_string().replace(' ', ""));
                action = Some(path);
            }

            while action.is_none() && input.peek(Token![-]) {
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

            attributes.push(Attribute {
                name,
                action,
                span,
                value,
            });
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
                        format!("missing closing </{tag_name}>"),
                    ));
                }

                children.push(input.parse()?);
            }

            input.parse::<Token![<]>()?;
            input.parse::<Token![/]>()?;
            let closing = input.call(syn::Path::parse_mod_style)?;

            if quote!(#closing).to_string() != quote!(#tag).to_string() {
                return Err(syn::Error::new(
                    closing.span(),
                    format!("expected closing </{tag_name}>"),
                ));
            }

            input.parse::<Token![>]>()?;
        }

        Ok(Self {
            tag,
            resolved,
            display_name: tag_name,
            self_closing,
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
            Node::Element(element)
                if element.resolved != Tag::Entity && element.attribute("id").is_none() =>
            {
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
            Node::Text(text) => return Err(loop_text_error(text.span())),
            Node::Expression(expression) => return Err(loop_text_error(expression.span())),
            _ => {}
        }
    }

    Ok(())
}

fn loop_text_error(span: Span) -> syn::Error {
    syn::Error::new(
        span,
        "text and expressions in a for loop require an element wrapper with an explicit stable id",
    )
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
    fn is_component(&self) -> bool {
        self.resolved == Tag::Component
    }

    fn check_tags(&self) -> syn::Result<()> {
        if matches!(self.resolved, Tag::Inline | Tag::Unsupported) {
            return Err(syn::Error::new(
                self.tag.span(),
                format!("unsupported view element <{}>", self.display_name),
            ));
        }

        if self.resolved.composes_text() {
            return Ok(());
        }

        fn check_nodes(nodes: &[Node]) -> syn::Result<()> {
            for node in nodes {
                match node {
                    Node::Element(element) => element.check_tags()?,
                    Node::If {
                        then_nodes,
                        else_nodes,
                        ..
                    } => {
                        check_nodes(then_nodes)?;
                        check_nodes(else_nodes)?;
                    }
                    Node::For { children, .. } => check_nodes(children)?,
                    _ => {}
                }
            }

            Ok(())
        }

        check_nodes(&self.children)
    }

    fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }

    fn expand(&self, rake: &TokenStream, next: &mut usize) -> syn::Result<TokenStream> {
        if self.resolved == Tag::Entity {
            return self.expand_entity(rake);
        }

        if self.is_component() {
            return self.expand_component(rake, next);
        }

        let index = *next;

        *next += 1;
        let node = format_ident!("__rake_view_element_{index}", span = Span::mixed_site());
        let siblings = format_ident!("__rake_view_children_{index}", span = Span::mixed_site());
        let disabled = format_ident!("__rake_view_disabled_{index}", span = Span::mixed_site());
        let mapped = format_ident!("__rake_view_builder_{index}", span = Span::mixed_site());
        let is_button = self.resolved.supports_disabled();
        let mut setup = TokenStream::new();

        if let Some(attribute) = self.attribute("disabled")
            && !is_button
        {
            return Err(syn::Error::new(
                attribute.span,
                "disabled is supported only on button",
            ));
        }

        if is_button {
            let value = self.attribute("disabled").map(|attribute| &attribute.value);
            let value = value.map_or_else(|| quote!(false), |value| quote!(#value));

            setup.extend(quote!(let #disabled: bool = #value;));
        }

        let id = self.attribute("id").map(|attribute| &attribute.value);

        let id = id.map_or_else(
            || {
                quote_spanned!(self.tag.span()=> concat!(
                    "rake::view:", file!(), ":", line!(), ":", column!(), ":", #index
                ))
            },
            |value| quote!(#value),
        );

        let mut builder = self.base_builder(&id, rake)?;

        if self.resolved.auto_label()
            && self.attribute("aria-label").is_none()
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

        builder = self.apply_attributes(builder, rake, index, &disabled, &mapped)?;

        builder = self.apply_focus(builder, rake, &disabled, &mapped)?;

        if self.resolved.composes_text() {
            return self.expand_paragraph(builder, rake, next);
        }

        let children = expand_children_into(&self.children, &node, &siblings, rake, next)?;

        let output = if self.children.is_empty() {
            builder
        } else {
            quote!({
                let mut #node = #builder;
                let mut #siblings = #rake::__private::ViewChildren::default();
                #children
                #node
            })
        };

        Ok(quote!({ #setup #output }))
    }

    fn base_builder(&self, id: &TokenStream, rake: &TokenStream) -> syn::Result<TokenStream> {
        Ok(match self.resolved {
            Tag::Container => quote!(#rake::gpui::div().id(#id).flex().flex_col()),
            Tag::Paragraph => quote!(#rake::gpui::div().id(#id).flex().flex_col().min_w_0()
                .whitespace_normal().role(#rake::gpui::accesskit::Role::Paragraph)),
            Tag::Heading(level) => {
                let level = level as usize;

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
            Tag::List => quote!(#rake::gpui::div().id(#id).flex().flex_col()
                .role(#rake::gpui::accesskit::Role::List)),
            Tag::ListItem => quote!(#rake::gpui::div().id(#id).flex().flex_col()
                .role(#rake::gpui::accesskit::Role::ListItem)),
            Tag::Button => quote!(#rake::gpui::div().id(#id).flex().items_center().justify_center()
                .role(#rake::gpui::accesskit::Role::Button).cursor_pointer()),
            Tag::Image => {
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
                    format!("unsupported view element <{}>", self.display_name),
                ));
            }
        })
    }

    fn apply_attributes(
        &self,
        mut builder: TokenStream,
        rake: &TokenStream,
        index: usize,
        disabled: &Ident,
        mapped: &Ident,
    ) -> syn::Result<TokenStream> {
        let is_button = self.resolved.supports_disabled();

        for attribute in &self.attributes {
            let value = &attribute.value;

            if self.resolved == Tag::Image
                && matches!(
                    attribute.name.as_str(),
                    "role" | "aria-keyshortcuts" | "aria-expanded"
                )
            {
                return Err(syn::Error::new(
                    attribute.span,
                    "images expose their role and label through alt; this attribute requires a container or button",
                ));
            }

            if let Some(action) = &attribute.action {
                let handler = quote!(.on_action::<#action>(#value));

                let handler = if is_button {
                    quote!(.when(!#disabled, |#mapped| #mapped #handler))
                } else {
                    handler
                };

                builder.extend(quote_spanned!(attribute.span=> #handler));
                continue;
            }

            let method = match attribute.name.as_str() {
                "id" => continue,
                "disabled" | "focus" | "tabindex" | "tab-navigation" => continue,
                "src" if self.resolved == Tag::Image => continue,
                "alt" if self.resolved == Tag::Image => quote!(.aria_label(#value)),
                "aria-label" => quote!(.aria_label(#value)),
                "aria-description" => quote!(.aria_description(#value)),
                "aria-keyshortcuts" => quote!(.aria_keyshortcuts(#value)),
                "aria-expanded" => quote!(.aria_expanded(#value)),
                "role" => quote!(.role(#value)),
                "key-context" => quote!(.key_context(#value)),
                "scroll" => {
                    require_borrowed_place(value, "scroll")?;

                    quote!(.track_scroll(#value))
                }
                "wrap" if self.resolved.composes_text() => quote!(.when_else(#value,
                    |#mapped| #mapped.whitespace_normal(),
                    |#mapped| #mapped.whitespace_nowrap())),
                "ellipsis" if self.resolved.composes_text() => quote!(.when(#value,
                    |#mapped| #mapped.overflow_hidden().text_ellipsis())),
                "max-lines" if self.resolved.composes_text() => {
                    let limit =
                        format_ident!("__rake_paragraph_limit_{index}", span = Span::mixed_site());

                    quote!(.line_clamp({ let #limit: usize = #value; #limit.max(1) }))
                }
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
                        format!(
                            "unsupported attribute `{}` on <{}>",
                            attribute.name, self.display_name
                        ),
                    ));
                }
            };

            let method = if is_button && attribute.name.starts_with("on") {
                quote!(.when(!#disabled, |#mapped| #mapped #method))
            } else {
                method
            };

            builder.extend(quote_spanned!(attribute.span=> #method));
        }

        Ok(builder)
    }

    fn apply_focus(
        &self,
        mut builder: TokenStream,
        rake: &TokenStream,
        disabled: &Ident,
        mapped: &Ident,
    ) -> syn::Result<TokenStream> {
        let is_button = self.resolved.supports_disabled();
        let focus = self.attribute("focus").map(|attribute| &attribute.value);

        if let Some(focus) = focus {
            require_borrowed_place(focus, "focus")?;
        }

        let tabindex = self.attribute("tabindex").map(|attribute| &attribute.value);
        let tabindex = tabindex.map_or_else(|| quote!(None), |value| quote!(Some(#value)));

        if !is_button {
            if let Some(focus) = focus {
                builder = quote!(#rake::__private::view_focus(#builder, #focus, #tabindex));
            } else if self.attribute("tabindex").is_some() {
                let value = &self.attribute("tabindex").expect("checked tabindex").value;

                builder = quote!(#rake::__private::view_tab_index(#builder, #value));
            }
        }

        if let Some(navigation) = self.attribute("tab-navigation") {
            let value = &navigation.value;

            builder.extend(quote!(.when(#value, |#mapped| #mapped.on_key_down(#rake::__private::view_tab_navigation))));
        }

        if is_button {
            let focus = focus.map_or_else(|| quote!(None), |value| quote!(Some((#value).clone())));

            builder = quote!(#rake::__private::view_button(#builder, #disabled, #focus, #tabindex));
        }

        Ok(builder)
    }

    fn expand_entity(&self, rake: &TokenStream) -> syn::Result<TokenStream> {
        if !self.children.is_empty() {
            return Err(syn::Error::new(
                self.tag.span(),
                "view mounts a retained entity and cannot have children",
            ));
        }

        for attribute in &self.attributes {
            if attribute.name != "entity" {
                return Err(syn::Error::new(
                    attribute.span,
                    "view accepts only entity={&stored_entity}; put styles and IDs on a surrounding element",
                ));
            }
        }

        let entity = self.attribute("entity").ok_or_else(|| {
            syn::Error::new(self.tag.span(), "view requires entity={&stored_entity}")
        })?;

        require_borrowed_place(&entity.value, "entity")?;
        let value = &entity.value;

        Ok(quote_spanned!(entity.span=> #rake::__private::view_entity(#value)))
    }
}

fn require_borrowed_place(value: &Expr, attribute: &str) -> syn::Result<()> {
    fn stored_place(value: &Expr) -> bool {
        match value {
            Expr::Path(_) => true,
            Expr::Field(field) => stored_place(&field.base),
            Expr::Index(index) => stored_place(&index.expr),
            Expr::Paren(paren) => stored_place(&paren.expr),
            _ => false,
        }
    }

    if let Expr::Reference(reference) = value
        && reference.mutability.is_none()
        && stored_place(&reference.expr)
    {
        return Ok(());
    }

    Err(syn::Error::new_spanned(
        value,
        format!(
            "{attribute} must borrow stored state, e.g. {attribute}={{&self.{attribute}}}; construct it once outside render"
        ),
    ))
}

fn expand_children_into(
    nodes: &[Node],
    parent: &Ident,
    siblings: &Ident,
    rake: &TokenStream,
    next: &mut usize,
) -> syn::Result<TokenStream> {
    let mut output = TokenStream::new();

    for node in nodes {
        output.extend(match node {
            Node::Element(element) => {
                let source = element.tag.span();
                let element = element.expand(rake, next)?;

                quote_spanned!(source=> #siblings.push(&mut #parent, #element);)
            }
            Node::Text(text) => {
                expand_text_child(quote!(#text), text.span(), parent, siblings, rake, next)
            }
            Node::Expression(expression) => expand_text_child(
                quote!(#expression),
                expression.span(),
                parent,
                siblings,
                rake,
                next,
            ),
            Node::If {
                condition,
                then_nodes,
                else_nodes,
            } => {
                let then_nodes = expand_children_into(then_nodes, parent, siblings, rake, next)?;
                let else_nodes = expand_children_into(else_nodes, parent, siblings, rake, next)?;

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
                require_loop_ids(children)?;
                let children = expand_children_into(children, parent, siblings, rake, next)?;

                quote!(for #pattern in #iterator { #children })
            }
        });
    }

    Ok(output)
}

fn expand_text_child(
    value: TokenStream,
    source: Span,
    parent: &Ident,
    siblings: &Ident,
    rake: &TokenStream,
    next: &mut usize,
) -> TokenStream {
    let index = *next;

    *next += 1;

    quote_spanned!(source=> #siblings.push_child(
        &mut #parent, #value, #rake::gpui::ElementId::from(("rake::view:child", #index))
    );)
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

mod component;
mod paragraph;
mod tag;

#[cfg(test)]
mod tests;
