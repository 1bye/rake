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
        #[cfg(feature = "reactive")]
        {
            self.expand_builder(rake, next)
        }

        #[cfg(not(feature = "reactive"))]
        self.expand_struct(rake, next)
    }

    #[cfg(not(feature = "reactive"))]
    fn expand_struct(&self, rake: &TokenStream, next: &mut usize) -> syn::Result<TokenStream> {
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
            return Ok(quote!(#rake::__private::inspection!(
                #path { #fields #defaults },
                |__rake_component| #rake::__private::view_inspect_component(__rake_component, #definition, #key)
            )));
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
        let component = quote!(#path {
            #fields
            children: {
                let mut #children = #rake::__private::ViewComponentChildren::default();
                let mut #siblings = #rake::__private::ViewChildren::default();
                #body
                #children.into_vec()
            },
            #defaults
        });

        Ok(quote!(#rake::__private::inspection!(
            #component,
            |__rake_component| #rake::__private::view_inspect_component(__rake_component, #definition, #key)
        )))
    }

    #[cfg(feature = "reactive")]
    fn expand_builder(&self, rake: &TokenStream, next: &mut usize) -> syn::Result<TokenStream> {
        let index = *next;

        *next += 1;
        let definition = self.inspection_definition(rake, index);

        let key = if self.attribute("id").is_some() {
            quote!(|component| #rake::__private::view_component_key(&component.id))
        } else {
            quote!(|_| None)
        };

        let path = &self.tag;
        let mut setters = TokenStream::new();

        for attribute in &self.attributes {
            if !self.self_closing && attribute.name == "children" {
                return Err(syn::Error::new(
                    attribute.span,
                    "use either children or nested markup, not both",
                ));
            }

            let name = attribute
                .name
                .strip_prefix("on:")
                .map(|event| format!("on_{}", event.replace('-', "_")))
                .unwrap_or_else(|| attribute.name.replace('-', "_"));

            let method = syn::parse_str::<syn::Ident>(&name)
                .map_err(|_| syn::Error::new(attribute.span, "invalid component property"))?;

            match &attribute.value {
                AttributeValue::Expression(value) => {
                    if attribute.name.starts_with("on:") {
                        let value = super::events::handler(value)?;

                        setters.extend(quote_spanned!(attribute.span=> .#method(#value)));
                        continue;
                    }

                    let value = if attribute.name == "class" {
                        super::View::attribute_style(value, rake)?
                    } else {
                        quote!(#value)
                    };

                    setters.extend(quote_spanned!(attribute.span=> .#method(#value)));
                }
                AttributeValue::Markup(element) => {
                    let method = format_ident!("{name}_with");

                    let theme =
                        super::theme::used(element).map(|span| super::theme::binding(span, rake));

                    let element = element.expand(rake, next)?;

                    setters.extend(quote_spanned!(attribute.span=> .#method(window, cx, move |window, cx| { let _ = (&window, &cx); #theme #rake::gpui::IntoElement::into_any_element(#element) })));
                }
            }
        }

        if !self.self_closing {
            let children = format_ident!(
                "__rake_component_children_{index}",
                span = Span::mixed_site()
            );

            let siblings = format_ident!(
                "__rake_component_siblings_{index}",
                span = Span::mixed_site()
            );

            let body = expand_children_into(&self.children, &children, &siblings, rake, next)?;

            let theme = super::theme::nodes_use(&self.children)
                .map(|span| super::theme::binding(span, rake));

            setters.extend(quote!(.children_with(window, cx, move |window, cx| {
                let _ = (&window, &cx);
                #theme
                let mut #children = #rake::__private::ViewComponentChildren::default();
                let mut #siblings = #rake::__private::ViewChildren::default();
                #body
                #children.into_vec()
            })));
        }

        let (builder, finish) = self.defaults.as_ref().map_or_else(
            || (quote!(#path::builder()), quote!(.build())),
            |base| {
                (
                    quote!(#path::builder_empty()),
                    quote!(.build_from(#path::into_props(#base))),
                )
            },
        );

        Ok(quote!({
            #rake::__private::inspection!(
                #path::from(#builder #setters #finish),
                |component| #rake::__private::view_inspect_component(component, #definition, #key)
            )
        }))
    }
}
