//! Owner-only development files. Validation is performed on the opened object.

use std::{fs::File, io, path::Path};

#[cfg(windows)]
#[path = "private/windows.rs"]
mod windows;

#[cfg(windows)]
pub(crate) use windows::{Security, validate_security};
#[cfg(windows)]
pub use windows::{open_source, replace, write_atomic};

/// Creates a new private regular file without replacing an existing object.
///
/// # Errors
/// Returns filesystem errors, including existing paths and unsafe path components.
pub fn create(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
    }

    #[cfg(windows)]
    {
        windows::create(path)
    }
}

/// Opens an owned private regular file without following its final link.
///
/// # Errors
/// Returns filesystem errors or `PermissionDenied` for unsafe objects.
pub fn open(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;

        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };

        if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file must be an owned private regular file",
            ));
        }

        Ok(file)
    }

    #[cfg(windows)]
    {
        windows::open(path)
    }
}

/// Exclusively creates a current-user-only directory, rejecting existing objects.
///
/// # Errors
/// Returns filesystem errors, including existing paths and unsafe components.
pub fn create_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        std::fs::DirBuilder::new().mode(0o700).create(path)
    }

    #[cfg(windows)]
    {
        windows::create_directory(path)
    }
}

/// Creates or validates an owned private directory without following its final link.
///
/// # Errors
/// Returns filesystem errors or `PermissionDenied` for unsafe objects.
pub fn prepare_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};

        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }

        let metadata = std::fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };

        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "control directory must be an owned directory with mode 0700",
            ));
        }

        Ok(())
    }

    #[cfg(windows)]
    {
        windows::prepare_directory(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_round_trip_and_never_replace_existing_objects() {
        let root = std::env::temp_dir().join(format!(
            "rake-private-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        prepare_directory(&root).unwrap();
        assert_eq!(
            create_directory(&root).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        create_directory(&root.join("child")).unwrap();
        prepare_directory(&root.join("child")).unwrap();
        let path = root.join("private.json");

        std::io::Write::write_all(&mut create(&path).unwrap(), b"secret").unwrap();
        assert_eq!(
            std::io::read_to_string(open(&path).unwrap()).unwrap(),
            "secret"
        );
        assert!(create(&path).is_err());
        assert!(open(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn public_files_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = std::env::temp_dir().join(format!("rake-private-links-{}", std::process::id()));

        prepare_directory(&root).unwrap();
        let path = root.join("private");

        drop(create(&path).unwrap());
        symlink(&path, root.join("link")).unwrap();
        assert!(open(&root.join("link")).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
