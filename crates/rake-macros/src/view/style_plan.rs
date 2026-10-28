//! Ordered style contributions shared by literal and conditional classes.

use std::collections::BTreeMap;

use proc_macro2::{Ident, TokenStream};
use quote::quote;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Variant {}

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

    pub fn emit_base(self, mapped: &Ident) -> TokenStream {
        debug_assert!(self.variants.is_empty());
        let mut methods = TokenStream::new();

        for segment in self.base {
            methods.extend(match segment {
                Segment::Always(tokens) => tokens,
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
}
