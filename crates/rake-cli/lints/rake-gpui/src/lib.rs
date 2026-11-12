//! Type-aware checks for the GPUI release pinned by Rake.
#![feature(rustc_private)]

extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use rustc_errors::{Diag, DiagDecorator};
use rustc_hir::def::Res;
use rustc_hir::{Expr, ExprKind, Node, PatKind, QPath, Stmt, StmtKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;

dylint_linting::dylint_library!();

declare_lint! {
    /// A GPUI task dropped immediately cancels its work.
    pub GPUI_DROPPED_TASK, Warn, "GPUI task is immediately dropped"
}

declare_lint! {
    /// Entity allocation in render loses identity on every frame.
    pub GPUI_RENDER_NEW, Warn, "entity allocated during render"
}

declare_lint! {
    /// Updating a borrowed entity again panics at runtime.
    pub GPUI_REENTRANT_UPDATE, Warn, "entity updated inside its own update"
}

declare_lint! {
    /// Synchronous blocking work stalls the UI thread.
    pub GPUI_UI_BLOCKING, Warn, "blocking operation on the UI thread"
}

struct Gpui;

impl_lint_pass!(Gpui => [GPUI_DROPPED_TASK, GPUI_RENDER_NEW, GPUI_REENTRANT_UPDATE, GPUI_UI_BLOCKING]);

#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    store.register_lints(&[
        GPUI_DROPPED_TASK,
        GPUI_RENDER_NEW,
        GPUI_REENTRANT_UPDATE,
        GPUI_UI_BLOCKING,
    ]);
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

fn gpui_method(cx: &LateContext<'_>, expr: &Expr<'_>, names: &[&str]) -> bool {
    cx.typeck_results()
        .type_dependent_def_id(expr.hir_id)
        .is_some_and(|id| {
            cx.tcx.crate_name(id.krate).as_str() == "gpui"
                && names.contains(&cx.tcx.item_name(id).as_str())
        })
}

fn in_render(cx: &LateContext<'_>, expr: &Expr<'_>) -> bool {
    let owner = cx.tcx.hir_enclosing_body_owner(expr.hir_id);
    let Some(item) = cx.tcx.opt_associated_item(owner.to_def_id()) else {
        return false;
    };

    if item.name().as_str() != "render" {
        return false;
    }

    let Some(impl_id) = cx.tcx.impl_of_assoc(owner.to_def_id()) else {
        return false;
    };

    cx.tcx.impl_opt_trait_ref(impl_id).is_some_and(|reference| {
        let id = reference.skip_binder().def_id;

        cx.tcx.crate_name(id.krate).as_str() == "gpui"
            && matches!(cx.tcx.item_name(id).as_str(), "Render" | "RenderOnce")
    })
}

fn ui_callback(cx: &LateContext<'_>, expr: &Expr<'_>) -> bool {
    if in_render(cx, expr) {
        return true;
    }

    // Only the nearest closure's direct registration establishes UI execution.
    // Async blocks and arbitrary nested closures do not inherit the UI context.
    let mut closure = None;

    for (_, node) in cx.tcx.hir_parent_iter(expr.hir_id) {
        if let Node::Expr(parent) = node {
            if matches!(parent.kind, ExprKind::Closure(_)) {
                if closure.is_some() {
                    return false;
                }

                closure = Some(parent.hir_id);
            } else if let Some(id) = closure {
                if let ExprKind::MethodCall(_, _, args, _) = parent.kind {
                    return args.iter().any(|arg| arg.hir_id == id)
                        && gpui_method(
                            cx,
                            parent,
                            &[
                                "listener",
                                "on_action",
                                "on_click",
                                "on_mouse_down",
                                "on_mouse_up",
                                "on_key_down",
                                "on_key_up",
                            ],
                        );
                }

                return false;
            }
        }
    }

    false
}

fn local(expr: &Expr<'_>) -> Option<rustc_hir::HirId> {
    if let ExprKind::Path(QPath::Resolved(_, path)) = expr.kind {
        if let Res::Local(id) = path.res {
            return Some(id);
        }
    }

    None
}

fn reentrant(cx: &LateContext<'_>, expr: &Expr<'_>, receiver: &Expr<'_>) -> bool {
    if let ExprKind::MethodCall(_, context, _, _) = receiver.kind {
        if gpui_method(cx, receiver, &["entity"])
            && gpui_type(cx, cx.typeck_results().expr_ty(context), "Context")
        {
            return true;
        }
    }

    let Some(binding) = local(receiver) else {
        return false;
    };

    let mut closure = None;

    for (_, node) in cx.tcx.hir_parent_iter(expr.hir_id) {
        if let Node::Expr(parent) = node {
            if matches!(parent.kind, ExprKind::Closure(_)) {
                closure = Some(parent.hir_id);
                continue;
            }

            if let Some(id) = closure.take() {
                let ExprKind::MethodCall(_, outer, args, _) = parent.kind else {
                    return false;
                };

                // Only a closure passed directly to update runs during that borrow.
                if !args.iter().any(|arg| arg.hir_id == id)
                    || !gpui_method(cx, parent, &["update", "update_in"])
                {
                    return false;
                }

                if gpui_type(cx, cx.typeck_results().expr_ty(outer), "Entity")
                    && local(outer) == Some(binding)
                {
                    return true;
                }
            }
        }
    }

    false
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

        if !cx.typeck_results().expr_ty(expr).is_ref()
            && gpui_type(cx, cx.typeck_results().expr_ty(expr), "Task")
        {
            emit(
                cx,
                GPUI_DROPPED_TASK,
                expr.span,
                "store the GPUI Task or detach it; dropping it cancels the work",
            );
        }
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if let ExprKind::MethodCall(_, receiver, _, _) = expr.kind {
            if gpui_method(cx, expr, &["new"])
                && gpui_type(cx, cx.typeck_results().expr_ty(expr), "Entity")
                && in_render(cx, expr)
            {
                emit(
                    cx,
                    GPUI_RENDER_NEW,
                    expr.span,
                    "create this entity before render and retain its handle",
                );
            }

            if gpui_method(cx, expr, &["update", "update_in"])
                && gpui_type(cx, cx.typeck_results().expr_ty(receiver), "Entity")
                && reentrant(cx, expr, receiver)
            {
                emit(
                    cx,
                    GPUI_REENTRANT_UPDATE,
                    expr.span,
                    "the entity is already borrowed; mutate the supplied state directly",
                );
            }
        }

        if let ExprKind::Call(function, _) = expr.kind {
            if let ty::FnDef(id, _) = cx.typeck_results().expr_ty(function).kind() {
                let path = format!(
                    "{}{}",
                    cx.tcx.crate_name(id.krate),
                    cx.tcx.def_path(*id).to_string_no_crate_verbose()
                );
                let blocking = path == "std::thread::functions::sleep"
                    || path.starts_with("std::fs::")
                    || path == "async_io::driver::block_on"
                    || path == "futures_executor::local_pool::block_on";

                if blocking && ui_callback(cx, expr) {
                    emit(
                        cx,
                        GPUI_UI_BLOCKING,
                        expr.span,
                        "move blocking work to a background task before updating the UI",
                    );
                }
            }
        }
    }
}
