//! Shared invocation discovery for source-only linting and formatting.

use proc_macro2::{Group, Span, TokenStream, TokenTree};
use syn::visit::Visit;

pub(crate) struct Invocation {
    pub tokens: TokenStream,
    pub open: Span,
    pub close: Span,
}

impl From<Group> for Invocation {
    fn from(group: Group) -> Self {
        Self {
            tokens: group.stream(),
            open: group.span_open(),
            close: group.span_close(),
        }
    }
}

/// Other macros are opaque to syn; preserve the linter's nested-token discovery.
pub(crate) fn nested_views(tokens: TokenStream, visit: &mut impl FnMut(Invocation)) {
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut position = 0;
    while position < tokens.len() {
        if position + 2 < tokens.len()
            && matches!(&tokens[position], TokenTree::Ident(name) if name == "view")
            && matches!(&tokens[position + 1], TokenTree::Punct(mark) if mark.as_char() == '!')
            && let TokenTree::Group(group) = &tokens[position + 2]
        {
            visit(group.clone().into());
            position += 3;
            continue;
        }
        if let TokenTree::Group(group) = &tokens[position] {
            nested_views(group.stream(), visit);
        }
        position += 1;
    }
}

pub(crate) fn invocations(source: &str) -> syn::Result<Vec<Invocation>> {
    struct Collector(Vec<Invocation>);
    impl<'ast> Visit<'ast> for Collector {
        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if mac
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "view")
            {
                let span = match &mac.delimiter {
                    syn::MacroDelimiter::Paren(token) => token.span,
                    syn::MacroDelimiter::Brace(token) => token.span,
                    syn::MacroDelimiter::Bracket(token) => token.span,
                };
                self.0.push(Invocation {
                    tokens: mac.tokens.clone(),
                    open: span.open(),
                    close: span.close(),
                });
            } else if !mac.path.is_ident("macro_rules") {
                nested_views(mac.tokens.clone(), &mut |invocation| {
                    self.0.push(invocation)
                });
            }
        }
    }
    let file = syn::parse_file(source)?;
    let mut collector = Collector(Vec::new());
    collector.visit_file(&file);
    Ok(collector.0)
}
