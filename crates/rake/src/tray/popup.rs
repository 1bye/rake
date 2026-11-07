//! Separate GPUI windows for rich tray content, with explicit placement fallback.
use super::{Error, Feature, Owner};
use gpui::{AnyView, Window};

#[cfg(not(target_family = "wasm"))]
use gpui::{
    AppContext, Bounds, Context, FocusHandle, WindowBounds, WindowOptions, point, prelude::*, px,
    size,
};

#[cfg(not(target_family = "wasm"))]
use std::cell::RefCell;
use std::{collections::BTreeMap, rc::Rc};

/// Placement policy when native icon geometry is unavailable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopupFallback {
    /// Reject display when the host cannot anchor reliably.
    RequireAnchor,
    /// Use an ordinary centered window; works with compositor-owned placement.
    #[default]
    CenteredWindow,
}

/// Content entity lifetime between popup sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopupRetention {
    /// Run the mount factory for each session; keep shared state in services.
    #[default]
    Recreate,
    /// Keep the GPUI entity; content must not own resources tied to its previous window.
    Retain,
}

/// How the last popup invocation was actually positioned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupPlacement {
    /// Current native icon geometry and monitor work area were available.
    Anchored,
    /// An ordinary window with compositor/desktop placement was used.
    Centered,
    /// An open popup was closed by a toggle invocation.
    Dismissed,
}

type Mount = Rc<dyn Fn(&mut Window, &mut gpui::App) -> Result<AnyView, crate::router::MountError>>;

/// Repeatable rich popup content attached to one tray item.
#[derive(Clone)]
pub struct Popup {
    width: f32,
    height: f32,
    fallback: PopupFallback,
    retention: PopupRetention,
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "browser rejects native popup mounting")
    )]
    mount: Mount,
}

impl std::fmt::Debug for Popup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Popup")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fallback", &self.fallback)
            .field("retention", &self.retention)
            .finish_non_exhaustive()
    }
}

impl Popup {
    /// Mounts rich content at a fixed logical size, with centered fallback by default.
    pub fn new(
        width: f32,
        height: f32,
        mount: impl Fn(&mut Window, &mut gpui::App) -> Result<AnyView, crate::router::MountError>
        + 'static,
    ) -> Self {
        Self {
            width,
            height,
            fallback: PopupFallback::CenteredWindow,
            retention: PopupRetention::Recreate,
            mount: Rc::new(mount),
        }
    }

    /// Selects explicit behavior on hosts lacking reliable global tray geometry.
    pub fn fallback(mut self, fallback: PopupFallback) -> Self {
        self.fallback = fallback;

        self
    }

    /// Selects content lifetime; recreated content is safest for native child views.
    pub fn retention(mut self, retention: PopupRetention) -> Self {
        self.retention = retention;

        self
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
        if !self.width.is_finite()
            || !self.height.is_finite()
            || !(1.0..=4096.0).contains(&self.width)
            || !(1.0..=4096.0).contains(&self.height)
        {
            return Err(Error::InvalidInput(
                "popup dimensions must be finite and in [1, 4096] logical points".into(),
            ));
        }

        Ok(())
    }
}

struct Attachment {
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "browser rejects native popup mounting")
    )]
    popup: Popup,
    content: Option<AnyView>,
    window: Option<gpui::AnyWindowHandle>,
    generation: Option<u64>,
}

#[derive(Default)]
pub(super) struct Popups {
    items: BTreeMap<String, Attachment>,
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "browser rejects native popup mounting")
    )]
    opening: bool,
}

impl Popups {
    pub(super) fn new(specs: Vec<(String, Popup)>) -> Result<Self, Error> {
        let mut items = BTreeMap::new();

        for (id, popup) in specs {
            popup.validate()?;

            if items
                .insert(
                    id.clone(),
                    Attachment {
                        popup,
                        content: None,
                        window: None,
                        generation: None,
                    },
                )
                .is_some()
            {
                return Err(Error::Duplicate(id));
            }
        }

        Ok(Self {
            items,
            opening: false,
        })
    }
}

/// Shows or toggles one attached popup. Only one tray popup is open at a time.
/// # Errors
/// Reports missing attachment/item, stopped owner, unsupported anchoring or failed mount.
pub fn toggle_popup(id: &str, cx: &mut gpui::App) -> Result<PopupPlacement, Error> {
    #[cfg(target_family = "wasm")]
    {
        let _ = (id, cx);

        Err(Error::Unsupported(Feature::Popup))
    }

    #[cfg(not(target_family = "wasm"))]
    {
        let owner = cx.try_global::<Owner>().ok_or(Error::HostUnavailable)?;
        let handle = owner.host.collection().get(id)?;
        let generation = handle.state()?.generation;

        let attached = owner
            .popups
            .items
            .get(id)
            .ok_or_else(|| Error::InvalidInput("tray has no popup attachment".into()))?;

        if owner.popups.opening {
            return Err(Error::InvalidInput("popup is already opening".into()));
        }

        let current = attached
            .window
            .filter(|_| attached.generation == Some(generation));

        if let Some(current) = current
            && current.update(cx, |_, _, _| ()).is_ok()
        {
            dismiss_popup(id, cx)?;

            return Ok(PopupPlacement::Dismissed);
        }

        dismiss_all(cx);
        let owner = cx.global::<Owner>();
        let attached = owner.popups.items.get(id).expect("validated attachment");
        let popup = attached.popup.clone();

        let content = if attached.generation == Some(generation) {
            attached.content.clone()
        } else {
            None
        };

        let anchor = handle.popup_anchor()?;

        if anchor.is_none() && popup.fallback == PopupFallback::RequireAnchor {
            return Err(Error::Unsupported(Feature::Rectangle));
        }

        let placement = if anchor.is_some() {
            PopupPlacement::Anchored
        } else {
            PopupPlacement::Centered
        };

        let bounds = anchor
            .as_ref()
            .map(|anchor| position(anchor, popup.width, popup.height));

        let options = WindowOptions {
            window_bounds: bounds.map(WindowBounds::Windowed),
            display_id: anchor
                .as_ref()
                .map(|anchor| gpui::DisplayId::new(anchor.display_id)),
            kind: if anchor.is_some() {
                gpui::WindowKind::PopUp
            } else {
                gpui::WindowKind::Normal
            },
            titlebar: None,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            focus: true,
            ..Default::default()
        };

        let failure = Rc::new(RefCell::new(None));
        let failed = failure.clone();
        let retained = Rc::new(RefCell::new(None));
        let saved = retained.clone();

        cx.global_mut::<Owner>().popups.opening = true;
        let popup_handle = handle.clone();

        let result = cx.open_window(options, move |window, cx| {
            let content = match content.map(Ok).unwrap_or_else(|| (popup.mount)(window, cx)) {
                Ok(view) => view,
                Err(error) => {
                    *failed.borrow_mut() = Some(error);

                    cx.new(|_| gpui::Empty).into()
                }
            };

            if popup.retention == PopupRetention::Retain {
                *saved.borrow_mut() = Some(content.clone());
            }

            let view = cx.new(|cx| {
                let focus = cx.focus_handle();

                window.focus(&focus, cx);
                let blur = cx.observe_window_activation(window, |_, window, cx| {
                    if !window.is_window_active() {
                        window.remove_window();
                        cx.notify();
                    }
                });

                PopupRoot {
                    content,
                    focus,
                    _blur: blur,
                    handle: popup_handle,
                }
            });

            cx.new(|cx| rake_ui::Root::new(view, window, cx))
        });

        cx.global_mut::<Owner>().popups.opening = false;
        let opened =
            result.map_err(|e| Error::InvalidInput(format!("popup window could not open: {e}")))?;

        if let Some(error) = failure.borrow_mut().take() {
            let _ = opened.update(cx, |_, window, _| window.remove_window());

            return Err(Error::InvalidInput(format!(
                "popup content could not mount: {error}"
            )));
        }

        if let Err(error) = handle.state() {
            let _ = opened.update(cx, |_, window, _| window.remove_window());

            return Err(error);
        }

        #[cfg(target_os = "macos")]
        if let Err(error) = handle.set_popup_highlight(true) {
            let _ = opened.update(cx, |_, window, _| window.remove_window());

            return Err(error);
        }

        let attached = cx
            .global_mut::<Owner>()
            .popups
            .items
            .get_mut(id)
            .expect("validated attachment");

        attached.window = Some(opened.into());
        attached.content = retained.borrow_mut().take();
        attached.generation = Some(generation);
        cx.activate(true);

        Ok(placement)
    }
}

/// Dismisses an attached popup without requesting application quit.
/// # Errors
/// Reports an absent tray host or attachment.
pub fn dismiss_popup(id: &str, cx: &mut gpui::App) -> Result<(), Error> {
    if cx.try_global::<Owner>().is_none() {
        return Err(Error::HostUnavailable);
    }

    let attached = cx
        .global_mut::<Owner>()
        .popups
        .items
        .get_mut(id)
        .ok_or(Error::HostUnavailable)?;

    let window = attached.window.take();

    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }

    Ok(())
}

pub(super) fn dismiss_all(cx: &mut gpui::App) {
    if cx.try_global::<Owner>().is_none() {
        return;
    }

    let windows = cx
        .global_mut::<Owner>()
        .popups
        .items
        .values_mut()
        .filter_map(|item| item.window.take())
        .collect::<Vec<_>>();

    for window in windows {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

pub(super) fn removed(id: &str, generation: u64, cx: &mut gpui::App) {
    if cx
        .global::<Owner>()
        .popups
        .items
        .get(id)
        .is_some_and(|item| item.generation == Some(generation))
    {
        let _ = dismiss_popup(id, cx);

        if let Some(item) = cx.global_mut::<Owner>().popups.items.get_mut(id) {
            item.content = None;
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn position(anchor: &rake_tray::PopupAnchor, width: f32, height: f32) -> Bounds<gpui::Pixels> {
    let icon = &anchor.icon;
    let work = &anchor.work_area;
    let width = f64::from(width).min(work.width);
    let height = f64::from(height).min(work.height);
    let below = icon.origin.y + icon.height + 4.0;
    let above = icon.origin.y - height - 4.0;

    let y = if below + height <= work.origin.y + work.height {
        below
    } else {
        above
    };

    let (x, y) = if icon.origin.x + icon.width <= work.origin.x {
        (
            icon.origin.x + icon.width + 4.0,
            icon.origin.y + icon.height / 2.0 - height / 2.0,
        )
    } else if icon.origin.x >= work.origin.x + work.width {
        (
            icon.origin.x - width - 4.0,
            icon.origin.y + icon.height / 2.0 - height / 2.0,
        )
    } else {
        (icon.origin.x + icon.width / 2.0 - width / 2.0, y)
    };

    let x = x.clamp(work.origin.x, work.origin.x + work.width - width);
    let y = y.clamp(work.origin.y, work.origin.y + work.height - height);

    Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(width as f32), px(height as f32)),
    )
}

#[cfg(not(target_family = "wasm"))]
struct PopupRoot {
    content: AnyView,
    focus: FocusHandle,
    _blur: gpui::Subscription,
    handle: super::Handle,
}

#[cfg(not(target_family = "wasm"))]
impl Drop for PopupRoot {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        let _ = self.handle.set_popup_highlight(false);

        #[cfg(not(target_os = "macos"))]
        let _ = &self.handle;
    }
}

#[cfg(not(target_family = "wasm"))]
impl Render for PopupRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                    cx.stop_propagation();
                }
            })
            .child(self.content.clone())
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    fn placement_clamps_negative_origins_and_bottom_panels() {
        let anchor = rake_tray::PopupAnchor {
            display_id: 1,
            icon: super::super::Rect {
                origin: super::super::Point { x: -10.0, y: 740.0 },
                width: 20.0,
                height: 20.0,
            },
            work_area: super::super::Rect {
                origin: super::super::Point { x: -800.0, y: 0.0 },
                width: 800.0,
                height: 740.0,
            },
        };

        let bounds = position(&anchor, 300.0, 400.0);

        assert_eq!(bounds.origin.x, px(-300.0));
        assert_eq!(bounds.origin.y, px(336.0));
        let huge = position(&anchor, 4000.0, 4000.0);

        assert_eq!(huge.size, size(px(800.0), px(740.0)));
    }
}
