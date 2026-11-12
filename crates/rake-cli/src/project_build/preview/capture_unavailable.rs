//! Native Swift capture is intentionally unavailable on Windows.

use super::{Error, Result};
use serde_json::Value;
use std::{io::Write, path::Path, sync::Arc};

// No constructor succeeds on this platform; keep the adapter types inhabited
// so portable orchestration is checked without unreachable-code assumptions.
pub(super) struct Capture {
    _private: (),
}

impl Capture {
    pub(super) fn start(_: &Path, _: u64, _: &Value, _: &str, _: &str) -> Result<Self> {
        Err(Error::new(
            400,
            "Native preview capture is available only on macOS",
        ))
    }

    pub(super) fn error(&self) -> Option<String> {
        Some("Native preview capture is available only on macOS".into())
    }

    pub(super) fn viewers(&self) -> usize {
        0
    }
    pub(super) fn viewer(self: &Arc<Self>) -> Viewer {
        Viewer { _private: () }
    }
    pub(super) fn close(&self) {}
}

pub(super) struct Viewer {
    _private: (),
}

impl Viewer {
    pub(super) fn stream(&self, _: &mut impl Write) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Native preview capture is available only on macOS",
        ))
    }
}
