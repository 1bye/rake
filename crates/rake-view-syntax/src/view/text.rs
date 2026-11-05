//! Unquoted text uses source locations to preserve punctuation and normalize whitespace.

use proc_macro2::{Delimiter, Span, TokenTree};
use syn::parse::ParseStream;
use syn::{Expr, LitStr, Pat, Token};

use super::Node;

pub(super) fn starts_control(input: ParseStream<'_>) -> bool {
    let fork = input.fork();

    let parsed = if fork.peek(Token![if]) {
        fork.parse::<Token![if]>()
            .and_then(|_| fork.call(Expr::parse_without_eager_brace))
            .map(|_| ())
    } else if fork.peek(Token![match]) {
        fork.parse::<Token![match]>()
            .and_then(|_| fork.call(Expr::parse_without_eager_brace))
            .map(|_| ())
    } else if fork.peek(Token![for]) {
        fork.parse::<Token![for]>()
            .and_then(|_| fork.call(Pat::parse_single))
            .and_then(|_| fork.parse::<Token![in]>())
            .and_then(|_| fork.call(Expr::parse_without_eager_brace))
            .map(|_| ())
    } else {
        return false;
    };

    parsed.is_ok() && fork.peek(syn::token::Brace)
}

pub(super) fn push_node(
    input: ParseStream<'_>,
    nodes: &mut Vec<Node>,
    previous: &mut Span,
) -> syn::Result<()> {
    let mut cursor = input.cursor();
    let first = cursor.span();
    let quoted = input.peek(LitStr);
    let mut node: Node = input.parse()?;

    // Only unquoted text adopts surrounding source whitespace. Existing string
    // literals keep their exact contents and never gain indentation spaces.
    if let Node::Text(text) = &mut node
        && !quoted
        && same_line_gap(*previous, first)
    {
        *text = LitStr::new(&format!(" {}", text.value()), text.span());
    }

    let end = input.cursor();

    while cursor != end {
        let (token, next) = cursor
            .token_tree()
            .expect("parsed node advances the cursor");

        *previous = token.span();
        cursor = next;
    }

    nodes.push(node);

    Ok(())
}

fn same_line_gap(left: Span, right: Span) -> bool {
    left.end().line == right.start().line && left.end().column < right.start().column
}

fn append_token(text: &mut String, previous: Span, token: &TokenTree) {
    let span = token.span();

    let source = match token {
        TokenTree::Group(group) => {
            let (open, close) = match group.delimiter() {
                Delimiter::Parenthesis => ("(", ")"),
                Delimiter::Bracket => ("[", "]"),
                Delimiter::Brace => ("{", "}"),
                Delimiter::None => ("", ""),
            };

            let mut content = String::new();
            let mut previous = group.span_open();

            for token in group.stream() {
                append_token(&mut content, previous, &token);
                previous = token.span();
            }

            format!("{open}{content}{close}")
        }
        _ => token.to_string(),
    };

    // Synthesized tokens may share one span. Separate adjacent words in that
    // case; source-written punctuation still follows its actual source gaps.
    let synthesized_words = previous.start() == span.start()
        && previous.end() == span.end()
        && text.chars().last().is_some_and(char::is_alphanumeric)
        && source.chars().next().is_some_and(char::is_alphanumeric);

    if !text.is_empty() && (previous.end() < span.start() || synthesized_words) {
        text.push(' ');
    }

    text.push_str(&source);
}

pub(super) fn parse(input: ParseStream<'_>) -> syn::Result<LitStr> {
    let first = input.span();
    let mut previous = first;
    let mut text = String::new();

    while !input.is_empty()
        && !input.peek(Token![<])
        && !input.peek(syn::token::Brace)
        && !input.peek(LitStr)
        && !starts_control(input)
    {
        let token: TokenTree = input.parse()?;
        let span = token.span();

        append_token(&mut text, previous, &token);
        previous = span;
    }

    if text.is_empty() {
        return Err(input.error("expected an element, text, {expression}, if, for, or match"));
    }

    if !input.is_empty() && same_line_gap(previous, input.span()) {
        text.push(' ');
    }

    Ok(LitStr::new(&text, first))
}
