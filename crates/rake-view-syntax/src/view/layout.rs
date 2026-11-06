//! Source-only layout captured by the same parser that builds the semantic tree.

use proc_macro2::Span;
use syn::buffer::Cursor;
use syn::parse::ParseStream;

/// A source range and its nested layout boundaries; comments live in source gaps.
#[derive(Clone)]
pub struct Layout {
    /// Span of the first token, including an opening delimiter.
    pub start: Span,
    /// Span of the last token, including a closing delimiter.
    pub end: Span,
    /// Formatting role of this range.
    pub kind: LayoutKind,
    /// Contained ranges in original source order.
    pub children: Vec<Layout>,
    pub(super) errors: Vec<syn::Error>,
}

/// Source roles needed by the formatter without reinterpreting markup grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    /// Complete element.
    Element,
    /// Opening tag and its attributes.
    Opening,
    /// One attribute, including its original value delimiters.
    Attribute,
    /// Closing tag.
    Closing,
    /// Quoted text or fixed punctuation.
    Literal,
    /// Unquoted text; its source spacing can affect lowering.
    Text,
    /// Braced embedded Rust.
    Rust,
    /// Control-flow construct or match arm.
    Flow,
    /// Control-flow header, excluding the body delimiter.
    Header,
    /// Braced markup body or match-arm list.
    Block,
}

impl Layout {
    pub(super) fn capture(
        kind: LayoutKind,
        mut start: Cursor<'_>,
        input: ParseStream<'_>,
        children: Vec<Self>,
    ) -> Self {
        let first = start.span();
        let mut last = first;
        while start != input.cursor() {
            let Some((token, next)) = start.token_tree() else {
                break;
            };
            last = token.span();
            start = next;
        }
        Self {
            start: first,
            end: last,
            kind,
            children,
            errors: Vec::new(),
        }
    }

    pub(super) fn validate(&self) -> syn::Result<()> {
        if let Some(error) = self.errors.first() {
            return Err(error.clone());
        }
        for child in &self.children {
            child.validate()?;
        }
        Ok(())
    }
}
