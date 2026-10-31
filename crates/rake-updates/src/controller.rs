//! Observable provider-neutral state with host-owned background execution.
//!
//! ```no_run
//! use rake_updates::{UpdateController, UpdateState};
//! async fn watch_updates(updates: UpdateController) {
//!     let mut snapshot = updates.snapshot();
//!     loop {
//!         if let UpdateState::Available(info) = &snapshot.state {
//!             println!("Update {} available", info.version);
//!         }
//!         snapshot = updates.changed(snapshot.revision).await;
//!     }
//! }
//! ```
use crate::{Capability, DownloadProgress, Error, RestartHandoff, UpdateOwner, UpdateService};
use event_listener::Event;
#[cfg(all(target_os = "macos", feature = "sparkle"))]
use std::cell::RefCell;
use std::{
    ffi::OsString,
    future::Future,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Public metadata for an eligible update, obtained from the configured provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Human-facing available version.
    pub version: String,
}
/// The application's observable update lifecycle.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum UpdateState {
    /// No operation is active. A check is explicitly requested by the app.
    Idle,
    /// Checking the configured provider without installing anything.
    Checking,
    /// No eligible update was found.
    UpToDate,
    /// A provider has found an eligible release.
    Available(UpdateInfo),
    /// Downloading; completion still requires verification.
    Downloading {
        /// Selected release.
        update: UpdateInfo,
        /// Bounded transfer progress.
        progress: DownloadProgress,
    },
    /// The provider is preparing the downloaded update.
    Extracting {
        /// Selected release.
        update: UpdateInfo,
        /// Progress from zero to one.
        fraction: f64,
    },
    /// Verified staging is ready for a user-requested restart.
    ReadyToRestart(UpdateInfo),
    /// The host is running its save/quit guards.
    AwaitingApproval(UpdateInfo),
    /// Guards approved and the provider accepted the restart handoff.
    Restarting(UpdateInfo),
    /// Cancellation is pending completion of bounded provider I/O.
    Cancelling,
    /// The most recent operation failed; checking again is allowed.
    Failed {
        /// Provider diagnostic, without signing credentials.
        message: String,
    },
    /// Updates were disabled in the installed build.
    Disabled,
    /// The installation's package manager owns updates.
    ManagedExternally(UpdateOwner),
    /// The required provider or native lifecycle adapter is unavailable.
    ProviderUnavailable(UpdateOwner),
    /// Updates require an external installation action.
    External,
}
/// A revisioned snapshot for race-free UI subscriptions.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateSnapshot {
    /// Monotonic change counter; pass this to [`UpdateController::changed`].
    pub revision: u64,
    /// Current observable lifecycle.
    pub state: UpdateState,
}
#[cfg(any(feature = "appimage", feature = "velopack"))]
enum Selected {
    #[cfg(feature = "appimage")]
    Image(crate::ImageUpdate),
    #[cfg(feature = "velopack")]
    Velo(crate::VelopackUpdate),
}
#[cfg(any(feature = "appimage", feature = "velopack"))]
impl Selected {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: match self {
                #[cfg(feature = "appimage")]
                Self::Image(update) => update.release().version.clone(),
                #[cfg(feature = "velopack")]
                Self::Velo(update) => update.version().into(),
            },
        }
    }
}
#[cfg(any(feature = "appimage", feature = "velopack"))]
enum Staged {
    #[cfg(feature = "appimage")]
    Image(crate::StagedImage),
    #[cfg(feature = "velopack")]
    Velo(Box<crate::StagedVelopack>),
}
#[cfg(any(feature = "appimage", feature = "velopack"))]
impl Staged {
    fn handoff(self, args: Vec<OsString>) -> RestartHandoff {
        match self {
            #[cfg(feature = "appimage")]
            Self::Image(update) => update.restart_handoff(args),
            #[cfg(feature = "velopack")]
            Self::Velo(update) => update.restart_handoff(args),
        }
    }
}
enum Job {
    Check,
    #[cfg(any(feature = "appimage", feature = "velopack"))]
    Download(Box<Selected>),
    #[cfg(test)]
    Test(Box<dyn FnOnce() -> Result<Output, Error> + Send>),
}
enum Output {
    #[cfg(any(feature = "appimage", feature = "velopack"))]
    Checked(Option<Box<Selected>>),
    #[cfg(any(feature = "appimage", feature = "velopack"))]
    Downloaded(Staged),
    #[cfg(test)]
    Empty,
}
struct Machine {
    snapshot: UpdateSnapshot,
    pending: Option<Job>,
    active: Option<Arc<AtomicBool>>,
    generation: u64,
    #[cfg(any(feature = "appimage", feature = "velopack"))]
    selected: Option<Box<Selected>>,
    #[cfg(any(feature = "appimage", feature = "velopack"))]
    staged: Option<Staged>,
    closed: bool,
}
struct Shared {
    machine: Mutex<Machine>,
    changed: Event,
}
impl Shared {
    fn edit<T>(&self, edit: impl FnOnce(&mut Machine) -> T) -> T {
        let result = {
            let mut machine = self.machine.lock().expect("update state lock poisoned");
            let result = edit(&mut machine);
            machine.snapshot.revision = machine.snapshot.revision.saturating_add(1);
            result
        };
        self.changed.notify(usize::MAX);
        result
    }
}
struct Local {
    shared: Arc<Shared>,
    service: Arc<UpdateService>,
    #[cfg(all(target_os = "macos", feature = "sparkle"))]
    sparkle: RefCell<Option<crate::Sparkle>>,
}
impl Drop for Local {
    fn drop(&mut self) {
        self.shared.edit(|machine| {
            machine.closed = true;
            if let Some(flag) = &machine.active {
                flag.store(true, Ordering::Release);
            }
            machine.pending = None;
            #[cfg(any(feature = "appimage", feature = "velopack"))]
            {
                machine.selected = None;
                machine.staged = None;
            }
        });
    }
}
/// Shared local controller. Clones observe the same state; no threads are spawned.
///
/// Native Rake workspaces install and drive this service automatically. Custom
/// hosts own a task running [`Self::run`] and dispatch [`UpdateWork::run`] onto
/// their blocking executor. Dropping that task cancels active work. Network calls
/// remain bounded by provider deadlines; cancellation does not block the UI.
#[derive(Clone)]
pub struct UpdateController(Rc<Local>);
impl std::fmt::Debug for UpdateController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateController")
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}
impl UpdateController {
    /// Returns the installed public policy shared by all controller handles.
    pub fn binding(&self) -> &crate::UpdateBinding {
        self.0.service.binding()
    }

    /// Owns immutable installed policy; performs no I/O or automatic checks.
    pub fn new(service: UpdateService) -> Self {
        let state = match service.capability() {
            Capability::Disabled => UpdateState::Disabled,
            Capability::Managed(owner) => UpdateState::ManagedExternally(owner),
            Capability::External => UpdateState::External,
            Capability::ProviderUnavailable(owner) => UpdateState::ProviderUnavailable(owner),
            Capability::SelfManaged if service.binding().owner == UpdateOwner::Sparkle => {
                UpdateState::ProviderUnavailable(UpdateOwner::Sparkle)
            }
            Capability::SelfManaged => UpdateState::Idle,
        };
        Self(Rc::new(Local {
            shared: Arc::new(Shared {
                machine: Mutex::new(Machine {
                    snapshot: UpdateSnapshot { revision: 0, state },
                    pending: None,
                    active: None,
                    generation: 0,
                    #[cfg(any(feature = "appimage", feature = "velopack"))]
                    selected: None,
                    #[cfg(any(feature = "appimage", feature = "velopack"))]
                    staged: None,
                    closed: false,
                }),
                changed: Event::new(),
            }),
            service: Arc::new(service),
            #[cfg(all(target_os = "macos", feature = "sparkle"))]
            sparkle: RefCell::new(None),
        }))
    }
    /// Reports whether the installation and native lifecycle allow download/install actions.
    /// Standalone Sparkle hosts can check availability before attaching a quit guard.
    pub fn can_install(&self) -> bool {
        if self.0.service.capability() != Capability::SelfManaged {
            return false;
        }
        #[cfg(all(target_os = "macos", feature = "sparkle"))]
        if self.binding().owner == UpdateOwner::Sparkle {
            return self
                .0
                .sparkle
                .borrow()
                .as_ref()
                .is_some_and(crate::Sparkle::guarded);
        }
        true
    }
    /// Returns an owned snapshot without performing provider I/O.
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.0
            .shared
            .machine
            .lock()
            .expect("update state lock poisoned")
            .snapshot
            .clone()
    }
    /// Returns current state for rendering; use [`Self::snapshot`] for subscriptions.
    pub fn state(&self) -> UpdateState {
        self.snapshot().state
    }
    /// Waits until a revision changes, including Sparkle callbacks on the main thread.
    /// Registering before inspecting the revision prevents missed wakeups.
    pub async fn changed(&self, revision: u64) -> UpdateSnapshot {
        loop {
            let listener = self.0.shared.changed.listen();
            #[cfg(all(target_os = "macos", feature = "sparkle"))]
            let native = self.0.sparkle.borrow().as_ref().map(crate::Sparkle::listen);
            self.refresh_native();
            let snapshot = self.snapshot();
            if snapshot.revision != revision {
                return snapshot;
            }
            #[cfg(all(target_os = "macos", feature = "sparkle"))]
            if let Some(native) = native {
                let mut listener = std::pin::pin!(listener);
                let mut native = std::pin::pin!(native);
                std::future::poll_fn(|cx| {
                    if listener.as_mut().poll(cx).is_ready() || native.as_mut().poll(cx).is_ready()
                    {
                        std::task::Poll::Ready(())
                    } else {
                        std::task::Poll::Pending
                    }
                })
                .await;
                continue;
            }
            listener.await;
        }
    }
    /// Queues a check. Sparkle uses an information-only check with no native dialog.
    /// # Errors
    /// Rejects busy, disabled, externally managed or unavailable providers.
    pub fn check(&self) -> Result<(), Error> {
        #[cfg(all(target_os = "macos", feature = "sparkle"))]
        if let Some(sparkle) = self.0.sparkle.borrow().as_ref() {
            self.ensure_idle()?;
            sparkle.check_for_update_information()?;
            self.refresh_native();
            return Ok(());
        }
        self.ensure_available()?;
        self.queue(Job::Check, UpdateState::Checking)
    }
    /// Starts downloading the available update. Sparkle presents its native review UI.
    /// # Errors
    /// Rejects concurrent operations and calls without an eligible checked update.
    pub fn download(&self) -> Result<(), Error> {
        self.refresh_native();
        self.ensure_idle()?;
        let UpdateState::Available(info) = self.state() else {
            return Err(Error::Policy(
                "check for an available update before downloading".into(),
            ));
        };
        #[cfg(all(target_os = "macos", feature = "sparkle"))]
        if let Some(sparkle) = self.0.sparkle.borrow().as_ref() {
            sparkle.check_for_updates()?;
            self.refresh_native();
            return Ok(());
        }
        #[cfg(any(feature = "appimage", feature = "velopack"))]
        {
            let selected = self
                .0
                .shared
                .edit(|machine| machine.selected.take())
                .ok_or_else(|| Error::Policy("checked update is no longer available".into()))?;
            self.queue(
                Job::Download(selected),
                UpdateState::Downloading {
                    update: info,
                    progress: DownloadProgress {
                        total_bytes: 0,
                        received_bytes: 0,
                    },
                },
            )
        }
        #[cfg(not(any(feature = "appimage", feature = "velopack")))]
        {
            let _ = info;
            Err(Error::Policy("no download provider is enabled".into()))
        }
    }
    /// Requests cancellation; active I/O owns cleanup until its bounded call finishes.
    pub fn cancel(&self) {
        #[cfg(all(target_os = "macos", feature = "sparkle"))]
        if let Some(sparkle) = self.0.sparkle.borrow().as_ref() {
            sparkle.cancel();
        }
        self.0.shared.edit(|machine| {
            if let Some(flag) = &machine.active {
                flag.store(true, Ordering::Release);
                if machine.pending.take().is_some() {
                    machine.active = None;
                    machine.snapshot.state = UpdateState::Idle;
                } else {
                    machine.snapshot.state = UpdateState::Cancelling;
                }
            }
            if matches!(machine.snapshot.state, UpdateState::AwaitingApproval(_)) {
                machine.generation = machine.generation.wrapping_add(1);
                machine.snapshot.state = UpdateState::Idle;
            }
            #[cfg(any(feature = "appimage", feature = "velopack"))]
            if machine.staged.take().is_some() {
                machine.snapshot.state = UpdateState::Idle;
            }
        });
        self.refresh_native();
    }
    /// Takes verified staging once, without approving or performing installation.
    /// Pass the handoff to the host's save/quit guards. Dropping or declining it
    /// never installs. AppImage/Velopack discard staging and return to Idle; check
    /// again to retry. Sparkle may retain an eligible SDK update for native review.
    /// # Errors
    /// Rejects calls without ready staging or a live Sparkle restart continuation.
    pub fn restart_handoff(&self, args: Vec<OsString>) -> Result<RestartHandoff, Error> {
        self.refresh_native();
        let UpdateState::ReadyToRestart(info) = self.state() else {
            return Err(Error::Policy("no update is ready to restart".into()));
        };
        #[cfg(all(target_os = "macos", feature = "sparkle"))]
        if let Some(sparkle) = self.0.sparkle.borrow().as_ref() {
            let handoff = sparkle
                .take_restart()?
                .ok_or_else(|| Error::Policy("no native restart was requested".into()))?;
            return Ok(self.observe_handoff(handoff, info));
        }
        #[cfg(any(feature = "appimage", feature = "velopack"))]
        {
            let staged = self
                .0
                .shared
                .edit(|machine| machine.staged.take())
                .ok_or_else(|| Error::Policy("staging was already consumed".into()))?;
            Ok(self.observe_handoff(staged.handoff(args), info))
        }
        #[cfg(not(any(feature = "appimage", feature = "velopack")))]
        {
            let _ = (args, info);
            Err(Error::Policy("no update provider is enabled".into()))
        }
    }
    #[cfg(any(
        test,
        feature = "appimage",
        feature = "velopack",
        all(target_os = "macos", feature = "sparkle")
    ))]
    fn observe_handoff(&self, handoff: RestartHandoff, info: UpdateInfo) -> RestartHandoff {
        let generation = self.0.shared.edit(|machine| {
            machine.generation = machine.generation.wrapping_add(1);
            machine.snapshot.state = UpdateState::AwaitingApproval(info.clone());
            machine.generation
        });
        let guard = HandoffGuard {
            shared: Arc::clone(&self.0.shared),
            committed: false,
            generation,
        };
        let disposition = handoff.disposition;
        RestartHandoff {
            disposition,
            action: Some(Box::new(move || {
                let mut guard = guard;
                guard.committed = true;
                let valid = {
                    let machine = guard
                        .shared
                        .machine
                        .lock()
                        .expect("update state lock poisoned");
                    !machine.closed && machine.generation == guard.generation
                };
                let result = if valid {
                    handoff.commit(true)
                } else {
                    Err(Error::Cancelled)
                };
                guard.shared.edit(|machine| {
                    if machine.generation != guard.generation || machine.closed {
                        return;
                    }
                    machine.snapshot.state = match &result {
                        Ok(_) => UpdateState::Restarting(info),
                        Err(error) => UpdateState::Failed {
                            message: error.to_string(),
                        },
                    }
                });
                result.map(|_| ())
            })),
        }
    }
    fn ensure_idle(&self) -> Result<(), Error> {
        if matches!(
            self.state(),
            UpdateState::Checking
                | UpdateState::Downloading { .. }
                | UpdateState::Extracting { .. }
                | UpdateState::Cancelling
                | UpdateState::ReadyToRestart(_)
                | UpdateState::AwaitingApproval(_)
                | UpdateState::Restarting(_)
        ) {
            return Err(Error::Busy);
        }
        Ok(())
    }
    fn ensure_available(&self) -> Result<(), Error> {
        self.ensure_idle()?;
        if self.0.service.capability() != Capability::SelfManaged
            || matches!(self.state(), UpdateState::ProviderUnavailable(_))
        {
            return Err(Error::Policy(
                "this installation does not support in-app updates".into(),
            ));
        }
        Ok(())
    }
    fn queue(&self, job: Job, state: UpdateState) -> Result<(), Error> {
        self.ensure_idle()?;
        self.0.shared.edit(|machine| {
            machine.generation = machine.generation.wrapping_add(1);
            machine.active = Some(Arc::new(AtomicBool::new(false)));
            machine.pending = Some(job);
            machine.snapshot.state = state;
            #[cfg(any(feature = "appimage", feature = "velopack"))]
            {
                machine.selected = None;
                machine.staged = None;
            }
        });
        Ok(())
    }
    /// Takes one queued blocking operation. Hosts normally use [`Self::run`].
    pub fn take_work(&self) -> Option<UpdateWork> {
        let mut machine = self
            .0
            .shared
            .machine
            .lock()
            .expect("update state lock poisoned");
        machine.pending.take().map(|job| UpdateWork {
            job: Some(job),
            shared: Arc::clone(&self.0.shared),
            service: Arc::clone(&self.0.service),
            cancelled: Arc::clone(
                machine
                    .active
                    .as_ref()
                    .expect("queued update owns cancellation"),
            ),
            generation: machine.generation,
            completed: false,
        })
    }
    /// Drives queued work on a host-provided blocking executor and native events
    /// on the foreground executor. The host must retain this future until shutdown.
    /// Dropping it requests cancellation; provider cleanup may finish in the background.
    pub async fn run<F: Future<Output = ()>>(&self, mut execute: impl FnMut(UpdateWork) -> F) {
        let _guard = DriverGuard(self.clone());
        loop {
            self.refresh_native();
            let revision = self.snapshot().revision;
            if let Some(work) = self.take_work() {
                execute(work).await;
            } else {
                self.changed(revision).await;
            }
        }
    }
    #[cfg(all(target_os = "macos", feature = "sparkle"))]
    /// Initializes the macOS information-only SDK without granting installation authority.
    /// Native Rake applications call this on the AppKit main thread automatically.
    /// # Errors
    /// Returns native bundle/policy/SDK initialization failures.
    pub fn initialize_sparkle_checks(&self) -> Result<(), Error> {
        if self.binding().owner != UpdateOwner::Sparkle || self.0.sparkle.borrow().is_some() {
            return Ok(());
        }
        let sparkle = crate::Sparkle::probe(&self.0.service)?;
        *self.0.sparkle.borrow_mut() = Some(sparkle);
        self.0
            .shared
            .edit(|machine| machine.snapshot.state = UpdateState::Idle);
        Ok(())
    }
    #[cfg(all(target_os = "macos", feature = "sparkle"))]
    /// Installs the main-thread SDK using the host's existing native quit guard.
    /// # Errors
    /// Rejects another attachment, incorrect policy or invalid native bundle setup.
    pub fn attach_sparkle(&self, quit: Rc<crate::NativeQuitBridge>) -> Result<(), Error> {
        self.refresh_native();
        self.ensure_idle()?;
        if self
            .0
            .sparkle
            .borrow()
            .as_ref()
            .is_some_and(crate::Sparkle::guarded)
        {
            return Err(Error::Busy);
        }
        // A probing SDK is never replaced while its network session is active.
        self.0.sparkle.borrow_mut().take();
        self.0.shared.edit(|machine| {
            machine.snapshot.state = UpdateState::ProviderUnavailable(UpdateOwner::Sparkle)
        });
        let sparkle = crate::Sparkle::new(&self.0.service, quit)?;
        *self.0.sparkle.borrow_mut() = Some(sparkle);
        self.0
            .shared
            .edit(|machine| machine.snapshot.state = UpdateState::Idle);
        Ok(())
    }
    #[cfg(all(target_os = "macos", feature = "sparkle"))]
    /// Reads Sparkle-specific diagnostics for native lifecycle hosts.
    /// Application UI should normally observe [`Self::state`].
    pub fn native_status(&self) -> Option<crate::SparkleStatus> {
        self.0.sparkle.borrow().as_ref().map(crate::Sparkle::status)
    }
    fn refresh_native(&self) {
        #[cfg(all(target_os = "macos", feature = "sparkle"))]
        if let Some(status) = self.native_status() {
            use crate::SparkleStage;
            let info = status
                .available_version
                .map(|version| UpdateInfo { version });
            let state = match status.stage {
                SparkleStage::Idle => Some(UpdateState::Idle),
                SparkleStage::Checking => Some(UpdateState::Checking),
                SparkleStage::Available => info.map(UpdateState::Available),
                SparkleStage::Downloading => info.map(|update| UpdateState::Downloading {
                    update,
                    progress: DownloadProgress {
                        received_bytes: status.downloaded_bytes,
                        total_bytes: status.total_bytes,
                    },
                }),
                SparkleStage::Extracting => info.map(|update| UpdateState::Extracting {
                    update,
                    fraction: status.extraction_fraction,
                }),
                SparkleStage::AwaitingRestart => info.map(UpdateState::ReadyToRestart),
                SparkleStage::Installing => info.map(UpdateState::Restarting),
                SparkleStage::Failed => Some(UpdateState::Failed {
                    message: status
                        .error
                        .unwrap_or_else(|| "native update failed".into()),
                }),
                SparkleStage::NoUpdate => Some(UpdateState::UpToDate),
                SparkleStage::Cancelling => Some(UpdateState::Cancelling),
            };
            if let Some(state) = state {
                let current = self.state();
                let waiting_for_guards = matches!(current, UpdateState::AwaitingApproval(_))
                    && !matches!(state, UpdateState::Failed { .. });
                let waiting_for_native_exit = matches!(current, UpdateState::Restarting(_))
                    && matches!(state, UpdateState::ReadyToRestart(_));
                if current != state && !waiting_for_guards && !waiting_for_native_exit {
                    self.0.shared.edit(|machine| machine.snapshot.state = state);
                }
            }
        }
    }
}
struct DriverGuard(UpdateController);
impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
#[cfg(any(
    test,
    feature = "appimage",
    feature = "velopack",
    all(target_os = "macos", feature = "sparkle")
))]
struct HandoffGuard {
    shared: Arc<Shared>,
    committed: bool,
    generation: u64,
}
#[cfg(any(
    test,
    feature = "appimage",
    feature = "velopack",
    all(target_os = "macos", feature = "sparkle")
))]
impl Drop for HandoffGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.shared.edit(|machine| {
                if machine.generation == self.generation {
                    machine.snapshot.state = UpdateState::Idle;
                }
            });
        }
    }
}

/// One bounded blocking operation. Run on a host-owned worker, never the UI thread.
/// Dropping unstarted work cancels it. Results and diagnostics publish to the
/// controller automatically; no native installer runs until a guarded handoff.
#[must_use = "run on a blocking executor or drop to cancel"]
pub struct UpdateWork {
    job: Option<Job>,
    shared: Arc<Shared>,
    service: Arc<UpdateService>,
    cancelled: Arc<AtomicBool>,
    generation: u64,
    completed: bool,
}
impl std::fmt::Debug for UpdateWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateWork")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl UpdateWork {
    /// Executes blocking provider I/O and publishes its verified result or error.
    pub fn run(mut self) {
        let result = if self.cancelled.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else {
            let job = self.job.take().expect("update work runs once");
            self.execute(job)
        };
        self.shared.edit(|machine| {
            if machine.closed || machine.generation != self.generation {
                return;
            }
            machine.active = None;
            if self.cancelled.load(Ordering::Acquire) || matches!(result, Err(Error::Cancelled)) {
                machine.snapshot.state = UpdateState::Idle;
                return;
            }
            match result {
                #[cfg(any(feature = "appimage", feature = "velopack"))]
                Ok(Output::Checked(update)) => {
                    machine.snapshot.state =
                        update.as_ref().map_or(UpdateState::UpToDate, |update| {
                            UpdateState::Available(update.info())
                        });
                    machine.selected = update;
                }
                #[cfg(any(feature = "appimage", feature = "velopack"))]
                Ok(Output::Downloaded(staged)) => {
                    if let UpdateState::Downloading { update, .. } = &machine.snapshot.state {
                        machine.snapshot.state = UpdateState::ReadyToRestart(update.clone());
                    }
                    machine.staged = Some(staged);
                }
                #[cfg(test)]
                Ok(Output::Empty) => machine.snapshot.state = UpdateState::UpToDate,
                Err(error) => {
                    machine.snapshot.state = UpdateState::Failed {
                        message: error.to_string(),
                    }
                }
            }
        });
        self.completed = true;
    }
    fn execute(&self, job: Job) -> Result<Output, Error> {
        match job {
            Job::Check => match self.service.binding().owner {
                #[cfg(feature = "appimage")]
                UpdateOwner::Appimage => Ok(Output::Checked(
                    self.service
                        .check_appimage(&self.cancelled)?
                        .map(|update| Box::new(Selected::Image(update))),
                )),
                #[cfg(feature = "velopack")]
                UpdateOwner::Velopack => Ok(Output::Checked(
                    self.service
                        .check_velopack(Arc::clone(&self.cancelled))?
                        .map(|update| Box::new(Selected::Velo(update))),
                )),
                _ => Err(Error::Policy(
                    "no background update provider is active".into(),
                )),
            },
            #[cfg(any(feature = "appimage", feature = "velopack"))]
            Job::Download(update) => {
                let shared = Arc::clone(&self.shared);
                let generation = self.generation;
                let progress = move |progress| {
                    shared.edit(|machine| {
                        if machine.generation == generation
                            && !machine.closed
                            && let UpdateState::Downloading {
                                progress: current, ..
                            } = &mut machine.snapshot.state
                        {
                            *current = progress;
                        }
                    });
                };
                Ok(Output::Downloaded(match *update {
                    #[cfg(feature = "appimage")]
                    Selected::Image(update) => Staged::Image(self.service.stage_appimage(
                        update,
                        &self.cancelled,
                        progress,
                    )?),
                    #[cfg(feature = "velopack")]
                    Selected::Velo(update) => Staged::Velo(Box::new(self.service.stage_velopack(
                        update,
                        Arc::clone(&self.cancelled),
                        progress,
                    )?)),
                }))
            }
            #[cfg(test)]
            Job::Test(run) => run(),
        }
    }
}
impl Drop for UpdateWork {
    fn drop(&mut self) {
        if !self.completed {
            self.cancelled.store(true, Ordering::Release);
            self.shared.edit(|machine| {
                if machine.generation == self.generation {
                    machine.active = None;
                    machine.snapshot.state = UpdateState::Idle;
                }
            });
        }
    }
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
