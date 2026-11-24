//! Managed work and GPUI visibility observation.
pub use rake_core::tasks::*;

pub(crate) mod host;

/// Reports the managed host's window observation support.
pub fn visibility_support() -> VisibilitySupport {
    if cfg!(target_os = "macos") {
        VisibilitySupport::NativeOcclusion
    } else if cfg!(target_os = "windows") {
        VisibilitySupport::NativeWindowState
    } else if cfg!(target_family = "wasm") {
        VisibilitySupport::BrowserDocument
    } else {
        VisibilitySupport::Unknown
    }
}
