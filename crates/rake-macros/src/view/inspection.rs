//! Adds static authored source descriptors and transparent runtime inspection boundaries.

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;

use super::{Element, Node};

pub(super) fn definition(
    tag: &str,
    span: Span,
    index: usize,
    styles: TokenStream,
    rake: &TokenStream,
) -> TokenStream {
    definition_with_handlers(tag, span, index, styles, TokenStream::new(), rake)
}

fn definition_with_handlers(
    tag: &str,
    span: Span,
    index: usize,
    styles: TokenStream,
    handlers: TokenStream,
    rake: &TokenStream,
) -> TokenStream {
    quote_spanned!(span=> {
        const DEFINITION: #rake::__private::ViewDefinition = #rake::__private::ViewDefinition {
            tag: #tag,
            site: concat!(file!(), ":", line!(), ":", column!(), ":", #index),
            source: ::std::panic::Location::caller(),
            styles: &[#styles],
            handlers: &[#handlers],
        };
        &DEFINITION
    })
}

impl Element {
    pub(super) fn inspection_definition(&self, rake: &TokenStream, index: usize) -> TokenStream {
        let mut styles = TokenStream::new();

        self.style_sources(rake, &mut styles);

        if self.resolved.composes_text() {
            inline_sources(&self.children, rake, &mut styles);
        }

        let mut handlers = TokenStream::new();

        for attribute in &self.attributes {
            if !self.is_component() && super::events::is_event(&attribute.name) {
                let name = super::events::LEGACY
                    .iter()
                    .find(|(legacy, _)| *legacy == attribute.name)
                    .map(|(_, canonical)| *canonical)
                    .unwrap_or_else(|| {
                        attribute
                            .name
                            .strip_prefix("on:")
                            .unwrap_or(&attribute.name)
                    });

                handlers.extend(
                    quote_spanned!(attribute.span=> #rake::__private::ViewHandlerSite {
                    name: #name, source: ::std::panic::Location::caller(),
                },),
                );
            }
        }

        definition_with_handlers(
            &self.display_name,
            self.tag.span(),
            index,
            styles,
            handlers,
            rake,
        )
    }

    fn style_sources(&self, rake: &TokenStream, output: &mut TokenStream) {
        for attribute in &self.attributes {
            let name = attribute.name.as_str();

            if matches!(
                name,
                "class" | "style" | "font-family" | "wrap" | "ellipsis" | "max-lines"
            ) || name.starts_with("class:")
            {
                let scope = &self.display_name;

                let Ok(expression) = attribute.expression() else {
                    continue;
                };

                output.extend(
                    quote_spanned!(attribute.span=> #rake::__private::ViewStyleSite {
                    scope: #scope, attribute: #name, expression: stringify!(#expression),
                    source: ::std::panic::Location::caller(),
                },),
                );
            }
        }
    }

    pub(super) fn inspect_output(
        &self,
        output: TokenStream,
        rake: &TokenStream,
        index: usize,
    ) -> TokenStream {
        let definition = self.inspection_definition(rake, index);
        let named = self.attribute("id").is_some();
        let node = syn::Ident::new("__rake_inspection_element", Span::mixed_site());
        let mut inspected = quote_spanned!(self.tag.span()=> #rake::__private::view_inspect(#node, #definition, #named));

        for (attribute, method) in [("focus", "focus_handle"), ("scroll", "scroll_handle")] {
            if let Some(value) = self.attribute(attribute) {
                let value = value.expression().expect("validated handle");
                let method = syn::Ident::new(method, value.span());

                inspected.extend(quote!(.#method(#value)));
            }
        }

        quote!(#rake::__private::inspection!(#output, |#node| #inspected))
    }
}

fn inline_sources(nodes: &[Node], rake: &TokenStream, output: &mut TokenStream) {
    for node in nodes {
        match node {
            Node::Element(element) => {
                element.style_sources(rake, output);
                inline_sources(&element.children, rake, output);
            }
            Node::If {
                then_nodes,
                else_nodes,
                ..
            } => {
                inline_sources(then_nodes, rake, output);
                inline_sources(else_nodes, rake, output);
            }
            Node::For { children, .. } => inline_sources(children, rake, output),
            Node::Match { arms, .. } => {
                for arm in arms {
                    inline_sources(&arm.children, rake, output);
                }
            }
            _ => {}
        }
    }
}
