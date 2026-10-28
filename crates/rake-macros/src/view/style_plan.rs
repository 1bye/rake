//! Ordered style contributions shared by literal and conditional classes.

use std::collections::BTreeMap;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Variant {
    Hover,
    Active,
    Focus,
    FocusVisible,
    InFocus,
    GroupHover(String),
    GroupActive(String),
}

impl Variant {
    pub fn parse(name: &str, span: Span) -> syn::Result<Self> {
        Ok(match name {
            "hover" => Self::Hover,
            "active" => Self::Active,
            "focus" => Self::Focus,
            "focus-visible" => Self::FocusVisible,
            "in-focus" => Self::InFocus,
            _ if name.starts_with("group-hover/") && name.len() > 12 => {
                Self::GroupHover(name[12..].into())
            }
            _ if name.starts_with("group-active/") && name.len() > 13 => {
                Self::GroupActive(name[13..].into())
            }
            "group-hover" | "group-active" | "group-hover/" | "group-active/" => {
                return Err(syn::Error::new(
                    span,
                    "group variants require a name, such as group-hover/card:",
                ));
            }
            _ => return Err(syn::Error::new(span, format!("unknown variant `{name}:`"))),
        })
    }

    pub fn method(&self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Active => "active",
            Self::Focus => "focus",
            Self::FocusVisible => "focus_visible",
            Self::InFocus => "in_focus",
            Self::GroupHover(_) => "group_hover",
            Self::GroupActive(_) => "group_active",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::FocusVisible => "focus-visible",
            Self::InFocus => "in-focus",
            Self::GroupHover(_) => "group-hover",
            Self::GroupActive(_) => "group-active",
            _ => self.method(),
        }
    }
}

pub(super) enum Segment {
    Always(TokenStream),
    When {
        condition: Ident,
        then: TokenStream,
        otherwise: TokenStream,
    },
}

#[derive(Default)]
pub(super) struct StylePlan {
    pub base: Vec<Segment>,
    pub variants: BTreeMap<Variant, Vec<Segment>>,
    pub interactive: bool,
}

impl StylePlan {
    pub fn always(methods: TokenStream) -> Self {
        Self {
            base: vec![Segment::Always(methods)],
            ..Self::default()
        }
    }

    pub fn conditional(condition: Ident, methods: TokenStream) -> Self {
        Self {
            base: vec![Segment::When {
                condition,
                then: methods,
                otherwise: TokenStream::new(),
            }],
            ..Self::default()
        }
    }

    pub fn extend(&mut self, other: Self, span: Span) -> syn::Result<()> {
        for variant in other.variants.keys() {
            if self
                .variants
                .keys()
                .any(|old| old != variant && old.method() == variant.method())
            {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "GPUI supports one named {} group per element",
                        variant.name()
                    ),
                ));
            }
        }

        self.base.extend(other.base);
        self.interactive |= other.interactive;

        for (variant, segments) in other.variants {
            self.variants.entry(variant).or_default().extend(segments);
        }

        Ok(())
    }

    pub fn gate(&mut self, outer: &Ident, setup: &mut TokenStream) {
        for (index, segment) in self
            .base
            .iter_mut()
            .chain(self.variants.values_mut().flatten())
            .enumerate()
        {
            match segment {
                Segment::Always(methods) => {
                    *segment = Segment::When {
                        condition: outer.clone(),
                        then: methods.clone(),
                        otherwise: TokenStream::new(),
                    };
                }
                Segment::When {
                    condition,
                    otherwise,
                    ..
                } => {
                    debug_assert!(
                        otherwise.is_empty(),
                        "class leaf selectors have no else styles"
                    );
                    let combined = format_ident!("{outer}_gate_{index}", span = Span::mixed_site());

                    setup.extend(quote!(let #combined: bool = #outer && #condition;));
                    *condition = combined;
                }
            }
        }
    }

    pub fn emit_base(&self, mapped: &Ident) -> TokenStream {
        let mut methods = TokenStream::new();

        for segment in &self.base {
            methods.extend(match segment {
                Segment::Always(tokens) => tokens.clone(),
                Segment::When { condition, then, otherwise } if otherwise.is_empty() => {
                    quote!(.when(#condition, |#mapped| #mapped #then))
                }
                Segment::When { condition, then, otherwise } => {
                    quote!(.when_else(#condition, |#mapped| #mapped #then, |#mapped| #mapped #otherwise))
                }
            });
        }

        methods
    }

    pub fn emit_refinement(&self, mapped: &Ident) -> TokenStream {
        debug_assert!(
            self.variants.is_empty(),
            "inline text has no interaction variants"
        );

        emit_segments(&self.base, mapped)
    }

    pub fn emit_variants(&self, mapped: &Ident) -> TokenStream {
        let mut methods = TokenStream::new();

        for (variant, segments) in &self.variants {
            let method = format_ident!("{}", variant.method());

            let group = match variant {
                Variant::GroupHover(name) | Variant::GroupActive(name) => quote!(#name,),
                _ => TokenStream::new(),
            };

            let body = emit_segments(segments, mapped);

            methods.extend(quote!(.#method(#group |#mapped| { #body #mapped })));
        }

        methods
    }
}

fn emit_segments(segments: &[Segment], mapped: &Ident) -> TokenStream {
    let mut body = TokenStream::new();

    for segment in segments {
        body.extend(match segment {
            Segment::Always(tokens) => quote!(let #mapped = #mapped #tokens;),
            Segment::When {
                condition,
                then,
                otherwise,
            } => {
                quote!(let #mapped = if #condition { #mapped #then } else { #mapped #otherwise };)
            }
        });
    }

    body
}
