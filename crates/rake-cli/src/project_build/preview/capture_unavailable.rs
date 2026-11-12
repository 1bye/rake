//! Native Swift capture is intentionally unavailable on Windows.

use super::{Error, Result};
use serde_json::Value;
use std::{io::Write, path::Path, sync::Arc};

pub(super) enum Capture {}

impl Capture {
    pub(super) fn start(_: &Path, _: u64, _: &Value, _: &str, _: &str) -> Result<Self> {
        Err(Error::new(
            400,
            "Native preview capture is available only on macOS",
        ))
    }

    pub(super) fn error(&self) -> Option<String> {
        match *self {}
    }
    pub(super) fn viewers(&self) -> usize {
        match *self {}
    }
    pub(super) fn viewer(self: &Arc<Self>) -> Viewer {
        match **self {}
    }
    pub(super) fn close(&self) {
        match *self {}
    }
}

pub(super) enum Viewer {}

impl Viewer {
    pub(super) fn stream(&self, _: &mut impl Write) -> std::io::Result<()> {
        match *self {}
    }
}
