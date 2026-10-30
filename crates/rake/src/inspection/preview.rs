//! Native preview bindings exist only in opted-in macOS development hosts.

use super::{Element, InspectionError, Store, preview as wire};

impl Store {
    pub(crate) fn preview_enabled(&self) -> bool {
        #[cfg(all(feature = "gpui", target_os = "macos"))]
        return self
            .0
            .borrow()
            .windows
            .values()
            .any(|host| host.preview.is_some());

        #[cfg(not(all(feature = "gpui", target_os = "macos")))]
        false
    }

    pub(crate) fn preview_windows(&self) -> Vec<wire::Window> {
        #[cfg(all(feature = "gpui", target_os = "macos"))]
        {
            let targets: Vec<_> = self
                .0
                .borrow()
                .windows
                .iter()
                .filter_map(|(id, host)| {
                    Some((*id, host.snapshot.capture.clone(), host.preview.clone()?))
                })
                .collect();

            targets
                .into_iter()
                .filter_map(|(id, generation, mut target)| {
                    target
                        .window
                        .update(&mut target.app, |_, window, _| {
                            native::describe(id, generation, window)
                        })
                        .ok()
                        .flatten()
                })
                .collect()
        }

        #[cfg(not(all(feature = "gpui", target_os = "macos")))]
        Vec::new()
    }

    pub(crate) fn preview_element(&self, point: &wire::Point) -> Result<Element, InspectionError> {
        if !self
            .preview_windows()
            .iter()
            .any(|window| valid_point(window, point))
        {
            return Err(InspectionError::StaleGeneration);
        }

        let state = self.0.borrow();

        let host = state
            .windows
            .get(&point.window)
            .ok_or(InspectionError::NotRetained)?;

        #[cfg(feature = "gpui")]
        return super::hit_test(&host.snapshot, point.x as f32, point.y as f32)
            .cloned()
            .ok_or(InspectionError::NotRetained);

        #[cfg(not(feature = "gpui"))]
        {
            let _ = host;

            Err(InspectionError::Unavailable)
        }
    }

    pub(crate) fn preview_input(
        &self,
        point: &wire::Point,
        input: &wire::Input,
    ) -> Result<(), InspectionError> {
        if !self
            .preview_windows()
            .iter()
            .any(|window| valid_input(window, point))
        {
            return Err(InspectionError::StaleGeneration);
        }

        if let wire::Input::Scroll { dx, dy } = input
            && (dx.unsigned_abs() > 2000 || dy.unsigned_abs() > 2000)
        {
            return Err(InspectionError::NavigationUnavailable);
        }

        if let wire::Input::Wheel { dx, dy, .. } = input
            && (dx.unsigned_abs() > 128000 || dy.unsigned_abs() > 128000)
        {
            return Err(InspectionError::NavigationUnavailable);
        }

        #[cfg(all(feature = "gpui", target_os = "macos"))]
        {
            let mut target = self
                .0
                .borrow()
                .windows
                .get(&point.window)
                .and_then(|host| host.preview.clone())
                .ok_or(InspectionError::Unavailable)?;

            target
                .window
                .update(&mut target.app, |_, window, cx| {
                    use gpui::{
                        MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
                        ScrollDelta, ScrollWheelEvent,
                    };
                    let position = gpui::point(gpui::px(point.x as f32), gpui::px(point.y as f32));

                    window.dispatch_event(
                        PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            ..Default::default()
                        }),
                        cx,
                    );

                    match input {
                        wire::Input::Move => {}
                        wire::Input::Click => {
                            window.dispatch_event(
                                PlatformInput::MouseDown(MouseDownEvent {
                                    button: MouseButton::Left,
                                    position,
                                    click_count: 1,
                                    ..Default::default()
                                }),
                                cx,
                            );
                            window.dispatch_event(
                                PlatformInput::MouseUp(MouseUpEvent {
                                    button: MouseButton::Left,
                                    position,
                                    click_count: 1,
                                    ..Default::default()
                                }),
                                cx,
                            );
                        }
                        wire::Input::Wheel { dx, dy, phase } => {
                            let touch_phase = match phase {
                                wire::WheelPhase::Started => gpui::TouchPhase::Started,
                                wire::WheelPhase::Moved => gpui::TouchPhase::Moved,
                                wire::WheelPhase::Ended => gpui::TouchPhase::Ended,
                            };

                            window.dispatch_event(
                                PlatformInput::ScrollWheel(ScrollWheelEvent {
                                    position,
                                    delta: ScrollDelta::Pixels(gpui::point(
                                        gpui::px(-(*dx as f32) / 64.),
                                        gpui::px(-(*dy as f32) / 64.),
                                    )),
                                    touch_phase,
                                    ..Default::default()
                                }),
                                cx,
                            );
                        }
                        wire::Input::Scroll { dx, dy } => {
                            window.dispatch_event(
                                PlatformInput::ScrollWheel(ScrollWheelEvent {
                                    position,
                                    delta: ScrollDelta::Pixels(gpui::point(
                                        gpui::px(-*dx as f32),
                                        gpui::px(-*dy as f32),
                                    )),
                                    ..Default::default()
                                }),
                                cx,
                            );
                        }
                    }

                    // Injected input bypasses the platform's normal frame scheduling path.
                    // Inactive preview windows must publish the resulting state as well.
                    window.refresh();
                    native::request_frame(window, cx);
                })
                .map_err(|_| InspectionError::NotRetained)?;

            Ok(())
        }

        #[cfg(not(all(feature = "gpui", target_os = "macos")))]
        Err(InspectionError::Unavailable)
    }
}

fn geometry(window: &wire::Window) -> [i32; 6] {
    [
        window.frame.width,
        window.frame.height,
        window.content.x,
        window.content.y,
        window.content.width,
        window.content.height,
    ]
    .map(|v| (v * 64.).round() as i32)
}

fn valid_input(window: &wire::Window, point: &wire::Point) -> bool {
    if let Some(expected) = point.geometry {
        expected == geometry(window)
            && valid_point(
                window,
                &wire::Point {
                    generation: window.generation.clone(),
                    ..point.clone()
                },
            )
    } else {
        valid_point(window, point)
    }
}

fn valid_point(window: &wire::Window, point: &wire::Point) -> bool {
    window.window == point.window
        && window.native_id == point.native_id
        && window.generation == point.generation
        && point.x >= 0
        && point.y >= 0
        && (point.x as f32) < window.content.width
        && (point.y as f32) < window.content.height
}

#[cfg(all(feature = "gpui", target_os = "macos"))]
pub(super) mod native {
    use super::{Store, wire};
    use crate::{inspection::Layout, router::WindowId};
    use gpui::{AnyWindowHandle, AsyncApp, Window};
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[derive(Clone)]
    pub(in crate::inspection) struct Target {
        pub window: AnyWindowHandle,
        pub app: AsyncApp,
    }

    impl Store {
        pub(in crate::inspection) fn bind_preview(
            &self,
            id: WindowId,
            window: &Window,
            cx: &gpui::App,
        ) {
            if std::env::var_os("RAKE_DEV_PREVIEW").as_deref() != Some(std::ffi::OsStr::new("1")) {
                return;
            }

            if self
                .0
                .borrow()
                .windows
                .get(&id)
                .is_some_and(|host| host.preview.is_none())
            {
                keep_on_stage(window);
            }

            if let Some(host) = self.0.borrow_mut().windows.get_mut(&id) {
                host.preview = Some(Target {
                    window: window.window_handle(),
                    app: cx.to_async(),
                });
            }
        }
    }

    pub(super) fn request_frame(window: &Window, cx: &gpui::App) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };

        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };

        // SAFETY: GPUI owns this borrowed NSView, and this call runs on its UI thread.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };

        if view.window().is_none_or(|native| {
            native
                .occlusionState()
                .contains(objc2_app_kit::NSWindowOcclusionState::Visible)
        }) {
            return;
        }

        if let Some(layer) = view.layer() {
            // GPUI pauses its display link when occluded. Run the layer delegate after
            // this App update ends, since it re-enters GPUI to draw and present.
            cx.foreground_executor()
                .spawn(async move {
                    layer.setNeedsDisplay();
                    layer.displayIfNeeded();
                })
                .detach();
        }
    }

    fn keep_on_stage(window: &Window) {
        use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };

        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };

        // SAFETY: the borrowed GPUI window owns this NSView on its UI thread.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };

        if let Some(native) = view.window() {
            // Stage Manager otherwise captures the inactive window's transformed thumbnail.
            let behavior = native.collectionBehavior() & !(Behavior::Primary | Behavior::Auxiliary);

            native.setCollectionBehavior(behavior | Behavior::CanJoinAllApplications);
        }
    }

    pub(super) fn describe(
        id: WindowId,
        generation: String,
        window: &Window,
    ) -> Option<wire::Window> {
        let handle = HasWindowHandle::window_handle(window).ok()?;

        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };

        // SAFETY: GPUI owns the NSView for this borrowed window, on its UI thread.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let native = view.window()?;

        if !native.isVisible() || native.isMiniaturized() {
            return None;
        }

        let frame = native.frame();
        let content = native.convertRectToScreen(view.convertRect_toView(view.bounds(), None));

        let result = wire::Window {
            window: id,
            native_id: native.windowNumber().try_into().ok()?,
            generation,
            frame: Layout {
                x: 0.,
                y: 0.,
                width: frame.size.width as f32,
                height: frame.size.height as f32,
            },
            content: Layout {
                x: (content.origin.x - frame.origin.x) as f32,
                y: (frame.origin.y + frame.size.height - content.origin.y - content.size.height)
                    as f32,
                width: content.size.width as f32,
                height: content.size.height as f32,
            },
            occluded: !native
                .occlusionState()
                .contains(objc2_app_kit::NSWindowOcclusionState::Visible),
            translucent: !native.isOpaque(),
        };

        [
            result.frame.width,
            result.frame.height,
            result.content.width,
            result.content.height,
        ]
        .iter()
        .all(|v| v.is_finite() && *v > 0.)
        .then_some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_rejects_stale_windows_generations_and_invalid_coordinates() {
        let window = wire::Window {
            window: crate::router::WindowId(1),
            native_id: 123,
            generation: "capture-1".into(),
            frame: super::super::Layout {
                x: 0.,
                y: 0.,
                width: 300.,
                height: 220.,
            },
            content: super::super::Layout {
                x: 0.,
                y: 20.,
                width: 300.,
                height: 200.,
            },
            occluded: false,
            translucent: true,
        };

        let point = wire::Point {
            window: window.window,
            native_id: 123,
            generation: "capture-1".into(),
            x: 20,
            y: 30,
            geometry: None,
        };

        assert!(valid_point(&window, &point));
        let live = wire::Point {
            generation: "older-tree".into(),
            geometry: Some(geometry(&window)),
            ..point.clone()
        };

        assert!(valid_input(&window, &live));
        assert!(!valid_point(&window, &live));
        let resized = wire::Window {
            content: super::super::Layout {
                width: 301.,
                ..window.content
            },
            ..window.clone()
        };

        assert!(!valid_input(&resized, &live));

        for invalid in [
            wire::Point {
                x: -1,
                ..point.clone()
            },
            wire::Point {
                y: 200,
                ..point.clone()
            },
            wire::Point {
                native_id: 124,
                ..point.clone()
            },
            wire::Point {
                generation: "old".into(),
                ..point.clone()
            },
        ] {
            assert!(!valid_point(&window, &invalid));
        }
    }
}
