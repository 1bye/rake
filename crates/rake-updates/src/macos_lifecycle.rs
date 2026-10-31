//! An owned AppKit delegate proxy defers OS termination until asynchronous guards finish.
use crate::Error;
use std::{
    ffi::c_void,
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
struct State {
    requested: AtomicBool,
    changed: event_listener::Event,
}
unsafe extern "C" {
    fn rake_native_app_stop() -> bool;
    fn rake_native_quit_create(
        callback: extern "C" fn(*mut c_void),
        context: *mut c_void,
    ) -> *mut c_void;
    fn rake_native_quit_active(handle: *mut c_void) -> bool;
    fn rake_native_quit_prepare(handle: *mut c_void, approved: bool);
    fn rake_native_quit_reply(handle: *mut c_void, approved: bool);
    fn rake_native_quit_destroy(handle: *mut c_void);
}

/// Requests the running AppKit event loop to return without terminating the process.
///
/// # Errors
/// Returns an error when AppKit is not initialized or the caller is not the main thread.
pub fn stop_application_run_loop() -> Result<(), Error> {
    // SAFETY: the native helper checks AppKit initialization and main-thread ownership before
    // calling `-[NSApplication stop:]` and posting its wake event.
    if unsafe { rake_native_app_stop() } {
        Ok(())
    } else {
        Err(Error::Policy(
            "AppKit application loop stop requires the initialized main thread".into(),
        ))
    }
}

extern "C" fn request(context: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: creation retains this stable Box allocation until the proxy is
        // disconnected on the main thread before its owner releases the state.
        let state = unsafe { &*context.cast::<State>() };
        state.requested.store(true, Ordering::Release);
        state.changed.notify(usize::MAX);
    });
}
/// Owns the AppKit termination delegate lease; it must remain on the main thread.
pub struct NativeQuitBridge {
    handle: NonNull<c_void>,
    state: Box<State>,
    _main_thread: PhantomData<Rc<()>>,
}
impl std::fmt::Debug for NativeQuitBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeQuitBridge")
            .finish_non_exhaustive()
    }
}
impl NativeQuitBridge {
    /// Installs a forwarding proxy on the running AppKit application's main thread.
    /// # Errors
    /// Rejects absent AppKit initialization, other threads and a foreign delegate
    /// that already owns termination. Existing delegate methods are forwarded.
    pub fn new() -> Result<Self, Error> {
        let state = Box::new(State {
            requested: AtomicBool::new(false),
            changed: event_listener::Event::new(),
        });
        // SAFETY: the callback's allocation remains alive through this lease and
        // the Objective-C constructor checks main-thread/application prerequisites.
        let handle = unsafe {
            rake_native_quit_create(request, (&*state as *const State).cast_mut().cast())
        };
        let handle = NonNull::new(handle).ok_or_else(|| {
            Error::Policy(
                "AppKit quit bridge requires the main thread and an unclaimed termination delegate"
                    .into(),
            )
        })?;
        Ok(Self {
            handle,
            state,
            _main_thread: PhantomData,
        })
    }
    /// Returns whether this lease still owns the running application's termination delegate.
    pub fn is_active(&self) -> bool {
        // SAFETY: the !Send owner retains a live main-thread delegate lease.
        unsafe { rake_native_quit_active(self.handle.as_ptr()) }
    }

    /// Observes a native request once; the host then submits its ordinary guarded quit.
    pub fn take_request(&self) -> bool {
        self.state.requested.swap(false, Ordering::AcqRel)
    }
    /// Waits for OS termination without polling or borrowing the delegate lease.
    pub fn changed(&self) -> impl std::future::Future<Output = ()> + use<> {
        let listener = self.state.changed.listen();
        let requested = self.state.requested.load(Ordering::Acquire);
        async move {
            if !requested {
                listener.await;
            }
        }
    }
    /// Authorizes a provider's termination attempt only after guarded shutdown is Ready.
    /// This does not answer an existing OS request until [`Self::reply`] is called.
    pub fn prepare_exit(&self, approved: bool) {
        // SAFETY: this !Send owner holds a live main-thread delegate lease.
        unsafe { rake_native_quit_prepare(self.handle.as_ptr(), approved) };
    }
    /// Completes a pending AppKit termination request; false keeps the app running.
    /// A true reply may terminate the process, so all managed cleanup must precede it.
    pub fn reply(&self, approved: bool) {
        // SAFETY: this !Send owner holds a live main-thread delegate lease.
        unsafe { rake_native_quit_reply(self.handle.as_ptr(), approved) };
    }
}
impl Drop for NativeQuitBridge {
    fn drop(&mut self) {
        // SAFETY: disconnecting clears callback/context before releasing the proxy;
        // the Box allocation remains alive until this Drop returns.
        unsafe { rake_native_quit_destroy(self.handle.as_ptr()) };
    }
}
