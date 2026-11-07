//! Narrow theme evidence and literal design values; aliases and data flow stay opaque.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, Lit};

fn path_is(path: &syn::Path, parts: &[&str]) -> bool {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .eq(parts.iter().copied())
}

pub(super) fn installs_theme(text: &str) -> bool {
    struct Installer(bool);

    impl<'ast> Visit<'ast> for Installer {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let Expr::Path(path) = &*call.func
                && path_is(&path.path, &["rake", "theme", "install"])
                && call.args.len() == 2
            {
                self.0 = true;
            }

            visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if call.method == "theme" && call.args.len() == 1 && rake_app(&call.receiver) {
                self.0 = true;
            }

            visit::visit_expr_method_call(self, call);
        }
    }

    fn rake_app(expression: &Expr) -> bool {
        match expression {
            Expr::MethodCall(call) => rake_app(&call.receiver),
            Expr::Call(call) => {
                matches!(&*call.func, Expr::Path(path) if path_is(&path.path, &["rake", "App", "new"]))
            }
            Expr::Paren(paren) => rake_app(&paren.expr),
            _ => false,
        }
    }
    let mut installer = Installer(false);

    if let Ok(file) = syn::parse_file(text) {
        installer.visit_file(&file);
    }

    installer.0
}

pub(super) fn class_literals(expression: &Expr) -> Vec<Span> {
    struct Classes(Vec<Span>);

    impl<'ast> Visit<'ast> for Classes {
        fn visit_expr_lit(&mut self, literal: &'ast syn::ExprLit) {
            if let Lit::Str(value) = &literal.lit
                && value.value().split_whitespace().any(literal_utility)
            {
                self.0.push(literal.span());
            }
        }

        // Inspect branch results, never conditions, scrutinees or arbitrary calls.
        fn visit_expr(&mut self, expression: &'ast Expr) {
            match expression {
                Expr::Lit(literal) => self.visit_expr_lit(literal),
                Expr::Paren(paren) => self.visit_expr(&paren.expr),
                Expr::Block(block) => {
                    if let [syn::Stmt::Expr(expression, None)] = block.block.stmts.as_slice() {
                        self.visit_expr(expression);
                    }
                }
                Expr::If(branch) => {
                    if let [syn::Stmt::Expr(expression, None)] = branch.then_branch.stmts.as_slice()
                    {
                        self.visit_expr(expression);
                    }

                    if let Some((_, expression)) = &branch.else_branch {
                        self.visit_expr(expression);
                    }
                }
                Expr::Match(branch) => {
                    for arm in &branch.arms {
                        self.visit_expr(&arm.body);
                    }
                }
                _ => {}
            }
        }
    }
    let mut classes = Classes(Vec::new());

    classes.visit_expr(expression);

    classes.0
}

pub(super) fn literal_utility(class: &str) -> bool {
    // Colons inside arbitrary values are not variants.
    let mut utility = class;

    while let Some((prefix, rest)) = utility.split_once(':') {
        if prefix.contains('[') {
            break;
        }

        utility = rest;
    }

    for prefix in ["bg", "text", "border", "rounded"] {
        if let Some(value) = utility
            .strip_prefix(&format!("{prefix}-["))
            .and_then(|v| v.strip_suffix(']'))
        {
            if matches!(prefix, "bg" | "text" | "border") && value.starts_with('#') {
                return true;
            }

            if matches!(prefix, "text" | "rounded")
                && ["px", "rem"].iter().any(|unit| {
                    value
                        .strip_suffix(unit)
                        .is_some_and(|n| n.parse::<f32>().is_ok_and(|n| n.is_finite() && n >= 0.))
                })
            {
                return true;
            }
        }
    }

    false
}

pub(super) fn style_literals(expression: &Expr) -> Vec<Span> {
    fn number(expression: &Expr) -> bool {
        matches!(expression, Expr::Lit(literal) if matches!(literal.lit, Lit::Int(_) | Lit::Float(_)))
            || matches!(expression, Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) && number(&unary.expr))
    }

    fn constructor(expression: &Expr, names: &[&str]) -> bool {
        let Expr::Call(call) = expression else {
            return false;
        };

        let Expr::Path(path) = &*call.func else {
            return false;
        };

        let parts: Vec<_> = path
            .path
            .segments
            .iter()
            .map(|part| part.ident.to_string())
            .collect();

        let Some(name) = parts.last() else {
            return false;
        };

        let known_path = parts.len() == 1
            || (parts.len() == 2 && parts[0] == "gpui")
            || (parts.len() == 3 && parts[0] == "rake" && parts[1] == "gpui");

        let arity = if matches!(name.as_str(), "hsl") {
            3
        } else if name == "hsla" {
            4
        } else {
            1
        };

        known_path
            && names.contains(&name.as_str())
            && call.args.len() == arity
            && call.args.iter().all(number)
    }

    // Only the identity closure chain: nested callbacks, statements and arbitrary
    // receivers may have unrelated methods with the same spelling.
    let Expr::Closure(closure) = expression else {
        return Vec::new();
    };

    if closure.inputs.len() != 1 {
        return Vec::new();
    }

    let syn::Pat::Ident(binding) = &closure.inputs[0] else {
        return Vec::new();
    };

    fn chain(expression: &Expr, binding: &syn::Ident, spans: &mut Vec<Span>) -> bool {
        match expression {
            Expr::Path(path) => path.path.is_ident(binding),
            Expr::Paren(paren) => chain(&paren.expr, binding, spans),
            Expr::Block(block) => {
                if let [syn::Stmt::Expr(expression, None)] = block.block.stmts.as_slice() {
                    chain(expression, binding, spans)
                } else {
                    false
                }
            }
            Expr::MethodCall(call) if chain(&call.receiver, binding, spans) => {
                let color = matches!(
                    call.method.to_string().as_str(),
                    "bg" | "text_color" | "border_color"
                );

                let size = matches!(call.method.to_string().as_str(), "text_size" | "rounded");

                if call.args.len() == 1
                    && ((color && constructor(&call.args[0], &["rgb", "rgba", "hsl", "hsla"]))
                        || (size && constructor(&call.args[0], &["px", "rems"])))
                {
                    spans.push(call.args[0].span());
                }

                true
            }
            _ => false,
        }
    }
    let mut spans = Vec::new();

    if chain(&closure.body, &binding.ident, &mut spans) {
        spans
    } else {
        Vec::new()
    }
}
