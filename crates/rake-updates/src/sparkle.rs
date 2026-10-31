//! Main-thread Sparkle SDK ownership and deferred native restart continuations.
use crate::{Error, NativeQuitBridge, RestartHandoff, UpdateService};
use rake_dist_core::UpdateOwner;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    ptr::NonNull,
    rc::Rc,
};
struct Signal {
    changed: event_listener::Event,
}
unsafe extern "C" {
    fn rake_sparkle_create(
        identifier: *const c_char,
        feed: *const c_char,
        key: *const c_char,
        installation_allowed: bool,
        notify: extern "C" fn(*mut c_void),
        context: *mut c_void,
        error: *mut c_char,
        capacity: usize,
    ) -> *mut c_void;
    fn rake_sparkle_check(handle: *mut c_void);
    fn rake_sparkle_probe(handle: *mut c_void) -> bool;
    fn rake_sparkle_pending(handle: *mut c_void) -> bool;
    fn rake_sparkle_take(handle: *mut c_void) -> bool;
    fn rake_sparkle_reply(handle: *mut c_void, approved: bool) -> bool;
    fn rake_sparkle_cancel(handle: *mut c_void);
    fn rake_sparkle_status(
        handle: *mut c_void,
        stage: *mut u32,
        received: *mut u64,
        total: *mut u64,
        extraction: *mut f64,
        error: *mut c_char,
        capacity: usize,
        version: *mut c_char,
        version_capacity: usize,
    );
    fn rake_sparkle_destroy(handle: *mut c_void);
}
extern "C" fn notify(context: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: native ownership clears this stable Box pointer before release.
        let signal = unsafe { &*context.cast::<Signal>() };
        signal.changed.notify(usize::MAX);
    });
}
struct Inner {
    handle: NonNull<c_void>,
    signal: Box<Signal>,
    quit: Option<Rc<NativeQuitBridge>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        // SAFETY: this Rc owner is !Send, and disconnects callbacks before the Box is freed.
        unsafe { rake_sparkle_destroy(self.handle.as_ptr()) };
    }
}
/// A native update cycle reported by Sparkle's standard user driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparkleStage {
    /// No update operation is active.
    Idle,
    /// Fetching and authenticating the native appcast.
    Checking,
    /// An eligible version is available for application UI.
    Available,
    /// A cancelled information check is finishing inside the SDK.
    Cancelling,
    /// Downloading an authenticated release.
    Downloading,
    /// Verifying/extracting through Sparkle's installer.
    Extracting,
    /// The user requested installation; Rake must approve guarded shutdown.
    AwaitingRestart,
    /// The SDK has prepared its installer and is waiting for the host process to exit.
    Installing,
    /// The native SDK reported a failure.
    Failed,
    /// No eligible update was found.
    NoUpdate,
}
/// Native progress and diagnostic state, without private signing information.
#[derive(Clone, Debug)]
pub struct SparkleStatus {
    /// Current SDK/user-driver stage.
    pub stage: SparkleStage,
    /// Eligible display version supplied by Sparkle, retained through staging.
    pub available_version: Option<String>,
    /// Bytes downloaded so far.
    pub downloaded_bytes: u64,
    /// Expected bytes; zero means unknown.
    pub total_bytes: u64,
    /// Extraction progress between zero and one.
    pub extraction_fraction: f64,
    /// SDK diagnostic for the latest error.
    pub error: Option<String>,
}
/// Owns the updater, standard user driver, deferred replies and native quit guard lease.
/// All methods and destruction remain on the AppKit main thread.
pub struct Sparkle {
    inner: Rc<Inner>,
}
impl std::fmt::Debug for Sparkle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Sparkle").finish_non_exhaustive()
    }
}
impl Sparkle {
    /// Starts the pinned SDK with embedded feed/key policy and an active native quit guard.
    /// Automatic download/install is disabled. The host must drive both native quit
    /// requests and [`Self::take_restart`] on its foreground executor.
    /// # Errors
    /// Rejects wrong owners, invalid bundle/key policy, missing AppKit initialization,
    /// a lost guard lease and native SDK startup failures.
    pub fn new(service: &UpdateService, quit: Rc<NativeQuitBridge>) -> Result<Self, Error> {
        if service.binding.owner != UpdateOwner::Sparkle || !quit.is_active() {
            return Err(Error::Policy(
                "Sparkle requires its compiled owner and an active native quit guard".into(),
            ));
        }
        Self::from_service(service, Some(quit))
    }
    pub(crate) fn probe(service: &UpdateService) -> Result<Self, Error> {
        Self::from_service(service, None)
    }
    pub(crate) fn guarded(&self) -> bool {
        self.inner
            .quit
            .as_ref()
            .is_some_and(|quit| quit.is_active())
    }
    fn from_service(
        service: &UpdateService,
        quit: Option<Rc<NativeQuitBridge>>,
    ) -> Result<Self, Error> {
        if service.binding.owner != UpdateOwner::Sparkle {
            return Err(Error::Policy(
                "this installation does not use Sparkle".into(),
            ));
        }
        let binding = &service.binding;
        let feed = rake_dist_core::feed_url(
            binding
                .base_url
                .as_deref()
                .ok_or_else(|| Error::Policy("Sparkle feed missing".into()))?,
            &binding.channel,
            &binding.target,
            "appcast.xml",
        )
        .map_err(|error| Error::Policy(error.to_string()))?;
        #[cfg(feature = "native-tests")]
        let feed = service.test_sparkle_feed.clone().unwrap_or(feed);
        let key = binding
            .public_keys
            .get("sparkle")
            .ok_or_else(|| Error::Policy("Sparkle public key missing".into()))?;
        let key = std::str::from_utf8(key).map_err(|error| Error::Policy(error.to_string()))?;
        Self::create(&binding.identifier, &feed, key.trim(), quit)
    }
    fn create(
        identifier: &str,
        feed: &str,
        key: &str,
        quit: Option<Rc<NativeQuitBridge>>,
    ) -> Result<Self, Error> {
        let identifier =
            CString::new(identifier).map_err(|error| Error::Policy(error.to_string()))?;
        let feed = CString::new(feed).map_err(|error| Error::Policy(error.to_string()))?;
        let key = CString::new(key).map_err(|error| Error::Policy(error.to_string()))?;
        let signal = Box::new(Signal {
            changed: event_listener::Event::new(),
        });
        let mut error = [0; c_char_buffer_size()];
        // SAFETY: all input strings/buffer are live through the call. The stable
        // signal allocation outlives native callbacks, disconnected in Inner::drop.
        let handle = unsafe {
            rake_sparkle_create(
                identifier.as_ptr(),
                feed.as_ptr(),
                key.as_ptr(),
                quit.is_some(),
                notify,
                (&*signal as *const Signal).cast_mut().cast(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let handle = NonNull::new(handle).ok_or_else(|| Error::Provider(diagnostic(&error)))?;
        Ok(Self {
            inner: Rc::new(Inner {
                handle,
                signal,
                quit,
            }),
        })
    }
    /// Shows Sparkle's native update/check UI; cached pending updates use the same guard.
    /// # Errors
    /// Rejects a lost native guard lease before starting another update cycle.
    pub fn check_for_updates(&self) -> Result<(), Error> {
        self.ensure_guard()?;
        // SAFETY: the owned !Send controller remains on the AppKit main thread.
        unsafe { rake_sparkle_check(self.inner.handle.as_ptr()) };
        Ok(())
    }
    /// Probes the signed appcast without opening Sparkle's update dialog.
    /// # Errors
    /// Rejects an inactive guard or another native update session.
    pub fn check_for_update_information(&self) -> Result<(), Error> {
        if self.inner.quit.is_some() {
            self.ensure_guard()?;
        }
        // SAFETY: the controller owns this live main-thread SDK bridge.
        if !unsafe { rake_sparkle_probe(self.inner.handle.as_ptr()) } {
            return Err(Error::Busy);
        }
        Ok(())
    }
    pub(crate) fn listen(&self) -> event_listener::EventListener {
        self.inner.signal.changed.listen()
    }
    /// Waits for native progress/restart events without polling or borrowing the controller.
    pub fn changed(&self) -> impl std::future::Future<Output = ()> + use<> {
        let listener = self.inner.signal.changed.listen();
        // SAFETY: the controller owns a live main-thread SDK bridge.
        let pending = unsafe { rake_sparkle_pending(self.inner.handle.as_ptr()) };
        async move {
            if !pending {
                listener.await;
            }
        }
    }
    /// Takes a user-requested native continuation once, for `Router::quit_for_update`.
    /// Dropping the handoff declines the SDK installation rather than retaining approval.
    /// # Errors
    /// Rejects a lost native quit guard lease.
    pub fn take_restart(&self) -> Result<Option<RestartHandoff>, Error> {
        self.ensure_guard()?;
        // SAFETY: the controller owns a live main-thread SDK bridge.
        if !unsafe { rake_sparkle_take(self.inner.handle.as_ptr()) } {
            return Ok(None);
        }
        let mut pending = Pending {
            inner: Rc::clone(&self.inner),
            committed: false,
        };
        Ok(Some(RestartHandoff::native_provider(move || {
            let quit = pending
                .inner
                .quit
                .as_ref()
                .filter(|quit| quit.is_active())
                .ok_or_else(|| Error::Policy("native quit guard lease was lost".into()))?;
            quit.prepare_exit(true);
            pending.committed = true;
            // SAFETY: pending retains the native bridge and commits its current callback once.
            if !unsafe { rake_sparkle_reply(pending.inner.handle.as_ptr(), true) } {
                return Err(Error::Provider(
                    "Sparkle restart continuation is no longer pending".into(),
                ));
            }
            Ok(())
        })))
    }
    /// Cancels a pending check, download or restart decision; never approves restart.
    /// Extraction is SDK-owned and cannot be interrupted through this method.
    pub fn cancel(&self) {
        // SAFETY: the controller owns a live main-thread SDK bridge.
        unsafe { rake_sparkle_cancel(self.inner.handle.as_ptr()) };
    }
    /// Reads native SDK progress and its latest diagnostic.
    pub fn status(&self) -> SparkleStatus {
        let (mut stage, mut received, mut total, mut extraction) = (0, 0, 0, 0.0_f64);
        let mut error = [0; c_char_buffer_size()];
        let mut version = [0; c_char_buffer_size()];
        // SAFETY: all output references/buffer are valid, uniquely borrowed and bounded.
        unsafe {
            rake_sparkle_status(
                self.inner.handle.as_ptr(),
                &mut stage,
                &mut received,
                &mut total,
                &mut extraction,
                error.as_mut_ptr(),
                error.len(),
                version.as_mut_ptr(),
                version.len(),
            )
        };
        let error = diagnostic(&error);
        let version = diagnostic(&version);
        SparkleStatus {
            stage: match stage {
                1 => SparkleStage::Checking,
                2 => SparkleStage::Downloading,
                3 => SparkleStage::Extracting,
                4 => SparkleStage::AwaitingRestart,
                5 => SparkleStage::Failed,
                6 => SparkleStage::NoUpdate,
                7 => SparkleStage::Installing,
                8 => SparkleStage::Available,
                9 => SparkleStage::Cancelling,
                _ => SparkleStage::Idle,
            },
            available_version: (!version.is_empty()).then_some(version),
            downloaded_bytes: received,
            total_bytes: total,
            extraction_fraction: if extraction.is_finite() {
                extraction.clamp(0.0, 1.0)
            } else {
                0.0
            },
            error: (!error.is_empty()).then_some(error),
        }
    }
    fn ensure_guard(&self) -> Result<(), Error> {
        if self.inner.quit.is_none() {
            return Err(Error::Policy(
                "installation requires the host's native quit guard".into(),
            ));
        }
        if self.guarded() {
            Ok(())
        } else {
            self.cancel();
            Err(Error::Policy("native quit guard lease was lost".into()))
        }
    }
}
struct Pending {
    inner: Rc<Inner>,
    committed: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.committed {
            // SAFETY: this !Send token retains the bridge until its decline reply completes.
            unsafe { rake_sparkle_reply(self.inner.handle.as_ptr(), false) };
        }
    }
}
const fn c_char_buffer_size() -> usize {
    1024
}
fn diagnostic(buffer: &[c_char; c_char_buffer_size()]) -> String {
    // SAFETY: native functions use snprintf with the nonempty buffer capacity,
    // guaranteeing a terminator; the initially zeroed buffer also covers early errors.
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}
