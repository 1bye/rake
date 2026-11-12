//! Opt-in native window placement, stored through the preferences service.
//!
//! Coordinates are logical pixels, independent of the display's backing scale.
//! Browser hosts ignore the policy and its reset operations.

use std::rc::Rc;
#[cfg(not(target_family = "wasm"))]
use std::{cell::RefCell, collections::BTreeMap};

use crate::{platform::Preferences, router::WindowId};

#[cfg(not(target_family = "wasm"))]
mod placement;

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
pub(crate) mod native;

#[cfg(not(target_family = "wasm"))]
use placement::SavedWindow;

#[cfg(not(target_family = "wasm"))]
const PREFERENCE_KEY: &str = "rake-window-state-v1";

/// Shared placement policy and reset handle for an application's native windows.
///
/// By default only the initial window (`WindowId(1)`) uses the stable key `main`.
/// Secondary windows require an explicit mapping with [`Self::keys`]. Clones share
/// records, writes and resets; use one policy per preference namespace.
///
/// ```no_run
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let preferences = rake::platform::Platform::default().preferences("example.app")?;
/// let state = rake::window_state::WindowState::new(preferences);
/// # #[cfg(feature = "gpui")]
/// let app = rake::App::new("example.app").window_state(state.clone());
/// // A settings action can await state.reset("main") to forget its placement.
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct WindowState {
    #[cfg(not(target_family = "wasm"))]
    inner: Rc<Inner>,
    keys: Rc<dyn Fn(WindowId) -> Option<String>>,
}

#[cfg(not(target_family = "wasm"))]
struct Inner {
    preferences: Preferences,
    records: RefCell<BTreeMap<String, SavedWindow>>,
    loaded: std::cell::Cell<bool>,
    writes: futures::lock::Mutex<()>,
}

impl std::fmt::Debug for WindowState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowState")
            .finish_non_exhaustive()
    }
}

impl WindowState {
    /// Uses a preference namespace dedicated to this application.
    /// No I/O occurs until native startup or an explicit reset.
    #[must_use]
    pub fn new(preferences: Preferences) -> Self {
        #[cfg(target_family = "wasm")]
        let _ = preferences;

        Self {
            #[cfg(not(target_family = "wasm"))]
            inner: Rc::new(Inner {
                preferences,
                records: RefCell::new(BTreeMap::new()),
                loaded: std::cell::Cell::new(false),
                writes: futures::lock::Mutex::new(()),
            }),
            keys: Rc::new(|id| (id == WindowId(1)).then(|| "main".into())),
        }
    }

    /// Maps runtime windows to durable application-owned keys, or opts them out.
    ///
    /// Return `None` to disable saving and restoration for a window. Keys must be
    /// nonempty and at most 128 bytes; concurrent windows must use different keys.
    /// Duplicate/invalid keys warn and opt out. Map logical roles or document IDs;
    /// do not format the runtime ID or rely on opening order across launches.
    /// The mapping is evaluated once when each window opens, including the initial
    /// window. Keep it consistent with your secondary-window options factory.
    #[must_use]
    pub fn keys(mut self, keys: impl Fn(WindowId) -> Option<String> + 'static) -> Self {
        self.keys = Rc::new(keys);

        self
    }

    /// Forgets one saved placement; the next launch uses the app's default options.
    ///
    /// Does not move an open window. Its next move/resize or final close/quit saves
    /// its current placement again. Resets serialize with in-flight writes so an
    /// older queued save cannot resurrect the removed record. Web is a no-op.
    ///
    /// # Errors
    /// Returns preference read/write failures on native hosts.
    pub async fn reset(&self, key: &str) -> Result<(), crate::platform::Error> {
        #[cfg(target_family = "wasm")]
        {
            let _ = key;

            Ok(())
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let _write = self.inner.writes.lock().await;

            self.load_locked().await?;
            self.inner.records.borrow_mut().remove(key);

            self.save_locked().await
        }
    }

    #[cfg(not(target_family = "wasm"))]
    async fn load_locked(&self) -> Result<(), crate::platform::Error> {
        if !self.inner.loaded.get() {
            let value = self.inner.preferences.get(PREFERENCE_KEY).await?;

            *self.inner.records.borrow_mut() =
                value.as_deref().map(placement::decode).unwrap_or_default();
            self.inner.loaded.set(true);
        }

        Ok(())
    }

    #[cfg(not(target_family = "wasm"))]
    async fn save_locked(&self) -> Result<(), crate::platform::Error> {
        let value = placement::encode(&self.inner.records.borrow());

        self.inner.preferences.set(PREFERENCE_KEY, &value).await
    }

    #[cfg(all(feature = "gpui", not(target_family = "wasm")))]
    pub(crate) fn load(&self) {
        // Startup runs before GPUI's event loop; the preferences service performs
        // filesystem work on its blocking pool, never in a window callback.
        if let Err(error) = futures::executor::block_on(async {
            let _write = self.inner.writes.lock().await;

            self.load_locked().await
        }) {
            crate::warn!(%error, "window placement could not be loaded; using defaults");
        }
    }

    #[cfg(all(feature = "gpui", not(target_family = "wasm")))]
    async fn save(&self) {
        let _write = self.inner.writes.lock().await;

        if let Err(error) = self.save_locked().await {
            crate::warn!(%error, "window placement could not be saved");
        }
    }
}
