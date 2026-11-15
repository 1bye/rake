//! UI dispatch and application lifetime for native shortcut registrations.
#[cfg(feature = "gpui")]
use super::Error;
use super::{Capability, PermissionState, Platform, Request};

/// Global shortcut service. Enable `global-shortcuts` (also enabled by `tray`).
#[derive(Clone, Debug)]
pub struct GlobalShortcuts(std::rc::Rc<std::cell::Cell<bool>>);

impl Default for GlobalShortcuts {
    fn default() -> Self {
        let supported = cfg!(all(
            feature = "global-shortcuts",
            any(target_os = "macos", target_os = "windows")
        ));

        Self(std::rc::Rc::new(std::cell::Cell::new(supported)))
    }
}

impl Platform {
    /// Returns the global shortcut service, including on unsupported hosts.
    pub fn global_shortcuts(&self) -> GlobalShortcuts {
        self.shortcuts.clone()
    }
}

impl GlobalShortcuts {
    pub(super) fn capability(&self) -> Capability {
        Capability {
            supported: self.0.get(),
            permission: if cfg!(target_os = "linux") {
                PermissionState::Unknown
            } else {
                PermissionState::NotRequired
            },
            requires_user_gesture: false,
        }
    }
}

impl GlobalShortcuts {
    /// Queries the current native session, including Wayland portal availability.
    /// Does not register keys or request permission.
    pub fn available(&self) -> Request<bool> {
        #[cfg(feature = "global-shortcuts")]
        let supported = self.0.clone();

        Box::pin(async move {
            #[cfg(feature = "global-shortcuts")]
            {
                supported.set(rake_shortcuts::available().await);

                return Ok(supported.get());
            }

            #[cfg(not(feature = "global-shortcuts"))]
            Ok(false)
        })
    }
}

#[cfg(feature = "gpui")]
pub use ui::GlobalShortcut;

#[cfg(feature = "gpui")]
mod ui {
    use super::*;

    /// An owned shortcut. Drop or `unregister` stops callbacks and releases the key.
    pub struct GlobalShortcut {
        #[cfg(feature = "global-shortcuts")]
        _quit: gpui::Subscription,
        #[cfg(feature = "global-shortcuts")]
        inner: std::rc::Rc<std::cell::RefCell<Registration>>,
    }

    impl GlobalShortcut {
        /// Releases the native registration and awaits its cleanup receipt.
        ///
        /// # Errors
        /// Returns a native cleanup failure; repeat calls are harmless.
        pub fn unregister(&mut self) -> Request<()> {
            #[cfg(feature = "global-shortcuts")]
            return stop(&self.inner);

            #[cfg(not(feature = "global-shortcuts"))]
            Box::pin(async { Ok(()) })
        }
    }

    impl GlobalShortcuts {
        /// Registers a keystroke using the tray accelerator grammar. The callback
        /// runs on the UI thread, including while another application is focused.
        /// Await in owned work; keep the returned handle alive. App quit releases
        /// even retained handles. Dropping the request cancels pending registration.
        ///
        /// # Errors
        /// Reports unsupported hosts, invalid keys, conflicts, permission denial
        /// and native failures. No fallback keystroke is registered.
        pub fn register(
            &self,
            accelerator: &str,
            callback: impl FnMut(&mut gpui::App) + 'static,
            cx: &mut gpui::App,
        ) -> Request<GlobalShortcut> {
            #[cfg(feature = "global-shortcuts")]
            {
                use std::{
                    cell::{Cell, RefCell},
                    rc::Rc,
                };

                let (cancel, cancellation) = futures::channel::oneshot::channel::<()>();
                let mut cancel = Some(cancel);
                let (settle, settled) = futures::channel::oneshot::channel::<()>();
                let mut settled = Some(settled);
                let closed = Rc::new(Cell::new(false));
                let registration = Rc::new(RefCell::new(Registration {
                    shortcut: None,
                    task: None,
                }));
                let weak = Rc::downgrade(&registration);
                let quitting = closed.clone();
                let quit = cx.on_app_quit(move |_| {
                    quitting.set(true);
                    cancel.take();
                    let pending = settled.take();
                    let cleanup = weak.upgrade().map(|inner| stop(&inner));

                    async move {
                        if let Some(pending) = pending {
                            let _ = pending.await;
                        }

                        if let Some(cleanup) = cleanup
                            && let Err(error) = cleanup.await
                        {
                            crate::error!(%error, "global shortcut cleanup failed");
                        }
                    }
                });
                let accelerator = accelerator.to_owned();
                let (send, receive) = futures::channel::oneshot::channel();
                let task = cx.spawn(async move |cx| {
                    let result = async {
                        let mut shortcut = {
                            use futures::FutureExt;

                            let registration =
                                rake_shortcuts::Shortcut::register(&accelerator).fuse();
                            let cancellation = cancellation.fuse();
                            futures::pin_mut!(registration, cancellation);
                            futures::select! {
                                result = registration => result.map_err(error)?,
                                _ = cancellation => return Err(Error::Unavailable {
                                    operation: "global shortcut",
                                    message: "application has quit".into(),
                                }),
                            }
                        };

                        if closed.get() {
                            shortcut.unregister().await.map_err(error)?;

                            return Err(Error::Unavailable {
                                operation: "global shortcut",
                                message: "application has quit".into(),
                            });
                        }

                        let events = shortcut.take_events();
                        let driver = cx.update(|cx| dispatch(events, callback, cx));
                        *registration.borrow_mut() = Registration {
                            shortcut: Some(shortcut),
                            task: Some(driver),
                        };

                        Ok(GlobalShortcut {
                            inner: registration,
                            _quit: quit,
                        })
                    }
                    .await;
                    let _ = send.send(result);
                    let _ = settle.send(());
                });

                Box::pin(async move {
                    let _task = task;
                    let result = receive.await.map_err(|_| Error::Unavailable {
                        operation: "global shortcut",
                        message: "registration task stopped".into(),
                    })?;

                    result
                })
            }

            #[cfg(not(feature = "global-shortcuts"))]
            {
                let _ = (accelerator, callback, cx);

                Box::pin(async { Err(Error::Unsupported("global shortcuts")) })
            }
        }
    }

    #[cfg(feature = "global-shortcuts")]
    struct Registration {
        shortcut: Option<rake_shortcuts::Shortcut>,
        task: Option<gpui::Task<()>>,
    }

    #[cfg(feature = "global-shortcuts")]
    fn dispatch(
        mut events: futures::channel::mpsc::Receiver<()>,
        mut callback: impl FnMut(&mut gpui::App) + 'static,
        cx: &mut gpui::App,
    ) -> gpui::Task<()> {
        cx.spawn(async move |cx| {
            use futures::StreamExt;

            while events.next().await.is_some() {
                cx.update(&mut callback);
            }
        })
    }

    #[cfg(feature = "global-shortcuts")]
    fn stop(inner: &std::cell::RefCell<Registration>) -> Request<()> {
        let mut registration = inner.borrow_mut();
        registration.task.take();
        let shortcut = registration.shortcut.take();

        Box::pin(async move {
            if let Some(mut shortcut) = shortcut {
                shortcut.unregister().await.map_err(error)?;
            }

            Ok(())
        })
    }

    #[cfg(feature = "global-shortcuts")]
    fn error(error: rake_shortcuts::Error) -> Error {
        match error {
            rake_shortcuts::Error::Unsupported => Error::Unsupported("global shortcuts"),
            rake_shortcuts::Error::InvalidInput(message) => Error::InvalidInput(message),
            rake_shortcuts::Error::Conflict => Error::ShortcutConflict,
            rake_shortcuts::Error::PermissionDenied => Error::PermissionDenied("global shortcuts"),
            rake_shortcuts::Error::Native(message) => Error::Unavailable {
                operation: "global shortcut",
                message,
            },
        }
    }

    #[cfg(all(test, feature = "global-shortcuts", feature = "view-test-support"))]
    mod tests {
        //! Covers: UI-thread dispatch and cancellation of queued activations.
        //! Real: GPUI foreground scheduler and registration owner. Doubles: native events.
        //! Requires: no native shortcut registration or graphical session.
        use super::*;
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };

        #[gpui::test]
        fn unregister_stops_queued_callbacks(cx: &mut gpui::TestAppContext) {
            let (mut sender, events) = futures::channel::mpsc::channel(1);
            let calls = Rc::new(Cell::new(0));
            let count = calls.clone();
            let ui_thread = std::thread::current().id();
            let task = cx.update(|cx| {
                dispatch(
                    events,
                    move |_| {
                        assert_eq!(std::thread::current().id(), ui_thread);
                        count.set(count.get() + 1);
                    },
                    cx,
                )
            });
            let registration = RefCell::new(Registration {
                shortcut: None,
                task: Some(task),
            });
            sender.try_send(()).unwrap();
            cx.run_until_parked();
            assert_eq!(calls.get(), 1);

            sender.try_send(()).unwrap();
            futures::executor::block_on(stop(&registration)).unwrap();
            cx.run_until_parked();

            assert_eq!(calls.get(), 1);
            assert!(sender.try_send(()).is_err());
            futures::executor::block_on(stop(&registration)).unwrap();
        }
    }
}
