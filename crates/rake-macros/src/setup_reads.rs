use std::collections::HashSet;

use syn::visit::{self, Visit};
use syn::{Expr, ExprMethodCall, Ident, Pat, Stmt};

struct Reads {
    signals: HashSet<String>,
    error: Option<syn::Error>,
}

impl<'ast> Visit<'ast> for Reads {
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}

    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if matches!(
            call.method.to_string().as_str(),
            "read" | "get" | "with" | "latest" | "is_loading"
        ) && let Expr::Path(path) = &*call.receiver
            && path
                .path
                .get_ident()
                .is_some_and(|name| self.signals.contains(&name.to_string()))
        {
            let error = syn::Error::new_spanned(
                call,
                "reading a signal in setup is not reactive; move it into view! or cx.memo",
            );
            if let Some(previous) = &mut self.error {
                previous.combine(error);
            } else {
                self.error = Some(error);
            }
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        visit::visit_local(self, local);
        if let Pat::Ident(name) = &local.pat
            && let Some(init) = &local.init
            && let Expr::MethodCall(call) = &*init.expr
            && matches!(
                call.method.to_string().as_str(),
                "signal" | "memo" | "resource"
            )
        {
            self.signals.insert(name.ident.to_string());
        }
    }
}

pub(super) fn validate(statements: &[Stmt], tracked: &[&Ident]) -> syn::Result<()> {
    let mut reads = Reads {
        signals: tracked.iter().map(ToString::to_string).collect(),
        error: None,
    };
    for statement in statements {
        reads.visit_stmt(statement);
    }
    reads.error.map_or(Ok(()), Err)
}
