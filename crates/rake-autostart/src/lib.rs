//! Per-user launch-at-login settings without a UI or executor dependency.
//!
//! Operations are explicit and never run on construction or drop. Native filesystem
//! and registry operations are synchronous inside the async API; Flatpak requests
//! await user interaction through the background portal. See the launch-at-login guide.
//!
//! ```no_run
//! use rake_autostart::{LaunchAtLogin, Status};
//! # fn main() -> Result<(), rake_autostart::Error> {
//! let login = LaunchAtLogin::new("dev.example.app")?;
//! # let _future = async {
//! if login.status().await? == Status::Disabled {
//!     let applied = login.enable().await?;
//!     // Present RequiresApproval to the user instead of claiming success.
//!     println!("{applied:?}");
//! }
//! # Ok::<_, rake_autostart::Error>(()) };
//! # Ok(()) }
//! ```

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::remove_installed;
#[cfg(any(target_os = "windows", test))]
mod windows_command;

/// Current OS state, queried afresh rather than persisted as an application preference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Status {
    /// The OS registration exists and is eligible to start at login.
    Enabled,
    /// The app is not registered, or its Linux desktop entry is hidden.
    Disabled,
    /// The user must grant permission in system settings or the portal.
    RequiresApproval,
    /// This environment cannot perform or query the operation, with an actionable reason.
    Unsupported(String),
}

/// An invalid configuration, native I/O failure, or service rejection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Application IDs must be safe portable filenames and registry value names.
    #[error("invalid application id: use 1–128 ASCII letters, digits, dots, dashes or underscores")]
    InvalidId,
    /// A path or argument cannot be represented by the native registration format.
    #[error("invalid launch command: {0}")]
    InvalidCommand(String),
    /// Native file or registry access failed.
    #[error("launch-at-login I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// A native service or portal rejected the operation.
    #[error("launch-at-login service failed: {0}")]
    Service(String),
}

/// A stable application identity and optional login-only arguments.
///
/// Use the same ID as the distribution configuration and macOS bundle identifier.
/// Arguments are supported on Windows/Linux; macOS main-app services do not accept
/// custom arguments. No login state is changed until an operation is awaited.
#[derive(Debug, Clone)]
pub struct LaunchAtLogin {
    app_id: String,
    args: Vec<String>,
}

impl LaunchAtLogin {
    /// Constructs a setting for a stable, case-sensitive application ID.
    ///
    /// # Errors
    /// Returns [`Error::InvalidId`] for unsafe or empty IDs.
    pub fn new(app_id: impl Into<String>) -> Result<Self, Error> {
        let app_id = app_id.into();
        if app_id.is_empty()
            || app_id.len() > 128
            || app_id == "."
            || app_id == ".."
            || !app_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err(Error::InvalidId);
        }

        Ok(Self {
            app_id,
            args: Vec::new(),
        })
    }

    /// Adds arguments used only when the OS launches the app at login.
    ///
    /// # Errors
    /// Returns [`Error::InvalidCommand`] for NULs or line breaks.
    pub fn args(
        mut self,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, Error> {
        self.args = args.into_iter().map(Into::into).collect();
        if self.args.iter().any(|arg| arg.contains(['\0', '\r', '\n'])) {
            return Err(Error::InvalidCommand(
                "arguments contain NUL or line breaks".into(),
            ));
        }

        Ok(self)
    }

    /// Registers the app without bypassing OS approval or enabling it on construction.
    ///
    /// Idempotent for an already registered app. Inspect the returned status: a
    /// successful native call can still require approval. Flatpak may show a dialog.
    ///
    /// # Errors
    /// Returns native I/O, command encoding, or service errors.
    pub async fn enable(&self) -> Result<Status, Error> {
        self.apply(Operation::Enable).await
    }

    /// Unregisters the app; an absent registration is already disabled.
    ///
    /// # Errors
    /// Returns native I/O or service errors. Flatpak may require user interaction.
    pub async fn disable(&self) -> Result<Status, Error> {
        self.apply(Operation::Disable).await
    }

    /// Queries live OS state without changing login settings or prompting.
    ///
    /// Flatpak's background portal has no read-only autostart query; this returns
    /// `Unsupported` there. The result of enable/disable is authoritative for that request.
    ///
    /// # Errors
    /// Returns native I/O or service errors.
    pub async fn status(&self) -> Result<Status, Error> {
        self.apply(Operation::Status).await
    }

    async fn apply(&self, operation: Operation) -> Result<Status, Error> {
        #[cfg(target_os = "linux")]
        {
            linux::apply(self, operation).await
        }
        #[cfg(target_os = "macos")]
        {
            macos::apply(self, operation)
        }
        #[cfg(target_os = "windows")]
        {
            windows::apply(self, operation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            let _ = (operation, &self.app_id);
            Ok(Status::Unsupported(
                "launch at login is unavailable on this platform (including web/wasm)".into(),
            ))
        }
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Enable,
    Disable,
    Status,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ids_that_can_escape_the_registration_namespace() {
        for id in [
            "",
            ".",
            "..",
            "../app",
            "app/name",
            "app\\name",
            "app\nHidden=true",
        ] {
            assert!(
                matches!(LaunchAtLogin::new(id), Err(Error::InvalidId)),
                "{id:?}"
            );
        }
        assert!(LaunchAtLogin::new("dev.rake.app-1_test").is_ok());
    }
}
