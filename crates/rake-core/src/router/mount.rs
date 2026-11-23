//! Local route preparation, attachment and retained mount lifetime.

use std::any::Any;

use futures::future::LocalBoxFuture;

use crate::http::CancellationToken;
use crate::services::ServiceRegistry;
use crate::tasks::HostVisibility;

use super::{EntryHandle, EntryId, MountError, Navigator, PreparedPage, Route, WindowId};

/// The router's presentation of a committed mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutePresentation {
    /// The entry is alive in a tab or its history, but is not the window's selected entry.
    Retained,
    /// The entry is selected in its window, which may itself be hidden.
    Presented(HostVisibility),
}

/// The provisional entry and its host, supplied when mounting a fresh route.
pub struct RouteMountContext<R: Route> {
    /// Window that owns the entry. A secondary host has already been opened.
    pub window_id: WindowId,
    /// Stable identity of this visit, preserved across history and checkpoint restore.
    pub entry_id: EntryId,
    /// Weak handle valid during mount, including for reading restored entry state.
    pub entry: EntryHandle<R>,
    /// Weak navigator bound to the entry's window and tab.
    pub navigator: Navigator<R>,
}

/// The host's concrete view and resources for one retained route entry.
///
/// The router takes ownership of the handle and retains it across history
/// traversal. Implementations can hold local UI state and weak runtime handles
/// and should avoid owning the router or a strong runtime client.
pub trait MountedRoute: Any {
    /// Applies a presentation change synchronously on the host's event loop.
    ///
    /// The first call follows commit, with the entry already in the router
    /// snapshot. A hidden history entry receives `Retained`. When selection
    /// changes, the previous entry is hidden before the next is shown. Changes
    /// to the selected window's reported visibility are also delivered here.
    /// A provisional mount never receives this callback.
    ///
    /// During checkpoint restore, apply the initial presentation before
    /// returning. The router reports `Restored` after all initial callbacks
    /// return. A host can report its first painted frame separately.
    fn presentation_changed(&self, _presentation: RoutePresentation) {}

    /// Releases host resources once, after the entry's weak handle is invalidated.
    ///
    /// A presented entry is first told it is retained. A rolled-back mount
    /// receives only disposal. The router releases its ownership after this
    /// call. Implement this for framework resources that need explicit cleanup.
    fn dispose(&self) {}
}

/// Prepares and mounts typed destinations on the caller-driven router loop.
///
/// Both callbacks can use local futures. The router owns admission and history,
/// and publishes the mount only after the transition commits.
pub trait RouteMount<R: Route>: 'static {
    /// Prepares owned data after admission and initial guards, before mounting
    /// or opening a secondary host. During restore, all entries prepare before
    /// any secondary host is opened.
    ///
    /// The router can drop the future on cancellation. Use the token for
    /// cooperative work, and release provisional resources on drop.
    ///
    /// # Errors
    /// Return a preparation error to leave the current workspace intact.
    fn prepare(
        &self,
        _destination: &R,
        _services: ServiceRegistry,
        _cancellation: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
        Box::pin(async { Ok(PreparedPage::empty()) })
    }

    /// Consumes prepared data and creates a provisional mount.
    ///
    /// Keep it hidden until committed presentation. During restore, read
    /// entry checkpoints through the weak handle before returning. On error or
    /// cancellation before a handle is returned, the adapter must clean up
    /// resources it has created. After it returns a handle, the router disposes
    /// it on rollback, removal, close or shutdown.
    ///
    /// # Errors
    /// Return a mount error to leave the current workspace intact.
    fn mount(
        &self,
        destination: &R,
        prepared: PreparedPage,
        context: RouteMountContext<R>,
    ) -> LocalBoxFuture<'static, Result<Box<dyn MountedRoute>, MountError>>;
}

pub(crate) struct Headless;

impl MountedRoute for Headless {}

impl<R: Route> RouteMount<R> for Headless {
    fn mount(
        &self,
        _destination: &R,
        _prepared: PreparedPage,
        _context: RouteMountContext<R>,
    ) -> LocalBoxFuture<'static, Result<Box<dyn MountedRoute>, MountError>> {
        Box::pin(async { Ok(Box::new(Self) as Box<dyn MountedRoute>) })
    }
}
