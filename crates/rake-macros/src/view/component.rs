//! Constructs typed GPUI components, collecting authored children only when requested.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

use super::{AttributeValue, Element, expand_children_into};

impl Element {
    pub(super) fn expand_component(
        &self,
        rake: &TokenStream,
        next: &mut usize,
    ) -> syn::Result<TokenStream> {
        let index = *next;
        *next += 1;
        let definition = self.inspection_definition(rake, index);
        let key = if self.attribute("id").is_some() {
            quote!(|component| #rake::__private::view_component_key(&component.id))
        } else {
            quote!(|_| None)
        };
        let path = &self.tag;
        let mut fields = TokenStream::new();

        for attribute in &self.attributes {
            if !self.self_closing && attribute.name == "children" {
                return Err(syn::Error::new(
                    attribute.span,
                    "component children must use either children={value} on a self-closing tag or nested markup, not both",
                ));
            }

            let mut name = syn::parse_str::<syn::Ident>(&attribute.name).map_err(|_| {
                syn::Error::new(
                    attribute.span,
                    "component properties must be Rust field names; pass callbacks and styling as ordinary typed properties",
                )
            })?;

            name.set_span(attribute.span);
            let value = match &attribute.value {
                AttributeValue::Expression(value) => quote!(#value),
                AttributeValue::Markup(element) => {
                    let element = element.expand(rake, next)?;

                    quote!(#rake::gpui::IntoElement::into_any_element(#element))
                }
            };

            fields.extend(quote_spanned!(attribute.span=> #name: #value,));
        }

        let defaults = self.defaults.as_ref().map(|value| quote!(..#value));

        if self.self_closing {
            return Ok(
                quote!(#rake::__private::view_inspect_component(#path { #fields #defaults }, #definition, #key)),
            );
        }
        let children = format_ident!(
            "__rake_component_children_{index}",
            span = Span::mixed_site()
        );

        let siblings = format_ident!(
            "__rake_component_siblings_{index}",
            span = Span::mixed_site()
        );

        let body = expand_children_into(&self.children, &children, &siblings, rake, next)?;

        // Rust evaluates explicit properties before the children field, exactly once.
        Ok(quote!(#rake::__private::view_inspect_component(#path {
            #fields
            children: {
                let mut #children = #rake::__private::ViewComponentChildren::default();
                let mut #siblings = #rake::__private::ViewChildren::default();
                #body
                #children.into_vec()
            },
            #defaults
        }, #definition, #key)))
    }
}
