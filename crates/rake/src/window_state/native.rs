//! GPUI observers and owned, coalesced preference writes for native windows.

use std::{cell::Cell, collections::HashMap, rc::Rc, time::Duration};

use gpui::{AppContext, Entity, Subscription, Window, WindowBounds, WindowOptions};

use super::{
    WindowState,
    placement::{self, Display, Mode, Rect, SavedWindow},
};
use crate::router::WindowId;

/// Coalesce events without postponing persistence indefinitely during a drag.
const SAVE_INTERVAL: Duration = Duration::from_millis(250);

struct Signals {
    dirty: Cell<bool>,
    final_requested: Cell<bool>,
    stopping: Cell<bool>,
    changed: event_listener::Event,
    final_save: event_listener::Event,
}

struct Driver {
    policy: WindowState,
    signals: Rc<Signals>,
    windows: HashMap<gpui::WindowId, (String, Entity<Observer>, gpui::AnyWindowHandle)>,
    _worker: gpui::Task<()>,
    _closed: Subscription,
    _quit: Subscription,
}

impl gpui::Global for Driver {}

struct Observer {
    policy: WindowState,
    signals: Rc<Signals>,
    key: String,
    normal: Rect,
    _bounds: Subscription,
    _display: Subscription,
}

impl Observer {
    fn capture(&mut self, window: &Window, cx: &gpui::App) {
        let mode = if window.is_fullscreen() {
            Mode::Fullscreen
        } else if window.is_maximized() {
            Mode::Maximized
        } else {
            Mode::Windowed
        };

        if mode == Mode::Windowed {
            let bounds = rect(window.window_bounds().get_bounds());

            if bounds.valid() {
                self.normal = bounds;
            }
        }

        let record = SavedWindow {
            bounds: self.normal,
            display: window
                .display(cx)
                .and_then(|display| display.uuid().ok())
                .map(|uuid| uuid.to_string()),
            mode,
        };

        if self.policy.inner.records.borrow().get(&self.key) != Some(&record) {
            self.policy
                .inner
                .records
                .borrow_mut()
                .insert(self.key.clone(), record);
            self.signals.dirty.set(true);
            self.signals.changed.notify(usize::MAX);
        }
    }
}

pub(crate) fn install(policy: WindowState, cx: &mut gpui::App) {
    let signals = Rc::new(Signals {
        dirty: Cell::new(false),
        final_requested: Cell::new(false),
        stopping: Cell::new(false),
        changed: event_listener::Event::new(),
        final_save: event_listener::Event::new(),
    });
    let writes = signals.clone();
    let saving = policy.clone();
    let executor = cx.background_executor().clone();

    let worker = cx.spawn(async move |_| {
        loop {
            let changed = writes.changed.listen();
            let final_save = writes.final_save.listen();

            if !writes.dirty.get() {
                changed.await;
            }

            if writes.stopping.get() {
                break;
            }

            if !writes.final_requested.replace(false) {
                futures::future::select(
                    Box::pin(executor.timer(SAVE_INTERVAL)),
                    Box::pin(final_save),
                )
                .await;
            }

            writes.final_requested.set(false);

            if writes.stopping.get() {
                break;
            }

            writes.dirty.set(false);
            saving.save().await;
        }
    });

    let closed = cx.on_window_closed(|cx, id| {
        let driver = cx.global_mut::<Driver>();

        if driver.windows.remove(&id).is_some() {
            driver.signals.dirty.set(true);
            driver.signals.final_requested.set(true);
            driver.signals.final_save.notify(usize::MAX);
            driver.signals.changed.notify(usize::MAX);
        }
    });

    let quit = cx.on_app_quit(|cx| {
        let windows: Vec<_> = cx
            .global::<Driver>()
            .windows
            .values()
            .map(|(_, observer, handle)| (observer.clone(), *handle))
            .collect();

        for (observer, handle) in windows {
            let _ = handle.update(cx, |_, window, cx| {
                observer.update(cx, |observer, cx| observer.capture(window, cx))
            });
        }

        let driver = cx.global::<Driver>();
        driver.signals.stopping.set(true);
        driver.signals.changed.notify(usize::MAX);
        driver.signals.final_save.notify(usize::MAX);
        let policy = driver.policy.clone();

        async move { policy.save().await }
    });

    cx.set_global(Driver {
        policy,
        signals,
        windows: HashMap::new(),
        _worker: worker,
        _closed: closed,
        _quit: quit,
    });
}

/// Resolve once per opening; a duplicate logical key never shares live geometry.
pub(crate) struct Prepared {
    key: String,
    normal: Option<Rect>,
}

pub(crate) fn prepare(
    id: WindowId,
    options: &mut WindowOptions,
    cx: &gpui::App,
) -> Option<Prepared> {
    let driver = cx.try_global::<Driver>()?;
    let key = (driver.policy.keys)(id)?;

    if !placement::valid_key(&key)
        || driver
            .windows
            .values()
            .any(|(existing, _, _)| existing == &key)
    {
        crate::warn!("invalid or duplicate window placement key; window opted out");

        return None;
    }

    let records = driver.policy.inner.records.borrow();
    let Some(saved) = records.get(&key) else {
        return Some(Prepared {
            key,
            normal: options
                .window_bounds
                .map(|bounds| rect(bounds.get_bounds())),
        });
    };

    let displays = cx.displays();
    let default_id = options
        .display_id
        .or_else(|| cx.primary_display().map(|display| display.id()));
    let default_display = displays
        .iter()
        .position(|display| Some(display.id()) == default_id)
        .unwrap_or(0);
    let default_bounds = options
        .window_bounds
        .map(|bounds| bounds.get_bounds())
        .or_else(|| {
            displays
                .get(default_display)
                .map(|display| display.default_bounds())
        });
    let snapshots: Vec<_> = displays
        .iter()
        .map(|display| Display {
            uuid: display.uuid().ok().map(|uuid| uuid.to_string()),
            visible: rect(display.visible_bounds()),
        })
        .collect();

    if let Some(default_bounds) = default_bounds
        && let Some((index, bounds)) =
            placement::restore(saved, &snapshots, default_display, rect(default_bounds))
    {
        let bounds = bounds_to_gpui(bounds);

        options.display_id = Some(displays[index].id());
        options.window_bounds = Some(match saved.mode {
            Mode::Windowed => WindowBounds::Windowed(bounds),
            Mode::Maximized => WindowBounds::Maximized(bounds),
            Mode::Fullscreen => WindowBounds::Fullscreen(bounds),
        });
    }

    Some(Prepared {
        key,
        normal: options
            .window_bounds
            .map(|bounds| rect(bounds.get_bounds())),
    })
}

pub(crate) fn observe(placement: Option<Prepared>, window: &mut Window, cx: &mut gpui::App) {
    let Some(Prepared { key, normal }) = placement else {
        return;
    };

    let driver = cx.global::<Driver>();
    let policy = driver.policy.clone();
    let signals = driver.signals.clone();
    let normal = normal.unwrap_or_else(|| rect(window.window_bounds().get_bounds()));
    let observer = cx.new(|cx: &mut gpui::Context<Observer>| Observer {
        policy,
        signals,
        key: key.clone(),
        normal,
        _bounds: cx
            .observe_window_bounds(window, |observer, window, cx| observer.capture(window, cx)),
        _display: window.observe_window_display({
            let observer = cx.weak_entity();

            move |window, cx| {
                let _ = observer.update(cx, |observer, cx| observer.capture(window, cx));
            }
        }),
    });

    observer.update(cx, |observer, cx| observer.capture(window, cx));
    cx.global_mut::<Driver>().windows.insert(
        window.window_handle().window_id(),
        (key, observer, window.window_handle()),
    );
}

/// Snapshot before a guarded native close; does not replace the host's close hook.
pub(crate) fn capture(window: &Window, cx: &mut gpui::App) {
    let observer = cx
        .try_global::<Driver>()
        .and_then(|driver| driver.windows.get(&window.window_handle().window_id()))
        .map(|(_, observer, _)| observer.clone());

    if let Some(observer) = observer {
        observer.update(cx, |observer, cx| observer.capture(window, cx));
    }
}

fn rect(bounds: gpui::Bounds<gpui::Pixels>) -> Rect {
    Rect {
        x: bounds.origin.x.into(),
        y: bounds.origin.y.into(),
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
    }
}

fn bounds_to_gpui(rect: Rect) -> gpui::Bounds<gpui::Pixels> {
    gpui::Bounds::new(
        gpui::point(gpui::px(rect.x), gpui::px(rect.y)),
        gpui::size(gpui::px(rect.width), gpui::px(rect.height)),
    )
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests;
