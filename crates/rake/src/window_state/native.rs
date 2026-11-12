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
// Native transitions normally complete sooner; a rejected request must not freeze tracking.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(2);

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
    decoration_height: f32,
    // Zoom animations can report windowed frames; keep the last settled normal bounds.
    candidate: Option<Rect>,
    pending_mode: Option<Mode>,
    _settle: Option<gpui::Task<()>>,
    _restore: Option<gpui::Task<()>>,
    _bounds: Subscription,
    _display: Subscription,
}

impl Observer {
    fn capture(&mut self, window: &Window, cx: &mut gpui::Context<Self>, settle: bool) {
        let actual_mode = if window.is_fullscreen() {
            Mode::Fullscreen
        } else if window.is_maximized() {
            Mode::Maximized
        } else {
            Mode::Windowed
        };

        if self.pending_mode == Some(actual_mode) {
            self.pending_mode = None;
        }

        let mode = self.pending_mode.unwrap_or(actual_mode);

        if mode == Mode::Windowed {
            let bounds = rect(window.window_bounds().get_bounds());

            // The pinned macOS adapter reports frame bounds, but WindowOptions
            // opens a content rect. Persist client height to avoid titlebar growth.
            #[cfg(target_os = "macos")]
            let bounds = {
                let client_height = f32::from(window.viewport_size().height);

                if !bounds.valid() || !client_height.is_finite() || client_height <= 0. {
                    return;
                }

                let decoration_height = (bounds.height - client_height).max(0.);

                if decoration_height > 1024. {
                    return;
                }

                self.decoration_height = decoration_height;

                Rect {
                    height: client_height,
                    ..bounds
                }
            };

            if bounds.valid() {
                self.candidate = Some(bounds);

                if settle {
                    let executor = cx.background_executor().clone();

                    self._settle = Some(cx.spawn(async move |observer, cx| {
                        executor.timer(SAVE_INTERVAL).await;
                        let _ = observer.update(cx, |observer, _| {
                            if let Some(bounds) = observer.candidate.take() {
                                observer.normal = bounds;
                            }
                        });
                    }));
                }
            }
        } else {
            if mode == Mode::Fullscreen {
                if let WindowBounds::Fullscreen(bounds) = window.window_bounds() {
                    let bounds = rect(bounds);

                    self.normal = Rect {
                        height: bounds.height - self.decoration_height,
                        ..bounds
                    };
                } else if let Some(bounds) = self.candidate {
                    self.normal = bounds;
                }
            }

            #[cfg(target_os = "windows")]
            if let WindowBounds::Maximized(bounds) = window.window_bounds() {
                self.normal = rect(bounds);
            }

            self.candidate = None;
            self._settle = None;
        }

        let record = SavedWindow {
            bounds: if mode == Mode::Windowed {
                self.candidate.unwrap_or(self.normal)
            } else {
                self.normal
            },
            display: window
                .display(cx)
                .and_then(|display| display.uuid().ok())
                .map(|uuid| uuid.to_string()),
            mode,
            decoration_height: self.decoration_height,
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
                observer.update(cx, |observer, cx| observer.capture(window, cx, false))
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
    mode: Option<Mode>,
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
            mode: requested_mode(options),
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
        mode: requested_mode(options),
        normal: options
            .window_bounds
            .map(|bounds| rect(bounds.get_bounds())),
    })
}

pub(crate) fn observe(placement: Option<Prepared>, window: &mut Window, cx: &mut gpui::App) {
    let Some(Prepared { key, normal, mode }) = placement else {
        return;
    };

    let driver = cx.global::<Driver>();
    let policy = driver.policy.clone();
    let signals = driver.signals.clone();
    let supplied_normal = normal;
    let normal = normal.unwrap_or_else(|| rect(window.window_bounds().get_bounds()));

    #[cfg(target_os = "macos")]
    let decoration_height = if window.is_fullscreen() {
        (f32::from(window.window_bounds().get_bounds().size.height) - normal.height).max(0.)
    } else {
        (f32::from(window.bounds().size.height) - f32::from(window.viewport_size().height)).max(0.)
    };

    #[cfg(not(target_os = "macos"))]
    let decoration_height = 0.;

    let normal = if supplied_normal.is_none() {
        Rect {
            height: normal.height - decoration_height,
            ..normal
        }
    } else {
        normal
    };

    let observer = cx.new(|cx: &mut gpui::Context<Observer>| Observer {
        policy,
        signals,
        key: key.clone(),
        normal,
        decoration_height,
        candidate: None,
        pending_mode: mode,
        _settle: None,
        _restore: None,
        _bounds: cx.observe_window_bounds(window, |observer, window, cx| {
            observer.capture(window, cx, true)
        }),
        _display: window.observe_window_display({
            let observer = cx.weak_entity();

            move |window, cx| {
                let _ = observer.update(cx, |observer, cx| observer.capture(window, cx, true));
            }
        }),
    });

    observer.update(cx, |observer, cx| {
        observer.capture(window, cx, true);

        if observer.pending_mode.is_some() {
            let handle = window.window_handle();
            let executor = cx.background_executor().clone();

            observer._restore = Some(cx.spawn(async move |observer, cx| {
                // A compositor can reject a mode request. Do not keep ignoring
                // windowed geometry after its native transition has had time to run.
                executor.timer(RESTORE_TIMEOUT).await;
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = observer.update(cx, |observer, cx| {
                        if observer.pending_mode.take().is_some() {
                            observer.capture(window, cx, true);
                        }
                    });
                });
            }));
        }
    });
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
        observer.update(cx, |observer, cx| observer.capture(window, cx, true));
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

fn requested_mode(options: &WindowOptions) -> Option<Mode> {
    match options.window_bounds {
        Some(WindowBounds::Maximized(_)) => Some(Mode::Maximized),
        Some(WindowBounds::Fullscreen(_)) => Some(Mode::Fullscreen),
        _ => None,
    }
}
