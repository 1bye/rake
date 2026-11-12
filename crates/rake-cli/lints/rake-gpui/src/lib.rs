//! Type-aware checks for the GPUI release pinned by Rake.
#![feature(rustc_private)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use rustc_errors::{Diag, DiagDecorator};
use rustc_hir::{PatKind, Stmt, StmtKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;

dylint_linting::dylint_library!();

declare_lint! {
    /// A GPUI task dropped immediately cancels its work.
    pub GPUI_DROPPED_TASK, Warn, "GPUI task is immediately dropped"
}

struct Gpui;

impl_lint_pass!(Gpui => [GPUI_DROPPED_TASK]);

#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    store.register_lints(&[GPUI_DROPPED_TASK]);
    store.register_late_lint_pass(Box::new(|_| Box::new(Gpui)));
}

fn gpui_type(cx: &LateContext<'_>, ty: Ty<'_>, name: &str) -> bool {
    let ty::Adt(def, _) = ty.peel_refs().kind() else {
        return false;
    };
    let id = def.did();
    let krate = cx.tcx.crate_name(id.krate);
    let path = cx.tcx.def_path_str(id);

    // gpui-pre 0.3.11 re-exports scheduler::Task (gpui-pre-scheduler).
    matches!(krate.as_str(), "gpui" | "scheduler") && path.rsplit("::").next() == Some(name)
}

fn emit(cx: &LateContext<'_>, lint: &'static rustc_session::lint::Lint, span: Span, message: &str) {
    cx.opt_span_lint(
        lint,
        Some(span),
        DiagDecorator(|diag: &mut Diag<'_, ()>| {
            diag.primary_message(message.to_owned());
        }),
    );
}

impl<'tcx> LateLintPass<'tcx> for Gpui {
    fn check_stmt(&mut self, cx: &LateContext<'tcx>, stmt: &'tcx Stmt<'tcx>) {
        let expr = match stmt.kind {
            StmtKind::Semi(expr) => expr,
            StmtKind::Let(local) if matches!(local.pat.kind, PatKind::Wild) => {
                let Some(expr) = local.init else { return };

                expr
            }
            _ => return,
        };

        if gpui_type(cx, cx.typeck_results().expr_ty(expr), "Task") {
            emit(
                cx,
                GPUI_DROPPED_TASK,
                expr.span,
                "store the GPUI Task or detach it; dropping it cancels the work",
            );
        }
    }
}
