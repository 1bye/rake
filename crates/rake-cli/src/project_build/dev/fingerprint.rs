//! Content fingerprints with the Python supervisor's ordering and exclusions.

use super::super::{Project, Result};
use sha2::{Digest, Sha256};

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const EXCLUDED: &[&str] = &[
    ".git",
    "target",
    "dist",
    "node_modules",
    ".rake",
    "__pycache__",
    ".DS_Store",
];

pub(super) fn fingerprint(project: &Project, output: Option<&Path>) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();
    let mut visited = HashSet::new();

    for root in &project.watch_roots {
        walk(root, output, &mut visited, &mut digest)?;
    }

    Ok(digest.finalize().into())
}

fn walk(
    root: &Path,
    output: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
    digest: &mut Sha256,
) -> Result<()> {
    // os.walk ignores inaccessible or concurrently deleted directories.
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(());
    };

    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();

        if EXCLUDED.iter().any(|name| entry.file_name() == *name)
            || path.is_symlink()
            || reparse(&path)
            || output.is_some_and(|output| path.starts_with(output))
        {
            continue;
        }

        if path.is_dir() {
            directories.push(path);
        } else {
            files.push(path);
        }
    }

    files.sort();
    directories.sort();

    for path in files {
        if !visited.insert(path.clone()) {
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            digest.update(path.as_os_str().as_bytes());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            for unit in path.as_os_str().encode_wide() {
                digest.update(unit.to_le_bytes());
            }
        }

        match fs::File::open(&path) {
            Ok(mut file) => {
                let mut buffer = [0; 65536];

                loop {
                    let count = file.read(&mut buffer)?;

                    if count == 0 {
                        break;
                    }

                    digest.update(&buffer[..count]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                digest.update(b"deleted-during-scan")
            }
            Err(error) => return Err(error.into()),
        }
    }

    for directory in directories {
        walk(&directory, output, visited, digest)?;
    }

    Ok(())
}

fn reparse(path: &Path) -> bool {
    #[cfg(unix)]
    {
        let _ = path;
        false
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        fs::symlink_metadata(path).is_ok_and(|metadata| {
            metadata.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
        })
    }
}
