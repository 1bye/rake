//! Covers: native actions, symbols, opacity, clipping, live accessibility, window moves and closes.
//! Real: AppKit controls, Base Root, Objective-C dispatch, GPUI and its macOS renderer.
//! Doubles: none.
//! Requires: macOS graphical session and a working GPU; `native-tests` feature.

#[cfg(all(target_os = "macos", feature = "gpui-fast"))]
extern crate gpui_fast as gpui;
#[cfg(all(target_os = "macos", feature = "gpui-fast"))]
extern crate gpui_fast_platform as gpui_platform;

#[cfg(target_os = "macos")]
mod suite {
    use std::{cell::Cell, mem, rc::Rc, time::Duration};

    use gpui::{self, AppContext, Context, IntoElement, Render, Window, WindowOptions, prelude::*};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use rake_native::macos::{
        Button, ButtonStyle, NativeError, NativeView, SegmentedControl,
        appkit::{
            NSAccessibility, NSAccessibilityButtonRole, NSAccessibilityToolbarRole, NSView,
            NSWindow,
        },
        foundation::{NSArray, NSObject, NSObjectProtocol, NSPoint, NSString},
        objc2::{
            MainThreadMarker, MainThreadOnly, define_class, ffi, msg_send,
            rc::Retained,
            runtime::{AnyClass, AnyObject, Imp, Sel},
            sel,
        },
    };

    define_class!(
        // SAFETY: NSObject has no subclassing obligations; the observer has no state.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[name = "RakeNativeTestWindowObserver"]
        struct WindowObserver;

        // SAFETY: NSObjectProtocol introduces no additional invariants.
        unsafe impl NSObjectProtocol for WindowObserver {}

        impl WindowObserver {
            // SAFETY: Matches Foundation's KVO callback ABI; notifications are ignored.
            #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
            fn observe(
                &self,
                _key: Option<&NSString>,
                _object: Option<&AnyObject>,
                _change: Option<&AnyObject>,
                _context: *mut std::ffi::c_void,
            ) {}
        }
    );

    struct WindowObservation {
        window: Retained<NSWindow>,
        observer: Retained<WindowObserver>,
        real_class: &'static AnyClass,
    }

    impl WindowObservation {
        fn new(window: &Window) -> Self {
            let handle = HasWindowHandle::window_handle(window).unwrap();
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                panic!("native test requires an AppKit window");
            };
            // SAFETY: GPUI supplies its live main-thread content view.
            let content = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
            let window = content.window().unwrap();
            // SAFETY: NSObject's class message returns the class hidden by KVO.
            let real_class: &'static AnyClass = unsafe { msg_send![&*window, class] };
            // SAFETY: Initialize a main-thread NSObject subclass with no ivars.
            let observer: Retained<WindowObserver> = unsafe {
                msg_send![
                    WindowObserver::alloc(MainThreadMarker::new().unwrap()),
                    init
                ]
            };
            // SAFETY: Both objects are retained until the matching removal in Drop.
            // Options zero avoids an initial notification; the callback accepts title changes.
            unsafe {
                let _: () = msg_send![&*window, addObserver: &*observer,
                    forKeyPath: &*NSString::from_str("title"), options: 0_usize,
                    context: std::ptr::null_mut::<std::ffi::c_void>()];
            }
            assert_ne!(
                window.class(),
                real_class,
                "KVO changes the runtime class before mounting"
            );

            Self {
                window,
                observer,
                real_class,
            }
        }
    }

    impl Drop for WindowObservation {
        fn drop(&mut self) {
            // SAFETY: Remove exactly the observation installed by new, on the main thread.
            unsafe {
                let _: () = msg_send![&*self.window, removeObserver: &*self.observer,
                    forKeyPath: &*NSString::from_str("title")];
            }
        }
    }

    thread_local! {
        static PROGRESS: Cell<(u8, &'static str)> = const { Cell::new((0, "starting")) };
        static CHILDREN_FORWARDING: Cell<bool> = const { Cell::new(false) };
        static HIT_FORWARDING: Cell<bool> = const { Cell::new(false) };
        static HIT_REENTERED: Cell<bool> = const { Cell::new(false) };
        static BASE_HIT: Cell<Option<unsafe extern "C-unwind" fn(&NSWindow, Sel, NSPoint) -> *mut AnyObject>> = const { Cell::new(None) };
    }

    unsafe extern "C-unwind" fn children_via_super(window: &NSWindow, _: Sel) -> *mut NSArray {
        // Bound a broken hook's recursion so the assertion reports duplicate hosts.
        if CHILDREN_FORWARDING.replace(true) {
            return std::ptr::null_mut();
        }

        // SAFETY: Start super dispatch above this live window's KVO subclass.
        let result = unsafe {
            msg_send![
                super(window, window.class().superclass().unwrap()),
                accessibilityChildren
            ]
        };
        CHILDREN_FORWARDING.set(false);

        result
    }

    unsafe extern "C-unwind" fn hit_via_super(
        window: &NSWindow,
        selector: Sel,
        point: NSPoint,
    ) -> *mut AnyObject {
        // Bound the same recursion for a canvas hit that reaches the fallback.
        if HIT_FORWARDING.replace(true) {
            HIT_REENTERED.set(true);
            return std::ptr::null_mut();
        }

        // SAFETY: The superclass hook has the same screen-point/object-return ABI.
        let _: *mut AnyObject = unsafe {
            msg_send![super(window, window.class().superclass().unwrap()), accessibilityHitTest: point]
        };
        // SAFETY: Preserve Base's saved forwarder after exercising super dispatch.
        let result = unsafe { BASE_HIT.get().unwrap()(window, selector, point) };
        HIT_FORWARDING.set(false);

        result
    }

    fn install_super_forwarders(observation: &WindowObservation) {
        let class = observation.window.class();

        for (selector, replacement) in [
            (
                sel!(accessibilityChildren),
                // SAFETY: Erase only the getter's receiver/selector/NSArray ABI.
                unsafe {
                    mem::transmute::<unsafe extern "C-unwind" fn(&NSWindow, Sel) -> *mut NSArray, Imp>(
                        children_via_super,
                    )
                },
            ),
            (
                sel!(accessibilityHitTest:),
                // SAFETY: Erase only the receiver/selector/NSPoint/object ABI.
                unsafe {
                    mem::transmute::<
                        unsafe extern "C-unwind" fn(&NSWindow, Sel, NSPoint) -> *mut AnyObject,
                        Imp,
                    >(hit_via_super)
                },
            ),
        ] {
            let method = class.instance_method(selector).unwrap();
            if selector == sel!(accessibilityHitTest:) {
                // SAFETY: Base's installed forwarder uses the selector's exact ABI.
                BASE_HIT.set(Some(unsafe {
                    mem::transmute::<
                        Imp,
                        unsafe extern "C-unwind" fn(&NSWindow, Sel, NSPoint) -> *mut AnyObject,
                    >(method.implementation())
                }));
            }
            // SAFETY: Test-only KVO overrides have the selectors' original ABIs.
            unsafe {
                ffi::class_replaceMethod(
                    std::ptr::from_ref(class).cast_mut(),
                    selector,
                    replacement,
                    ffi::method_getTypeEncoding(method),
                );
            }
        }
    }

    fn has_toolbar(window: &NSWindow) -> bool {
        window.accessibilityChildren().unwrap().iter().any(|child| {
            child.downcast_ref::<NSView>().is_some_and(|view| {
                // SAFETY: This is AppKit's static toolbar-role constant.
                view.accessibilityRole().as_deref() == Some(unsafe { NSAccessibilityToolbarRole })
            })
        })
    }

    fn assert_native_hit(native_window: &NSWindow, button: &Button) {
        // Base installs NSWindow accessibilityHitTest: while Rake
        // composes accessibilityChildren. Both must reach AppKit controls.
        let button_view: &NSView = button.as_appkit();
        let center = NSPoint::new(60.0, 16.0);
        let point_in_window = button_view.convertPoint_toView(center, None);
        let point_on_screen = native_window.convertPointToScreen(point_in_window);
        // SAFETY: NSAccessibility hit testing takes a screen NSPoint
        // and returns an autoreleased accessibility object.
        let hit: Option<Retained<AnyObject>> =
            unsafe { msg_send![native_window, accessibilityHitTest: point_on_screen] };
        // SAFETY: Ask the same live AppKit control for its own
        // representation (NSButtonCell for this button style).
        let expected: Option<Retained<AnyObject>> =
            unsafe { msg_send![button.as_appkit(), accessibilityHitTest: point_on_screen] };
        assert!(
            hit.as_ref()
                .zip(expected.as_ref())
                .is_some_and(|(hit, expected)| Retained::as_ptr(hit) == Retained::as_ptr(expected)),
            "Base Root hit testing reaches AppKit's button representation: {hit:?}, expected {expected:?}"
        );
    }

    fn assert_canvas_hit(window: &NSWindow) {
        let content = window.contentView().unwrap();
        let bounds = content.bounds();
        let point = NSPoint::new(bounds.size.width - 8.0, bounds.size.height - 8.0);
        let point = window.convertPointToScreen(content.convertPoint_toView(point, None));
        // SAFETY: Both live objects support accessibility hit testing in screen coordinates.
        let (hit, expected): (Option<Retained<AnyObject>>, Option<Retained<AnyObject>>) = unsafe {
            (
                msg_send![window, accessibilityHitTest: point],
                msg_send![&*content, accessibilityHitTest: point],
            )
        };

        assert!(
            hit.as_ref()
                .zip(expected.as_ref())
                .is_some_and(|(hit, expected)| Retained::as_ptr(hit) == Retained::as_ptr(expected)),
            "window hit testing preserves Base's GPUI canvas forwarder: {hit:?}, expected {expected:?}"
        );
    }

    #[derive(Clone)]
    struct AccessibilityState {
        content: Retained<NSView>,
        was_element: bool,
        role: Option<Retained<NSString>>,
    }

    struct Fixture {
        observation: Option<WindowObservation>,
        stage: u8,
        scheduled: Option<u8>,
        button: Button,
        glass: Button,
        segments: SegmentedControl,
        custom: NativeView,
        raw: Retained<NSView>,
        destination: Option<gpui::AnyWindowHandle>,
        destination_accessibility: Option<AccessibilityState>,
        finished: Rc<Cell<bool>>,
    }

    impl Fixture {
        fn new(
            cx: &mut Context<Self>,
            finished: Rc<Cell<bool>>,
            observation: WindowObservation,
        ) -> Self {
            assert!(matches!(
                SegmentedControl::new(&[]),
                Err(NativeError::EmptySegments)
            ));
            assert!(matches!(
                Button::symbol("rake.invalid.symbol", "Missing", ButtonStyle::Push),
                Err(NativeError::SymbolUnavailable(_))
            ));
            let symbol = Button::symbol("plus", "Add", ButtonStyle::Push).unwrap();
            let image = symbol
                .as_appkit()
                .image()
                .expect("valid SF Symbol image is set");

            assert_eq!(image.accessibilityDescription().unwrap().to_string(), "Add");
            assert_eq!(
                symbol.as_appkit().accessibilityLabel().unwrap().to_string(),
                "Add"
            );
            assert_eq!(symbol.as_appkit().toolTip().unwrap().to_string(), "Add");
            let button = Button::new("Click", ButtonStyle::Push).unwrap();
            let entity = cx.weak_entity();

            button.on_click(cx, move |cx| {
                entity
                    .update(cx, |this, cx| {
                        assert_eq!(this.stage, 0, "disabled button must not dispatch an action");
                        this.stage = 1;
                        cx.notify();
                    })
                    .unwrap();
            });
            let segments = SegmentedControl::new(&["One", "Two", "Three"]).unwrap();

            assert_eq!(segments.selected(), Some(0));
            assert!(matches!(
                segments.set_selected(Some(3)),
                Err(NativeError::InvalidSegment { index: 3, count: 3 })
            ));
            assert_eq!(segments.selected(), Some(0));
            segments.set_selected(None).unwrap();
            assert_eq!(segments.selected(), None);
            segments.set_selected(Some(0)).unwrap();
            let entity = cx.weak_entity();

            segments.on_change(cx, move |index, cx| {
                assert_eq!(index, 2);
                entity
                    .update(cx, |this, cx| {
                        assert_eq!(this.stage, 1);
                        this.stage = 2;
                        cx.notify();
                    })
                    .unwrap();
            });
            let raw = NSView::new(MainThreadMarker::new().unwrap());
            let custom = NativeView::from_appkit(raw.clone()).unwrap();

            Self {
                observation: Some(observation),
                stage: 0,
                scheduled: None,
                button,
                glass: Button::new("Glass", ButtonStyle::LiquidGlass).unwrap(),
                segments,
                custom,
                raw,
                destination: None,
                destination_accessibility: None,
                finished,
            }
        }
    }

    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let stage = self.stage;
            PROGRESS.set((stage, "render"));

            if self.scheduled != Some(stage) {
                self.scheduled = Some(stage);
                let observation = self.observation.take();
                let button = self.button.clone();
                let glass = self.glass.clone();
                let segments = self.segments.clone();
                let raw = self.raw.clone();
                let finished = self.finished.clone();
                let entity = cx.weak_entity();
                let destination = self.destination;
                let destination_accessibility = self.destination_accessibility.clone();

                window.on_next_frame(move |window, cx| match stage {
                    0 => {
                        assert!(button.as_appkit().window().is_some());
                        let native_window = button.as_appkit().window().unwrap();
                        // SAFETY: The live button is retained and mounted by the fixture.
                        let native_group = unsafe { button.as_appkit().superview() }.unwrap();

                        assert!(native_window.accessibilityChildren().unwrap().iter().any(|child| {
                            child.downcast_ref::<NSView>()
                                .is_some_and(|view| std::ptr::eq(view, &*native_group))
                        }), "native group is discoverable while KVO is active");
                        let observation = observation.expect("KVO is active during the first mount");

                        // AppKit can retain its own observers after ours is removed.
                        // Verify the real class has the hook even if the KVO class stays active.
                        for selector in [sel!(accessibilityChildren), sel!(accessibilityHitTest:)] {
                            assert!(std::ptr::fn_addr_eq(
                                observation.real_class.instance_method(selector).expect("stable class has accessibility hook").implementation(),
                                native_window.class().instance_method(selector).unwrap().implementation(),
                            ), "the stable window class must own {selector} composition");
                        }
                        assert_native_hit(&native_window, &button);
                        assert_canvas_hit(&native_window);
                        assert!(!HIT_REENTERED.get(), "super dispatch must not reenter the subclass original");
                        drop(observation);

                        assert!(native_group.isAccessibilityElement());
                        let window_children = native_window.accessibilityChildren().unwrap();
                        let gpui_group = native_window.contentView().unwrap();

                        assert!(gpui_group.isAccessibilityElement());
                        assert!(
                            window_children.iter().any(|child| {
                                child
                                    .downcast_ref::<NSView>()
                                    .is_some_and(|view| std::ptr::eq(view, &*gpui_group))
                            }),
                            "the live GPUI subtree is preserved"
                        );

                        assert!(
                            window_children.iter().any(|child| {
                                child
                                    .downcast_ref::<NSView>()
                                    .is_some_and(|view| std::ptr::eq(view, &*native_group))
                            }),
                            "native group remains discoverable alongside the live GPUI group"
                        );
                        assert_eq!(
                            window_children.iter().filter(|child| {
                                child.downcast_ref::<NSView>()
                                    .is_some_and(|view| std::ptr::eq(view, &*native_group))
                            }).count(),
                            1,
                            "super forwarding must compose each native host exactly once"
                        );
                        let group_children = native_group.accessibilityChildren().unwrap();

                        assert!(
                            group_children.iter().any(|child| {
                                // SAFETY: AppKit returns accessibility elements here;
                                // role follows the NSAccessibility NSString contract.
                                let role: Option<Retained<NSString>> =
                                    unsafe { msg_send![&*child, accessibilityRole] };
                                // SAFETY: This is AppKit's static button-role constant.
                                role.as_deref() == Some(unsafe { NSAccessibilityButtonRole })
                            }),
                            "AppKit exposes the native button through its group: {group_children:?}"
                        );
                        assert_native_hit(&native_window, &button);
                        assert_canvas_hit(&native_window);
                        assert!(glass.as_appkit().window().is_some());
                        assert_eq!(button.as_appkit().frame().size.width, 120.0);
                        assert_eq!(raw.frame().size.height, 64.0);
                        // SAFETY: raw is a live main-thread view retained by this fixture.
                        let clip = unsafe { raw.superview() }.unwrap();

                        assert_eq!(clip.frame().size.height, 32.0);
                        button.disable();
                        // SAFETY: This retained button has the adapter's valid
                        // target/action; a null sender is accepted by AppKit.
                        unsafe { button.as_appkit().performClick(None) };
                        button.enable();
                        // performClick pumps AppKit events; dispatch outside GPUI's
                        // borrowed App so the queued handler can update it safely.
                        cx.spawn(async move |_| {
                            // SAFETY: The same live target/action is still installed.
                            unsafe { button.as_appkit().performClick(None) };
                        })
                        .detach();
                    }
                    1 => {
                        assert!(button.as_appkit().window().is_none());
                        assert!(glass.as_appkit().window().is_none());
                        assert!(raw.window().is_none());
                        segments.set_selected(Some(2)).unwrap();
                        let control = segments.as_appkit();

                        // SAFETY: This is the action and live target installed by
                        // SegmentedControl; the retained control owns their lifetime.
                        unsafe {
                            assert!(
                                control
                                    .sendAction_to(control.action(), control.target().as_deref())
                            );
                        }
                    }
                    2 => {
                        assert!(
                            button.as_appkit().window().is_some(),
                            "the same control remounts"
                        );
                        assert_eq!(segments.selected(), Some(2));
                        entity
                            .update(cx, |this, cx| {
                                this.stage = 3;
                                cx.notify();
                            })
                            .unwrap();
                    }
                    3 | 4 => {
                        // SAFETY: The fixture retains this mounted main-thread button.
                        let group = unsafe { button.as_appkit().superview() }.unwrap();
                        let frame = group.frame();
                        let center = NSPoint::new(
                            frame.origin.x + frame.size.width / 2.0,
                            frame.origin.y + frame.size.height / 2.0,
                        );

                        assert_eq!(group.alphaValue(), if stage == 3 { 0.0 } else { 0.5 });
                        assert_eq!(group.isAccessibilityHidden(), stage == 3);
                        assert_eq!(
                            group.hitTest(center).is_some(),
                            stage == 4,
                            "zero opacity prevents hit testing; positive opacity restores it"
                        );
                        assert_eq!(group.isHidden(), stage == 3);

                        if stage == 3 {
                            entity
                                .update(cx, |this, cx| {
                                    this.stage = 4;
                                    cx.notify();
                                })
                                .unwrap();
                            return;
                        }

                        let native_window = button.as_appkit().window().unwrap();

                        assert!(!has_toolbar(&native_window));
                        // SAFETY: NSToolbar and its standard print item are created
                        // on the main thread; NSWindow retains the installed toolbar.
                        unsafe {
                            let toolbar: Retained<AnyObject> = msg_send![AnyClass::get(c"NSToolbar").unwrap(), new];
                            let _: () = msg_send![&*toolbar, insertItemWithItemIdentifier: &*NSString::from_str("NSToolbarPrintItem"), atIndex: 0_isize];
                            let _: () = msg_send![&*native_window, setToolbar: &*toolbar];
                        }

                        assert!(has_toolbar(&native_window), "a toolbar added after mounting is immediately discoverable");
                        let children = native_window.accessibilityChildren().unwrap();
                        let content = native_window.contentView().unwrap();

                        for expected in [&*content, &*group] {
                            assert!(
                                children.iter().any(|child| child.downcast_ref::<NSView>()
                                    .is_some_and(|view| std::ptr::eq(view, expected))),
                                "GPUI and native groups stay discoverable with the toolbar"
                            );
                        }

                        entity.update(cx, |this, cx| {
                            this.stage = 5;
                            cx.notify();
                        }).unwrap();
                    }
                    5 => {
                        let native_window = button.as_appkit().window().unwrap();

                        assert!(has_toolbar(&native_window), "toolbar survives native placement updates");
                        // SAFETY: Clear the live window's toolbar on the main thread.
                        unsafe { let _: () = msg_send![&*native_window, setToolbar: None::<&AnyObject>]; }

                        assert!(!has_toolbar(&native_window), "removed toolbar leaves the accessibility tree immediately");
                        entity.update(cx, |this, cx| {
                            this.stage = 6;
                            cx.notify();
                        }).unwrap();
                    }
                    6 => {
                        assert!(!has_toolbar(&button.as_appkit().window().unwrap()));
                        let source_window = button.as_appkit().window().unwrap();
                        // SAFETY: The fixture retains this mounted main-thread button.
                        let group = unsafe { button.as_appkit().superview() }.unwrap();

                        // Change the next layout without scheduling it yet, so the old
                        // Attachment survives while the shared control mounts elsewhere.
                        entity.update(cx, |this, _| this.stage = 7).unwrap();
                        let moved = button.clone();
                        let mut accessibility = None;
                        let destination = cx.open_window(
                            WindowOptions { focus: false, ..WindowOptions::default() },
                            |window, cx| {
                                let handle = HasWindowHandle::window_handle(window).unwrap();
                                let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                                    panic!("native test requires an AppKit window");
                                };
                                // SAFETY: GPUI supplies its live main-thread content view.
                                let content = unsafe {
                                    Retained::retain(handle.ns_view.as_ptr().cast::<NSView>())
                                }.unwrap();

                                accessibility = Some(AccessibilityState {
                                    was_element: content.isAccessibilityElement(),
                                    role: content.accessibilityRole(),
                                    content,
                                });
                                let view = cx.new(|_| Destination(moved));
                                cx.new(|cx| gpui_base::Root::new(view, window, cx))
                            },
                        ).unwrap();

                        assert!(!std::ptr::eq(
                            &*button.as_appkit().window().unwrap(), &*source_window
                        ));
                        assert!(
                            !source_window.accessibilityChildren().unwrap().iter().any(|child| {
                                child.downcast_ref::<NSView>()
                                    .is_some_and(|view| std::ptr::eq(view, &*group))
                            }),
                            "a moved host leaves the source accessibility tree before its next redraw"
                        );
                        entity.update(cx, |this, cx| {
                            this.destination = Some(destination.into());
                            this.destination_accessibility = accessibility;
                            cx.notify();
                        }).unwrap();
                    }
                    7 => {
                        PROGRESS.set((stage, "callback"));
                        assert!(button.as_appkit().window().is_some(),
                            "dropping the source Attachment preserves the destination mount");
                        let native_window = button.as_appkit().window().unwrap();
                        // SAFETY: The destination retains the same main-thread button.
                        let group = unsafe { button.as_appkit().superview() }.unwrap();

                        assert!(native_window.accessibilityChildren().unwrap().iter().any(|child| {
                            child.downcast_ref::<NSView>()
                                .is_some_and(|view| std::ptr::eq(view, &*group))
                        }), "the destination keeps the moved host discoverable");
                        assert!(group.accessibilityParent().is_some_and(|parent| {
                            Retained::as_ptr(&parent) == Retained::as_ptr(&native_window).cast()
                        }), "dropping the source Attachment preserves the new accessibility parent");
                        assert!(raw.window().is_none());
                        // Close from outside GPUI's borrowed update, as AppKit does
                        // for a title-bar close button. The source window drives stage 8.
                        cx.spawn(async move |cx| {
                            let destination = destination.unwrap();
                            let (closed, close_event) = futures::channel::oneshot::channel();
                            let mut closed = Some(closed);
                            let subscription = cx.update(|cx| cx.on_window_closed(move |_, id| {
                                if id == destination.window_id() {
                                    closed.take().expect("one close event")
                                        .send(()).expect("close receiver is alive");
                                }
                            }));

                            PROGRESS.set((stage, "performClose"));
                            native_window.performClose(None);
                            PROGRESS.set((stage, "closed"));

                            assert!(!native_window.isVisible(), "performClose closes the window");
                            assert!(button.as_appkit().window().is_none(),
                                "a user close detaches a retained control");
                            assert!(group.accessibilityParent().is_none());
                            let original = destination_accessibility.unwrap();

                            assert_eq!(original.content.isAccessibilityElement(), original.was_element);
                            assert_eq!(original.content.accessibilityRole(), original.role);
                            assert!(!native_window.accessibilityChildren().unwrap().iter().any(|child| {
                                child.downcast_ref::<NSView>()
                                    .is_some_and(|view| std::ptr::eq(view, &*group))
                            }), "a user close restores window accessibility traversal");
                            close_event.await.expect("GPUI reports the platform close");
                            drop(subscription);

                            entity.update(cx, |this, cx| {
                                this.stage = 8;
                                cx.notify();
                            }).unwrap();
                        }).detach();
                    }
                    8 => {
                        // The close notification confirms GPUI's foreground cleanup
                        // before the source frame checks registry state and remounting.
                        assert!(!cx.windows().contains(&destination.unwrap()),
                            "AppKit close removes the GPUI window");
                        assert!(button.as_appkit().window().is_some(),
                            "a retained control remounts after a user close");

                        window.remove_window();
                        cx.spawn(async move |cx| {
                            assert!(
                                button.as_appkit().window().is_none(),
                                "window close detaches retained controls"
                            );
                            assert!(raw.window().is_none());
                            finished.set(true);
                            println!("RAKE_NATIVE_TEST_COMPLETE");
                            cx.update(|cx| cx.quit());
                        })
                        .detach();
                    }
                    _ => unreachable!(),
                });
            }

            gpui::div()
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.segments.element().w(gpui::px(280.)).h(gpui::px(32.)))
                .when(stage != 1 && stage != 7, |root| {
                    root.child(
                        self.button
                            .element()
                            .w(gpui::px(120.))
                            .h(gpui::px(32.))
                            .opacity(match stage {
                                3 => 0.0,
                                4 => 0.5,
                                _ => 1.0,
                            }),
                    )
                    .child(self.glass.element().w(gpui::px(120.)).h(gpui::px(32.)))
                    .child(
                        gpui::div()
                            .w(gpui::px(80.))
                            .h(gpui::px(32.))
                            .overflow_hidden()
                            .child(
                                self.custom
                                    .element()
                                    .w(gpui::px(80.))
                                    .h(gpui::px(64.))
                                    .flex_shrink_0(),
                            ),
                    )
                })
        }
    }

    struct Destination(Button);

    impl Render for Destination {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.element().w(gpui::px(120.)).h(gpui::px(32.))
        }
    }

    pub(super) fn run(super_forwarding: bool) {
        assert!(
            std::thread::spawn(|| matches!(
                Button::new("Off thread", ButtonStyle::Push),
                Err(NativeError::NotMainThread)
            ))
            .join()
            .unwrap()
        );
        let finished = Rc::new(Cell::new(false));
        let completed = finished.clone();

        gpui_platform::application().run(move |cx| {
            gpui_base::init(cx);
            cx.set_quit_mode(gpui::QuitMode::Explicit);
            let frames_finished = completed.clone();
            cx.open_window(WindowOptions::default(), |window, cx| {
                let observation = WindowObservation::new(window);
                let view = cx.new(|cx| Fixture::new(cx, completed, observation));
                let root = cx.new(|cx| gpui_base::Root::new(view.clone(), window, cx));

                if super_forwarding {
                    install_super_forwarders(view.read(cx).observation.as_ref().unwrap());
                }

                root
            })
            .unwrap();
            cx.spawn(async move |cx| {
                while !frames_finished.get() {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let views = cx.update(|cx| {
                        cx.windows()
                            .into_iter()
                            .map(|handle| {
                                handle
                                    .update(cx, |_, window, _| {
                                        let handle =
                                            HasWindowHandle::window_handle(window).unwrap();
                                        let RawWindowHandle::AppKit(handle) = handle.as_raw()
                                        else {
                                            panic!("native test requires an AppKit window");
                                        };
                                        // SAFETY: Retain GPUI's live main-thread content view
                                        // until its layer callback has finished.
                                        unsafe {
                                            Retained::retain(
                                                handle.ns_view.as_ptr().cast::<NSView>(),
                                            )
                                        }
                                        .unwrap()
                                    })
                                    .unwrap()
                            })
                            .collect::<Vec<_>>()
                    });

                    // GPUI pauses its display link when AppKit reports occlusion. Request
                    // real frames through the layer delegate, outside the borrowed GPUI App.
                    for view in views {
                        view.layer()
                            .expect("GPUI content has a backing layer")
                            .display();
                    }
                }
            })
            .detach();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
                let (stage, phase) = PROGRESS.get();
                panic!("native integration test timed out at stage {stage} ({phase})");
            })
            .detach();
            cx.activate(true);
        });
        assert!(finished.get(), "all native integration stages completed");
        println!("AppKit actions, clipping, remount and teardown passed");
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    {
        // AppKit may terminate the process instead of returning from its event
        // loop. A parent checks completion independently of the child's exit code.
        if std::env::args().any(|arg| arg == "--appkit-child") {
            suite::run(std::env::args().any(|arg| arg == "--super-forwarding"));
        } else {
            for super_forwarding in [false, true] {
                let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                command.arg("--appkit-child");

                if super_forwarding {
                    command.arg("--super-forwarding");
                }

                let output = command.output().unwrap();

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                assert!(
                    output.status.success() && stdout.contains("RAKE_NATIVE_TEST_COMPLETE"),
                    "native test did not complete (super forwarding: {super_forwarding}): {}\n{stdout}\n{stderr}",
                    output.status
                );
            }
            println!("AppKit actions, clipping, remount and window teardown passed");
        }
    }
    #[cfg(not(target_os = "macos"))]
    panic!("the native integration suite requires macOS");
}
