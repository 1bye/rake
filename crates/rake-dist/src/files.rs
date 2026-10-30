//! Confined input copying, deterministic inventories and owned output replacement.
use crate::Error;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn copy(source: &Path, destination: &Path) -> Result<(), Error> {
    let root = dunce::canonicalize(source)?;
    copy_under(source, destination, &root)
}

fn copy_under(source: &Path, destination: &Path, root: &Path) -> Result<(), Error> {
    let canonical = dunce::canonicalize(source)?;
    if !canonical.starts_with(root) {
        return Err(Error::Execution(format!(
            "symlink escapes input: {}",
            source.display()
        )));
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err(Error::Execution(format!(
            "destination collision: {}",
            destination.display()
        )));
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;
        if target.is_absolute() {
            return Err(Error::Execution("absolute component symlink".into()));
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination)?;
        #[cfg(windows)]
        if canonical.is_dir() {
            std::os::windows::fs::symlink_dir(target, destination)?;
        } else {
            std::os::windows::fs::symlink_file(target, destination)?;
        }
    } else if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_under(&entry.path(), &destination.join(entry.file_name()), root)?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    } else {
        return Err(Error::Execution("special files cannot be packaged".into()));
    }
    Ok(())
}

pub(crate) fn confined(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    rake_dist_core::validate_relative_path(path)?;
    let root = dunce::canonicalize(root)?;
    let resolved = dunce::canonicalize(root.join(path))?;
    if !resolved.starts_with(&root) {
        return Err(Error::Execution(format!(
            "input escapes package root: {}",
            path.display()
        )));
    }
    Ok(resolved)
}

pub(crate) fn digest(path: &Path) -> Result<String, Error> {
    let mut hash = Sha256::new();
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        hash.update(fs::read_link(path)?.as_os_str().as_encoded_bytes());
    } else if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            hash.update(entry.file_name().as_encoded_bytes());
            hash.update(digest(&entry.path())?.as_bytes());
        }
    } else if fs::metadata(path)?.is_file() {
        let mut file = fs::File::open(path)?;
        let mut bytes = [0; 65536];
        loop {
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
    } else {
        return Err(Error::Execution("special files cannot be hashed".into()));
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), Error> {
    let mut file = fs::File::create(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn commit(staged: &Path, output: &Path, identifier: &str) -> Result<(), Error> {
    commit_compatible(staged, output, identifier, None)
}

pub(crate) fn commit_compatible(
    staged: &Path,
    output: &Path,
    identifier: &str,
    legacy_package: Option<&str>,
) -> Result<(), Error> {
    let marker = ".rake-distribution";
    if output.exists()
        && !legacy_package.is_some_and(|package| crate::compatibility::owns(output, package))
        && (output.is_symlink()
            || fs::read_to_string(output.join(marker)).ok().as_deref() != Some(identifier))
    {
        return Err(Error::Execution(format!(
            "refusing unowned output {}",
            output.display()
        )));
    }
    fs::write(staged.join(marker), identifier)?;
    let backup = tempfile::Builder::new()
        .prefix(".rake-backup-")
        .tempdir_in(
            output
                .parent()
                .ok_or_else(|| Error::Execution("output needs parent".into()))?,
        )?;
    let previous = backup.path().join("previous");
    if output.exists() {
        fs::rename(output, &previous)?;
    }
    if let Err(error) = fs::rename(staged, output) {
        if previous.exists()
            && let Err(rollback) = fs::rename(&previous, output)
        {
            let retained = backup.keep();
            return Err(Error::Execution(format!(
                "output replacement failed: {error}; rollback failed: {rollback}; previous output retained in {}",
                retained.display()
            )));
        }
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn validate_tree(root: &Path) -> Result<(), Error> {
    let canonical = dunce::canonicalize(root)?;
    let mut count = 0;
    fn inspect(path: &Path, root: &Path, count: &mut usize) -> Result<(), Error> {
        *count += 1;
        if *count > 100_000 {
            return Err(Error::Execution(
                "native output exceeds file inventory bound".into(),
            ));
        }
        let actual = dunce::canonicalize(path)?;
        if !actual.starts_with(root) {
            return Err(Error::Execution(
                "native output symlink escapes staging".into(),
            ));
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                inspect(&entry?.path(), root, count)?;
            }
        } else if !metadata.is_file() {
            return Err(Error::Execution(
                "native output contains a special file".into(),
            ));
        }
        Ok(())
    }
    inspect(root, &canonical, &mut count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_replacement_preserves_unowned_files() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        let output = root.path().join("output");
        fs::create_dir(&stage).unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(output.join("valuable"), "keep").unwrap();
        assert!(commit(&stage, &output, "dev.rake.fixture").is_err());
        assert_eq!(fs::read_to_string(output.join("valuable")).unwrap(), "keep");
        fs::write(output.join(".rake-distribution"), "dev.rake.fixture").unwrap();
        fs::write(stage.join("new"), "replacement").unwrap();
        commit(&stage, &output, "dev.rake.fixture").unwrap();
        assert_eq!(
            fs::read_to_string(output.join("new")).unwrap(),
            "replacement"
        );
    }

    #[cfg(unix)]
    #[test]
    fn input_tree_cannot_copy_an_escaping_symlink() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("input")).unwrap();
        fs::write(root.path().join("outside"), "secret").unwrap();
        std::os::unix::fs::symlink("../outside", root.path().join("input/escape")).unwrap();
        assert!(copy(&root.path().join("input"), &root.path().join("copy")).is_err());
    }
}
