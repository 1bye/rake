//! Unix modes for authored Web inputs; Windows ZIP payloads use regular-file modes.

use std::{fs, io, path::Path};

pub(super) fn set(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }

    #[cfg(windows)]
    {
        let _ = mode;

        fs::metadata(path).map(|_| ())
    }
}

pub(super) fn archive_mode(path: &Path) -> io::Result<u32> {
    let metadata = fs::metadata(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        Ok(metadata.permissions().mode())
    }

    #[cfg(windows)]
    {
        Ok(if metadata.is_dir() { 0o755 } else { 0o644 })
    }
}
