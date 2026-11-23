use super::{Capability, PermissionState, Platform, Request};

/// Global shortcut service. Enable `global-shortcuts` (also enabled by `tray`).
#[derive(Clone, Debug, Default)]
pub struct GlobalShortcuts;

impl Platform {
    /// Returns the global shortcut service, including on unsupported hosts.
    pub fn global_shortcuts(&self) -> GlobalShortcuts {
        self.shortcuts.clone()
    }
}

impl GlobalShortcuts {
    pub(super) fn capability(&self) -> Capability {
        Capability {
            supported: false,
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
        Box::pin(async { Ok(false) })
    }
}
