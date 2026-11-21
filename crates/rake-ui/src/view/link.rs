//! One href policy and window-scoped router bridge for native and Web links.

use gpui::{App, Global, SharedString, Window, WindowId};
use std::{collections::HashMap, rc::Rc};

/// Classification shared by mouse, keyboard, native and browser activation.
#[doc(hidden)]
#[derive(Debug, PartialEq, Eq)]
pub enum HrefKind {
    Route,
    External,
    Rejected,
}

/// App paths start with one slash. Only HTTP(S), mailto and tel leave the app.
/// Whitespace, control characters, protocol-relative URLs and all other schemes are rejected.
#[doc(hidden)]
pub fn classify_href(href: &str) -> HrefKind {
    if href.is_empty()
        || href
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return HrefKind::Rejected;
    }

    if href.starts_with('/') && !href.starts_with("//") {
        return HrefKind::Route;
    }

    let Some((scheme, rest)) = href.split_once(':') else {
        return HrefKind::Rejected;
    };

    if rest.is_empty() {
        return HrefKind::Rejected;
    }

    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" if rest.starts_with("//") && rest.len() > 2 => HrefKind::External,
        "mailto" | "tel" => HrefKind::External,
        _ => HrefKind::Rejected,
    }
}

type Navigate = Rc<dyn Fn(&str, &mut App)>;

#[derive(Default)]
struct LinkRoutes(HashMap<WindowId, Navigate>);

impl Global for LinkRoutes {}

/// The first router rendered in a physical window owns its links until close.
#[doc(hidden)]
pub fn install_link_router(
    window: &Window,
    cx: &mut App,
    navigate: impl Fn(&str, &mut App) + 'static,
) {
    let window_id = window.window_handle().window_id();

    if cx
        .try_global::<LinkRoutes>()
        .is_some_and(|routes| routes.0.contains_key(&window_id))
    {
        return;
    }

    if !cx.has_global::<LinkRoutes>() {
        cx.set_global(LinkRoutes::default());
        cx.on_window_closed(|cx, window_id| {
            cx.global_mut::<LinkRoutes>().0.remove(&window_id);
        })
        .detach();
    }

    cx.global_mut::<LinkRoutes>()
        .0
        .insert(window_id, Rc::new(navigate));
}

pub(super) fn activate(href: &SharedString, window: &mut Window, cx: &mut App) {
    match classify_href(href) {
        HrefKind::Route => {
            if let Some(navigate) = cx
                .try_global::<LinkRoutes>()
                .and_then(|routes| routes.0.get(&window.window_handle().window_id()))
                .cloned()
            {
                navigate(href, cx);
            }
        }
        HrefKind::External => cx.open_url(href),
        HrefKind::Rejected => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    #[test]
    fn first_router_owns_each_window_until_close() {
        use gpui::AppContext as _;
        use std::cell::Cell;

        let mut cx = gpui::TestAppContext::single();
        let visits = Rc::new(Cell::new(0));
        let weak_visits = Rc::downgrade(&visits);
        let window = cx.add_window(|_, _| gpui::EmptyView);

        cx.update_window(window.into(), |_, window, cx| {
            let visits = visits.clone();

            install_link_router(window, cx, move |_, _| visits.set(visits.get() + 1));
            install_link_router(window, cx, |_, _| {
                panic!("nested router replaced the owner")
            });
            activate(&"/next".into(), window, cx);
        })
        .unwrap();

        assert_eq!(visits.get(), 1);
        drop(visits);

        cx.update_window(window.into(), |_, window, _| window.remove_window())
            .unwrap();
        assert!(weak_visits.upgrade().is_none());
    }

    #[test]
    fn href_policy_rejects_unsafe_and_ambiguous_destinations() {
        for href in ["/", "/notes/42?q=a#b"] {
            assert_eq!(classify_href(href), HrefKind::Route);
        }

        for href in [
            "https://example.org/",
            "HTTP://example.org",
            "mailto:a@example.org",
            "tel:+123",
        ] {
            assert_eq!(classify_href(href), HrefKind::External);
        }

        for href in [
            "",
            "//evil.org",
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,test",
            "file:///tmp/a",
            " https://example.org",
            "java\nscript:test",
            "/\\evil.org",
            "relative",
            "https:",
        ] {
            assert_eq!(classify_href(href), HrefKind::Rejected, "{href}");
        }
    }
}
