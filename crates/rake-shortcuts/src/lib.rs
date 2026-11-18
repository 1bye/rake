//! Owned global shortcuts. Native events are queued; callers choose their UI dispatcher.
//! macOS and Windows registration, event pumping and destruction belong on the UI thread.

use futures::channel::mpsc;
use rake_tray::Accelerator;
use std::marker::PhantomData;
use std::rc::Rc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as native;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(target_os = "windows")]
use windows as native;

/// Registration failed; no alternative shortcut is silently chosen.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No backend is available in this host/session.
    #[error("global shortcuts are unsupported in this host/session")]
    Unsupported,
    /// Parsing or the native key mapping rejected the accelerator.
    #[error("invalid global shortcut: {0}")]
    InvalidInput(String),
    /// Another registration already owns this shortcut.
    #[error("global shortcut is already registered")]
    Conflict,
    /// Authorization was refused.
    #[error("global shortcut permission denied")]
    PermissionDenied,
    /// An OS failure, including its diagnostic.
    #[error("global shortcut failed: {0}")]
    Native(String),
}

/// A thread-bound registration. Drop releases its native resources and stops events.
/// Use `unregister` to observe cleanup errors and await portal session closure.
pub struct Shortcut {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    native: Option<native::Registration>,
    events: mpsc::Receiver<()>,
    _thread: PhantomData<Rc<()>>,
}

impl Shortcut {
    /// Registers a tray-parser accelerator. Poll on the native UI thread.
    /// On Wayland the portal may prompt and choose a user-approved binding.
    ///
    /// # Errors
    /// Reports invalid keys, conflicts, denied permission and unavailable backends.
    pub async fn register(value: &str) -> Result<Self, Error> {
        let accelerator =
            Accelerator::parse(value).map_err(|error| Error::InvalidInput(error.to_string()))?;

        if accelerator.key.chars().count() != 1 {
            return Err(Error::InvalidInput(
                "key normalization produced multiple characters".into(),
            ));
        }

        if !(accelerator.control || accelerator.alt || accelerator.super_key) {
            return Err(Error::InvalidInput(
                "use Ctrl, Alt or Super with a key".into(),
            ));
        }

        let (sender, events) = mpsc::channel::<()>(1);

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let native = native::register(accelerator, sender).await?;

            Ok(Self {
                native: Some(native),
                events,
                _thread: PhantomData,
            })
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = (accelerator, sender, events);

            Err(Error::Unsupported)
        }
    }

    /// Takes the activation inbox for a separate UI dispatch task.
    /// Subsequent calls return a closed inbox.
    pub fn take_events(&mut self) -> mpsc::Receiver<()> {
        std::mem::replace(&mut self.events, mpsc::channel(1).1)
    }

    /// Waits for an activation; a small bounded inbox coalesces repeated presses.
    pub async fn next(&mut self) -> Option<()> {
        use futures::StreamExt;

        self.events.next().await
    }

    /// Releases the registration. Repeated calls are harmless.
    ///
    /// # Errors
    /// Reports native cleanup failures; Drop still attempts resource release.
    pub async fn unregister(&mut self) -> Result<(), Error> {
        self.events.close();

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(mut native) = self.native.take() {
            native.unregister().await?;
        }

        Ok(())
    }
}

/// Detects backend availability without registering keys or prompting.
pub async fn available() -> bool {
    #[cfg(target_os = "linux")]
    return linux::available().await;

    #[cfg(not(target_os = "linux"))]
    cfg!(any(target_os = "macos", target_os = "windows"))
}

#[cfg(test)]
mod tests {
    //! Covers: parser validation before native registration.
    //! Real: accelerator parser. Doubles: none. Requires: no graphical session.
    use super::*;

    #[test]
    fn invalid_shortcuts_never_register() {
        for value in [
            "",
            "Ctrl+Ctrl+k",
            "Hyper+k",
            "Ctrl+Enter",
            "Ctrl+İ",
            "k",
            "Shift+k",
        ] {
            assert!(matches!(
                futures::executor::block_on(Shortcut::register(value)),
                Err(Error::InvalidInput(_))
            ));
        }
    }
}
