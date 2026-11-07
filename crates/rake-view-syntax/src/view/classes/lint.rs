//! Exact inverse of the zero-argument utilities; no inferred numeric conversions.

use syn::{Expr, Pat, ReturnType, Stmt};

use super::{FIXED, PREFIXES, SCALES};

/// Returns utilities for a pure, untyped identity closure's method chain.
///
/// Only zero-argument methods emitted verbatim by the class compiler are accepted.
/// Calls, captures, typed parameters, statements, variants and conversions are opaque.
pub fn style_classes(expression: &Expr) -> Option<String> {
    let Expr::Closure(closure) = expression else {
        return None;
    };
    if closure.asyncness.is_some()
        || closure.movability.is_some()
        || closure.capture.is_some()
        || closure.constness.is_some()
        || closure.lifetimes.is_some()
        || !closure.attrs.is_empty()
        || !matches!(closure.output, ReturnType::Default)
        || closure.inputs.len() != 1
    {
        return None;
    }
    let Pat::Ident(binding) = &closure.inputs[0] else {
        return None;
    };
    if !binding.attrs.is_empty()
        || binding.by_ref.is_some()
        || binding.mutability.is_some()
        || binding.subpat.is_some()
    {
        return None;
    }
    fn chain(expression: &Expr, binding: &syn::Ident, classes: &mut Vec<String>) -> Option<()> {
        match expression {
            Expr::Path(path) if path.attrs.is_empty() && path.path.is_ident(binding) => Some(()),
            Expr::Block(block) if block.attrs.is_empty() && block.label.is_none() => {
                let [Stmt::Expr(expression, None)] = block.block.stmts.as_slice() else {
                    return None;
                };
                chain(expression, binding, classes)
            }
            Expr::Paren(paren) if paren.attrs.is_empty() => chain(&paren.expr, binding, classes),
            Expr::MethodCall(call)
                if call.attrs.is_empty() && call.args.is_empty() && call.turbofish.is_none() =>
            {
                chain(&call.receiver, binding, classes)?;
                let method = call.method.to_string();
                let utility = FIXED
                    .iter()
                    .find_map(|(utility, candidate)| {
                        (*candidate == method).then(|| (*utility).to_owned())
                    })
                    .or_else(|| {
                        PREFIXES.iter().find_map(|prefix| {
                            SCALES.iter().find_map(|scale| {
                                let utility = format!("{prefix}-{scale}");
                                (utility.replace('-', "_") == method).then_some(utility)
                            })
                        })
                    })?;
                classes.push(utility);
                Some(())
            }
            _ => None,
        }
    }
    let mut classes = Vec::new();
    chain(&closure.body, &binding.ident, &mut classes)?;
    (!classes.is_empty()).then(|| classes.join(" "))
}

#[cfg(test)]
mod tests {
    use proc_macro2::Span;
    use quote::quote;

    use super::*;
    use crate::view::classes::utility_methods;

    #[test]
    fn every_inverse_emits_the_identical_method_call() {
        let methods =
            FIXED
                .iter()
                .map(|(_, method)| (*method).to_owned())
                .chain(PREFIXES.iter().flat_map(|prefix| {
                    SCALES
                        .iter()
                        .map(move |scale| format!("{}_{scale}", prefix.replace('-', "_")))
                }));
        for method in methods {
            let expression: Expr = syn::parse_str(&format!("|s| s.{method}()")).unwrap();
            let utility = style_classes(&expression).unwrap();
            let emitted = utility_methods(&utility, Span::call_site(), &quote!(::rake)).unwrap();
            assert_eq!(emitted.to_string(), format!(". {method} ()"), "{utility}");
        }
    }
}
