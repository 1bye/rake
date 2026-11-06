//! Walk Rust syntax and the shared view tree, retaining original token spans.

use std::path::Path;

use proc_macro2::{Span, TokenStream};
use rake_view_syntax::{AttributeValue, Element, Node, View};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, Pat, Token};

use super::Diagnostic;
use super::catalogue::{BUTTON, IMAGE, INDEX, SYNTAX};

pub(super) fn check(file: &Path, text: &str) -> Vec<Diagnostic> {
    let mut checker = Checker {
        file,
        diagnostics: Vec::new(),
    };

    match syn::parse_file(text) {
        Ok(file) => checker.visit_file(&file),
        Err(error) => checker.parse_error(error),
    }

    checker.diagnostics
}

struct Checker<'src> {
    file: &'src Path,
    diagnostics: Vec<Diagnostic>,
}

impl Checker<'_> {
    fn emit(&mut self, id: &'static str, span: Span) {
        self.diagnostics.push(Diagnostic::new(id, self.file, span));
    }

    fn parse_error(&mut self, error: syn::Error) {
        for error in error {
            let mut diagnostic = Diagnostic::new(SYNTAX, self.file, error.span());

            diagnostic.message = format!("{}: {error}", diagnostic.message);
            self.diagnostics.push(diagnostic);
        }
    }

    fn element(&mut self, element: &Element, indices: &[String]) {
        let has = |name| {
            element
                .attributes
                .iter()
                .any(|attribute| attribute.name == name)
        };
        // Match the macro's literal-only auto-label condition. Mixed text/expressions
        // also need an explicit name because the macro cannot derive one.
        let literal_label = !element.children.is_empty()
            && element
                .children
                .iter()
                .all(|child| matches!(child, Node::Text(_)))
            && element
                .children
                .iter()
                .any(|child| matches!(child, Node::Text(text) if !text.value().trim().is_empty()));

        if element.tag.is_ident("button") && !has("aria-label") && !literal_label {
            self.emit(BUTTON, element.tag.span());
        }

        if element.tag.is_ident("img") && !has("alt") {
            self.emit(IMAGE, element.tag.span());
        }

        for attribute in &element.attributes {
            match &attribute.value {
                AttributeValue::Expression(expression) => {
                    if attribute.name == "id" && index_id(expression, indices) {
                        self.emit(INDEX, expression.span());
                    }

                    self.visit_expr(expression);
                }
                AttributeValue::Markup(element) => self.element(element, indices),
            }
        }

        self.nodes(&element.children, indices);
    }

    fn nodes(&mut self, nodes: &[Node], indices: &[String]) {
        for node in nodes {
            match node {
                Node::Element(element) => self.element(element, indices),
                Node::Expression(expression) => self.visit_expr(expression),
                Node::Text(_) => {}
                Node::If {
                    condition,
                    then_nodes,
                    else_nodes,
                } => {
                    self.visit_expr(condition);
                    let mut nested = indices.to_vec();

                    remove_condition_bindings(condition, &mut nested);
                    self.nodes(then_nodes, &nested);
                    self.nodes(else_nodes, indices);
                }
                Node::Match { expression, arms } => {
                    self.visit_expr(expression);

                    for arm in arms {
                        let mut nested = indices.to_vec();

                        remove_bindings(&arm.pattern, &mut nested);

                        if let Some(guard) = &arm.guard {
                            self.visit_expr(guard);
                            remove_condition_bindings(guard, &mut nested);
                        }

                        self.nodes(&arm.children, &nested);
                    }
                }
                Node::For {
                    pattern,
                    iterator,
                    children,
                } => {
                    self.visit_expr(iterator);
                    let mut nested = indices.to_vec();

                    remove_bindings(pattern, &mut nested);

                    if enumerated(iterator)
                        && let Pat::Tuple(tuple) = pattern
                        && tuple.elems.len() == 2
                        && let Some(Pat::Ident(index)) = tuple.elems.first()
                    {
                        nested.push(index.ident.to_string());
                    }

                    self.nodes(children, &nested);
                }
            }
        }
    }

    // Other macros are opaque to syn. Inspect their token groups for nested view!
    // invocations, still delegating all markup parsing to View.
    fn nested_tokens(&mut self, tokens: TokenStream) {
        crate::view_source::nested_views(tokens, &mut |invocation| {
            self.view(invocation.tokens);
        });
    }

    fn view(&mut self, tokens: TokenStream) {
        match syn::parse2::<View>(tokens) {
            Ok(view) => self.element(view.root(), &[]),
            Err(error) => self.parse_error(error),
        }
    }
}

impl<'ast> Visit<'ast> for Checker<'_> {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "view")
        {
            self.view(mac.tokens.clone());
        } else if !mac.path.is_ident("macro_rules") {
            self.nested_tokens(mac.tokens.clone());
        }
    }
}

fn enumerated(expression: &Expr) -> bool {
    match expression {
        Expr::MethodCall(call) if call.method == "enumerate" && call.args.is_empty() => true,
        Expr::MethodCall(call)
            if matches!(
                call.method.to_string().as_str(),
                "rev" | "skip" | "take" | "step_by" | "by_ref"
            ) =>
        {
            enumerated(&call.receiver)
        }
        Expr::Paren(paren) => enumerated(&paren.expr),
        _ => false,
    }
}

fn remove_bindings(pattern: &Pat, indices: &mut Vec<String>) {
    struct Bindings<'a>(&'a mut Vec<String>);

    impl<'ast> Visit<'ast> for Bindings<'_> {
        fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
            self.0.retain(|name| pattern.ident != name.as_str());
            visit::visit_pat_ident(self, pattern);
        }
    }

    Bindings(indices).visit_pat(pattern);
}

fn remove_condition_bindings(condition: &Expr, indices: &mut Vec<String>) {
    match condition {
        Expr::Let(expression) => remove_bindings(&expression.pat, indices),
        Expr::Binary(expression) if matches!(expression.op, syn::BinOp::And(_)) => {
            remove_condition_bindings(&expression.left, indices);
            remove_condition_bindings(&expression.right, indices);
        }
        Expr::Paren(expression) => remove_condition_bindings(&expression.expr, indices),
        _ => {}
    }
}

fn index_id(expression: &Expr, indices: &[String]) -> bool {
    match expression {
        Expr::Path(path) => indices.iter().any(|index| path.path.is_ident(index)),
        Expr::Tuple(tuple) => tuple.elems.iter().any(|value| index_id(value, indices)),
        Expr::Paren(paren) => index_id(&paren.expr, indices),
        Expr::Group(group) => index_id(&group.expr, indices),
        Expr::Cast(cast) => index_id(&cast.expr, indices),
        Expr::Reference(reference) => index_id(&reference.expr, indices),
        Expr::Binary(binary) => index_id(&binary.left, indices) || index_id(&binary.right, indices),
        Expr::MethodCall(call) => index_id(&call.receiver, indices),
        Expr::Macro(mac) if mac.mac.path.is_ident("format") => {
            let arguments =
                Punctuated::<Expr, Token![,]>::parse_terminated.parse2(mac.mac.tokens.clone());

            if let Ok(arguments) = arguments {
                let mut captures = indices.to_vec();

                for argument in arguments.iter().skip(1) {
                    if let Expr::Assign(named) = argument {
                        if let Expr::Path(name) = &*named.left {
                            captures.retain(|index| !name.path.is_ident(index));
                        }

                        if index_id(&named.right, indices) {
                            return true;
                        }
                    } else if index_id(argument, indices) {
                        return true;
                    }
                }

                if let Some(Expr::Lit(literal)) = arguments.first()
                    && let syn::Lit::Str(format) = &literal.lit
                {
                    return format_captures(&format.value(), &captures);
                }
            }

            false
        }
        // Data lookups (rows[index].id), calls and blocks may produce stable keys.
        // Do not infer their result from index arguments without type/data-flow analysis.
        _ => false,
    }
}

fn format_captures(format: &str, indices: &[String]) -> bool {
    let mut characters = format.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '{' {
            continue;
        }

        if characters.peek() == Some(&'{') {
            characters.next();
            continue;
        }

        let name: String = characters
            .by_ref()
            .take_while(|c| !matches!(c, ':' | '}'))
            .collect();

        if indices.contains(&name) {
            return true;
        }
    }

    false
}
