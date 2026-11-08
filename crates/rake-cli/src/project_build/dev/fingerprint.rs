//! Content fingerprints with the Python supervisor's ordering and exclusions.

use super::super::{Project, Result};
use sha2::{Digest, Sha256};

use std::{
    collections::{BTreeMap, HashSet},
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
        walk(root, output, &mut visited, &mut digest, None)?;
    }

    Ok(digest.finalize().into())
}

fn walk(
    root: &Path,
    output: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
    digest: &mut Sha256,
    mut stamps: Option<&mut Stamps>,
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

        if let Some(stamps) = &mut stamps {
            if let Ok(metadata) = path.metadata() {
                #[cfg(unix)]
                use std::os::unix::fs::MetadataExt;
                #[cfg(windows)]
                use std::os::windows::fs::MetadataExt;
                #[cfg(unix)]
                let stamp = (metadata.mtime(), metadata.mtime_nsec(), metadata.len(), metadata.ino());
                #[cfg(windows)]
                let stamp = (metadata.last_write_time() as i64, 0, metadata.file_size(), 0);
                stamps.insert(
                    path,
                    stamp,
                );
            }
            continue;
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
        walk(&directory, output, visited, digest, stamps.as_deref_mut())?;
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

pub(super) type Stamps = BTreeMap<PathBuf, (i64, i64, u64, u64)>;

pub(super) fn stamps(project: &Project, output: Option<&Path>) -> Result<Stamps> {
    let mut digest = Sha256::new();
    let mut visited = HashSet::new();
    let mut stamps = Stamps::new();
    for root in &project.watch_roots {
        walk(root, output, &mut visited, &mut digest, Some(&mut stamps))?;
    }
    Ok(stamps)
}
