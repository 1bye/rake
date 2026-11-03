//! Covers: native actions, selection validation, clipping, remount and window teardown.
//! Real: AppKit controls, Objective-C action dispatch, GPUI and its macOS renderer.
//! Doubles: none.
//! Requires: macOS graphical session and a working GPU; `native-tests` feature.

#[cfg(target_os = "macos")]
mod suite {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use gpui::{self, AppContext, Context, IntoElement, Render, Window, WindowOptions, prelude::*};

    use rake_native::macos::{
        Button, ButtonStyle, NativeError, NativeView, SegmentedControl,
        appkit::{
            NSAccessibility, NSAccessibilityButtonRole, NSAccessibilityToolbarRole, NSView,
            NSWindow,
        },
        foundation::{NSPoint, NSString},
        objc2::{
            MainThreadMarker, msg_send,
            rc::Retained,
            runtime::{AnyClass, AnyObject},
        },
    };

    fn has_toolbar(window: &NSWindow) -> bool {
        window.accessibilityChildren().unwrap().iter().any(|child| {
            child.downcast_ref::<NSView>().is_some_and(|view| {
                // SAFETY: This is AppKit's static toolbar-role constant.
                view.accessibilityRole().as_deref() == Some(unsafe { NSAccessibilityToolbarRole })
            })
        })
    }

    struct Fixture {
        stage: u8,
        scheduled: Option<u8>,
        button: Button,
        glass: Button,
        segments: SegmentedControl,
        custom: NativeView,
        raw: Retained<NSView>,
        destination: Option<gpui::AnyWindowHandle>,
        finished: Rc<Cell<bool>>,
    }

    impl Fixture {
        fn new(cx: &mut Context<Self>, finished: Rc<Cell<bool>>) -> Self {
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
                stage: 0,
                scheduled: None,
                button,
                glass: Button::new("Glass", ButtonStyle::LiquidGlass).unwrap(),
                segments,
                custom,
                raw,
                destination: None,
                finished,
            }
        }
    }

    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let stage = self.stage;

            if self.scheduled != Some(stage) {
                self.scheduled = Some(stage);
                let button = self.button.clone();
                let glass = self.glass.clone();
                let segments = self.segments.clone();
                let raw = self.raw.clone();
                let finished = self.finished.clone();
                let entity = cx.weak_entity();
                let destination = self.destination;

                window.on_next_frame(move |window, cx| match stage {
                    0 => {
                        assert!(button.as_appkit().window().is_some());
                        let native_window = button.as_appkit().window().unwrap();
                        // SAFETY: The live button is retained and mounted by the fixture.
                        let native_group = unsafe { button.as_appkit().superview() }.unwrap();

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
                        let destination = cx.open_window(
                            WindowOptions { focus: false, ..WindowOptions::default() },
                            |_, cx| cx.new(|_| Destination(moved)),
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
                            cx.notify();
                        }).unwrap();
                    }
                    7 => {
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
                        destination.unwrap().update(cx, |_, window, _| window.remove_window()).unwrap();

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

    pub(super) fn run() {
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
            cx.set_quit_mode(gpui::QuitMode::Explicit);
            cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|cx| Fixture::new(cx, completed))
            })
            .unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
                panic!("native integration test timed out before completing");
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
            suite::run();
        } else {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--appkit-child")
                .output()
                .unwrap();

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            assert!(
                output.status.success() && stdout.contains("RAKE_NATIVE_TEST_COMPLETE"),
                "native test did not complete: {}\n{stdout}\n{stderr}",
                output.status
            );
            println!("AppKit actions, clipping, remount and window teardown passed");
        }
    }
    #[cfg(not(target_os = "macos"))]
    panic!("the native integration suite requires macOS");
}
