//! Collect local failures while retaining valid builder contexts for IDE type checking.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::spanned::Spanned;

use super::tag::{HEADING_CONTENT, PARAGRAPH_CONTENT, Tag};
use super::{Attribute, Diagnostics, Element, Node, View, require_loop_ids};

impl View {
    pub(crate) fn expand_resilient(mut self, rake: &TokenStream) -> syn::Result<TokenStream> {
        let mut diagnostics = Diagnostics::default();

        self.0.recover(rake, None, &mut diagnostics);
        let root = match self.emit(rake) {
            Ok(root) => root,
            Err(error) => {
                diagnostics.push(error);

                return Ok(diagnostics
                    .0
                    .expect("emission error was just recorded")
                    .to_compile_error());
            }
        };

        if let Some(errors) = diagnostics.0 {
            let errors = errors.to_compile_error();

            // Keep valid builders rather than extracting closures: handlers need
            // their GPUI builder's types for rust-analyzer and compiler inference.
            return Ok(quote!({
                #errors
                #[allow(unreachable_code, unused)]
                if false { let _ = #root; }
                ::core::unreachable!()
            }));
        }

        Ok(root)
    }

    #[cfg(test)]
    pub(super) fn expand(mut self, rake: &TokenStream) -> syn::Result<TokenStream> {
        let mut diagnostics = Diagnostics::default();

        self.0.recover(rake, None, &mut diagnostics);

        diagnostics.finish(self.emit(rake))
    }
}

impl Element {
    fn recover(&mut self, rake: &TokenStream, inline: Option<bool>, errors: &mut Diagnostics) {
        let mut invalid_component =
            self.resolved == Tag::Component && !self.parse_errors.is_empty();

        for error in self.parse_errors.drain(..) {
            errors.push(error);
        }

        let original = Self {
            tag: self.tag.clone(),
            resolved: self.resolved,
            display_name: self.display_name.clone(),
            self_closing: self.self_closing,
            attributes: self.attributes.clone(),
            children: Vec::new(),
            parse_errors: Vec::new(),
            defaults: self.defaults.clone(),
        };

        if let Some(heading) = inline {
            if !Tag::is_inline_name(&self.tag) {
                let message = if heading {
                    HEADING_CONTENT
                } else {
                    PARAGRAPH_CONTENT
                };

                errors.push(syn::Error::new(self.tag.span(), message));
                self.clear_as("span");

                return;
            }

            if self.tag.is_ident("br") && (!self.attributes.is_empty() || !self.children.is_empty())
            {
                errors.push(syn::Error::new(
                    self.tag.span(),
                    "br accepts no attributes or children",
                ));
                self.attributes.clear();
                self.children.clear();

                return;
            }
        } else {
            if matches!(self.resolved, Tag::Inline | Tag::Unsupported) {
                errors.push(syn::Error::new(
                    self.tag.span(),
                    format!("unsupported view element <{}>", self.display_name),
                ));
                self.clear_as("div");

                return;
            }

            for (name, message) in self.resolved.required_attributes() {
                if self.attribute(name).is_none() {
                    errors.push(syn::Error::new(self.tag.span(), message));
                    invalid_component = true;
                }
            }

            if let Some(message) = self.resolved.leaf_message()
                && !self.children.is_empty()
            {
                errors.push(syn::Error::new(self.tag.span(), message));
                self.children.clear();
            }
        }

        self.attributes.retain(|attribute| {
            if let Err(error) = original.probe_attribute(attribute, rake, inline.is_some()) {
                errors.push(error);
                invalid_component |= matches!(original.resolved, Tag::Component | Tag::Entity);

                false
            } else {
                true
            }
        });

        if invalid_component {
            // A malformed component/entity cannot supply reliable property types.
            // Skip its properties to avoid unrelated missing-type errors.
            self.resolved = Tag::Container(None);
            self.tag = syn::parse_quote!(div);
            self.display_name = "div".into();
            self.attributes.clear();
            self.defaults = None;
        }

        let child_inline = inline.or_else(|| {
            self.resolved
                .composes_text()
                .then_some(matches!(self.resolved, Tag::Heading(_)))
        });

        recover_nodes(&mut self.children, rake, child_inline, errors);
    }

    fn clear_as(&mut self, name: &str) {
        self.tag = syn::parse_str(name).expect("built-in recovery tag");
        self.resolved = Tag::resolve(&self.tag);
        self.display_name = name.into();
        self.attributes.clear();
        self.children.clear();
        self.defaults = None;
    }

    fn probe_attribute(
        &self,
        attribute: &Attribute,
        rake: &TokenStream,
        inline: bool,
    ) -> syn::Result<()> {
        let mut probe = self.clone();

        probe.children.clear();
        probe.attributes = vec![attribute.clone()];

        if inline {
            let dummy = format_ident!("__rake_probe", span = Span::mixed_site());

            super::paragraph::inline_element(&probe, &dummy, &dummy, rake, &mut 0)?;
        } else {
            // Required properties are context, not independent errors on every attribute.
            for (name, _) in self.resolved.required_attributes() {
                if attribute.name != *name {
                    let value = self.attribute(name).cloned().unwrap_or_else(|| Attribute {
                        name: (*name).into(),
                        action: None,
                        span: self.tag.span(),
                        value: syn::parse_quote!(""),
                    });

                    probe.attributes.push(value);
                }
            }

            probe.expand(rake, &mut 0)?;
        }

        Ok(())
    }
}

fn recover_nodes(
    nodes: &mut Vec<Node>,
    rake: &TokenStream,
    inline: Option<bool>,
    errors: &mut Diagnostics,
) {
    nodes.retain_mut(|node| {
        match node {
            Node::Element(element) => element.recover(rake, inline, errors),
            Node::If {
                then_nodes,
                else_nodes,
                ..
            } => {
                recover_nodes(then_nodes, rake, inline, errors);
                recover_nodes(else_nodes, rake, inline, errors);
            }
            Node::Match { arms, .. } => {
                for arm in arms {
                    recover_nodes(&mut arm.children, rake, inline, errors);
                }
            }
            Node::For { children, .. } => {
                if inline.is_none() {
                    let mut invalid = false;

                    for child in children.iter() {
                        if let Err(error) = require_loop_ids(std::slice::from_ref(child)) {
                            errors.push(error);
                            invalid = true;
                        }
                    }

                    if invalid {
                        // A malformed loop root has no reliable element identity/context.
                        return false;
                    }
                }

                recover_nodes(children, rake, inline, errors);
            }
            _ => {}
        }

        true
    });
}
