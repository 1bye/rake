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

type Navigate = Rc<dyn Fn(&str)>;

#[derive(Default)]
struct LinkRoutes(HashMap<WindowId, Navigate>);

impl Global for LinkRoutes {}

/// Installs a weak router navigator for this physical window.
#[doc(hidden)]
pub fn install_link_router(window: &Window, cx: &mut App, navigate: impl Fn(&str) + 'static) {
    cx.default_global::<LinkRoutes>()
        .0
        .insert(window.window_handle().window_id(), Rc::new(navigate));
}

pub(super) fn activate(href: &SharedString, window: &mut Window, cx: &mut App) {
    match classify_href(href) {
        HrefKind::Route => {
            if let Some(navigate) = cx
                .try_global::<LinkRoutes>()
                .and_then(|routes| routes.0.get(&window.window_handle().window_id()))
                .cloned()
            {
                navigate(href);
            }
        }
        HrefKind::External => cx.open_url(href),
        HrefKind::Rejected => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
