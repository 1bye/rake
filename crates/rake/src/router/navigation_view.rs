//! Shared presentation for operation-keyed navigation progress and recovery.
use gpui::{IntoElement, RenderOnce, Window, div, prelude::*, rgb};

use super::{
    BlockReason, CancelReason, NavigationState, Navigator, OperationId, Route, Router, WindowId,
};

use crate::inspection::InspectElement as _;

/// Built-in loading, cancellation and retry controls for one window.
///
/// `App::routes` presents this outside its layout by default. Custom router hosts
/// can render it alongside their outlet and notify their owning view on router
/// revisions. This projection keeps only a weak navigator.
#[derive(IntoElement)]
pub struct NavigationBoundary<R: Route> {
    navigator: Navigator<R>,
    window: WindowId,
}

impl<R: Route> NavigationBoundary<R> {
    /// Creates feedback for the router's initial window.
    pub fn new(router: Router<R>) -> Self {
        let navigator = router.navigator();
        let window = navigator.window_id();

        Self { navigator, window }
    }

    /// Limits presentation to this concrete host.
    #[must_use]
    pub fn in_window(mut self, window: WindowId) -> Self {
        self.window = window;

        self
    }
}

fn button(
    label: &'static str,
    operation: OperationId,
    action: impl Fn() + 'static,
) -> gpui::AnyElement {
    // GPUI owns stable focus and Enter/Space activation for focusable click targets.
    div()
        .id(format!("{label}-{operation}"))
        .role(gpui::accesskit::Role::Button)
        .aria_label(label)
        .focusable()
        .tab_index(0)
        .px_2()
        .py_1()
        .border_1()
        .border_color(rgb(0x29424b))
        .focus(|style| style.border_color(rgb(0xf0f4e8)))
        .cursor_pointer()
        .child(label)
        .on_click(move |_, _, _| action())
        .inspect_with(|| format!("rake.navigation.{label}.{operation}"))
        .into_any_element()
}

fn tab_navigation(event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut gpui::App) {
    let mut modifiers = event.keystroke.modifiers;

    modifiers.shift = false;

    if event.keystroke.key == "tab" && !modifiers.modified() && !window.default_prevented() {
        if event.keystroke.modifiers.shift {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }

        window.prevent_default();
        cx.stop_propagation();
    }
}

impl<R: Route> RenderOnce for NavigationBoundary<R> {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let Ok(router) = self.navigator.runtime() else {
            return div().into_any_element();
        };

        let has_page = self.navigator.for_window(self.window).current_tab().is_ok();
        let shutting_down = router.quit_status().phase.blocks_navigation();

        let mut feedback = div()
            .id("rake.navigation.feedback")
            .tab_group()
            .on_key_down(tab_navigation)
            .flex()
            .flex_col()
            .gap_2();

        let statuses = router.navigation(self.window);

        for status in &statuses {
            // A retry presents its own state. Hide earlier attempts in that
            // chain without suppressing unrelated navigation failures.
            if statuses
                .iter()
                .any(|child| child.retry_of.as_ref() == Some(&status.key))
            {
                continue;
            }

            let pending = matches!(status.state, NavigationState::Pending);

            let message = match &status.state {
                NavigationState::Pending => "Opening…".to_owned(),
                NavigationState::Failed(error) => error.to_string(),
                NavigationState::Blocked(BlockReason::Guard(reason)) => reason.0.clone(),
                NavigationState::Blocked(BlockReason::HistoryCapacity) => {
                    "History limit reached. Go back or close a tab to continue.".into()
                }
                NavigationState::Blocked(BlockReason::HistoryBoundary) => {
                    "No page in that direction.".into()
                }
                NavigationState::Cancelled(CancelReason::Superseded)
                | NavigationState::Committed => continue,
                NavigationState::Cancelled(_) if has_page && status.retry_of.is_none() => continue,
                NavigationState::Cancelled(_) => "Opening cancelled.".into(),
            };

            let destination = R::decode(&status.key.destination)
                .and_then(|destination| destination.to_uri())
                .unwrap_or_else(|_| status.key.destination.route.clone());

            let action = self.navigator.clone();
            let key = status.key.clone();

            let mut row = div()
                .id(format!("navigation-{}", key.operation_id))
                .role(if pending {
                    gpui::accesskit::Role::Status
                } else {
                    gpui::accesskit::Role::Alert
                })
                .flex()
                .flex_wrap()
                .gap_3()
                .p_3()
                .bg(rgb(0x29424b))
                .text_color(rgb(0xf0f4e8))
                .child(format!("{destination}: {message}"));

            if pending {
                row = row.child(button("Cancel", key.operation_id, move || {
                    if let Ok(router) = action.runtime() {
                        let _ = router.cancel(key.operation_id);
                    }
                }));
            } else {
                if !shutting_down {
                    let retry = key.clone();

                    row = row.child(button("Retry", key.operation_id, move || {
                        if let Ok(router) = action.runtime()
                            && let Err(error) = router.retry_navigation(&retry)
                        {
                            crate::warn!(%error, "navigation retry was rejected");
                        }
                    }));
                }

                // An initial failure keeps its retry control until a page opens.
                if has_page {
                    let dismiss = self.navigator.clone();

                    row = row.child(button("Dismiss", key.operation_id, move || {
                        if let Ok(router) = dismiss.runtime() {
                            let _ = router.dismiss_navigation(&key);
                        }
                    }));
                }
            }

            feedback = feedback.child(row);
        }

        feedback.inspect("rake.navigation").into_any_element()
    }
}
