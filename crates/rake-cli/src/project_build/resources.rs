//! Materialize only non-Python resources required by owned development services.

use std::{fs, path::Path};

pub(super) fn extract(directory: &Path) -> super::Result<()> {
    for (name, source) in [
        ("preview.swift", include_str!("../../scripts/preview.swift")),
        (
            "embedded-devtools.mjs",
            include_str!("../../scripts/embedded-devtools.mjs"),
        ),
        (
            "embedded-devtools.html",
            include_str!("../../scripts/embedded-devtools.html"),
        ),
    ] {
        fs::write(directory.join(name), source)?;
    }

    Ok(())
}
