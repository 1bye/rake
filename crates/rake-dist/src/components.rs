//! Exact-target component resolution with bounded, hash-verified archive staging.
use crate::{Error, files};
use fs2::FileExt;
use rake_dist_core::{Archive, Component, ComponentSource};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::{Component as PathComponent, Path, PathBuf},
    time::Duration,
};

const UNPACK_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
const FILE_LIMIT: usize = 100_000;

/// Reproducible component resolution record shipped with distribution artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentLock {
    /// Stable component name.
    pub name: String,
    /// Exact target triple.
    pub target: String,
    /// SHA-256 of the pinned input file or local input tree.
    pub source_sha256: String,
    /// SHA-256 of the prepared component tree, including relative symlinks.
    pub tree_sha256: String,
}

pub(crate) struct Prepared {
    pub roots: BTreeMap<String, PathBuf>,
    pub locks: Vec<ComponentLock>,
}

pub(crate) fn check_lock(
    resolved: &[ComponentLock],
    path: &Path,
    required: bool,
) -> Result<(), Error> {
    if required && !resolved.is_empty() && !path.is_file() {
        return Err(Error::Execution(
            "--locked-components requires rake-dist.lock for selected native components".into(),
        ));
    }
    if path.exists() {
        let locked: Vec<ComponentLock> = serde_json::from_slice(&fs::read(path)?)?;
        for component in resolved {
            if !locked.contains(component) {
                return Err(Error::Execution(format!(
                    "component {} differs from rake-dist.lock",
                    component.name
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn resolve(
    components: &[Component],
    target: &str,
    package: &Path,
    stage: &Path,
    cache: &Path,
    offline: bool,
) -> Result<Prepared, Error> {
    fs::create_dir_all(stage)?;
    fs::create_dir_all(cache)?;
    let mut prepared = Prepared {
        roots: BTreeMap::new(),
        locks: Vec::new(),
    };
    for component in components
        .iter()
        .filter(|component| component.target == target)
    {
        let destination = stage.join(&component.name);
        let source_sha256 = match &component.source {
            ComponentSource::Local { path } => {
                let source = files::confined(package, path)?;
                let hash = files::digest(&source)?;
                if source.is_file() {
                    fs::create_dir_all(&destination)?;
                    files::copy(&source, &destination.join("payload"))?;
                } else {
                    files::copy(&source, &destination)?;
                }
                hash
            }
            ComponentSource::Https {
                url,
                sha256,
                archive,
                max_download_bytes,
            } => {
                let payload = download(url, sha256, *max_download_bytes, cache, offline)?;
                unpack(&payload, *archive, &destination)?;
                sha256.clone()
            }
        };
        prepared.locks.push(ComponentLock {
            name: component.name.clone(),
            target: target.into(),
            source_sha256,
            tree_sha256: files::digest(&destination)?,
        });
        prepared.roots.insert(component.name.clone(), destination);
    }
    Ok(prepared)
}

pub(crate) fn download(
    url: &str,
    expected: &str,
    limit: u64,
    cache: &Path,
    offline: bool,
) -> Result<PathBuf, Error> {
    let destination = cache.join(expected);
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(cache.join(format!("{expected}.lock")))?;
    lock.lock_exclusive()?;
    if destination.exists() {
        if destination.is_symlink()
            || !destination.is_file()
            || fs::metadata(&destination)?.len() > limit
            || files::digest(&destination)? != expected
        {
            return Err(Error::Execution(
                "component cache integrity mismatch; remove the corrupt entry".into(),
            ));
        }
        return Ok(destination);
    }
    if offline {
        return Err(Error::Execution(format!(
            "component {expected} is not cached; offline resolution cannot download"
        )));
    }
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .max_redirects(5)
        .https_only(true)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| Error::Execution(format!("component download: {e}")))?;
    let mut temporary = tempfile::NamedTempFile::new_in(cache)?;
    let copied = io::copy(
        &mut response.body_mut().as_reader().take(limit + 1),
        temporary.as_file_mut(),
    )?;
    if copied > limit {
        return Err(Error::Execution(
            "component exceeds configured download bound".into(),
        ));
    }
    temporary.as_file().sync_all()?;
    if files::digest(temporary.path())? != expected {
        return Err(Error::Execution(
            "downloaded component SHA-256 mismatch".into(),
        ));
    }
    temporary
        .persist_noclobber(&destination)
        .map_err(|e| Error::Io(e.error))?;
    Ok(destination)
}

fn normalized(path: &Path) -> Result<PathBuf, Error> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            PathComponent::Normal(part) => {
                let text = part
                    .to_str()
                    .ok_or_else(|| Error::Execution("non-UTF8 archive path".into()))?;
                if text.contains(['\\', ':']) || text.chars().any(char::is_control) {
                    return Err(Error::Execution("nonportable archive path".into()));
                }
                result.push(part);
            }
            PathComponent::CurDir => {}
            _ => {
                return Err(Error::Execution(
                    "archive path escapes component root".into(),
                ));
            }
        }
    }
    Ok(result)
}

fn output_file(root: &Path, path: &Path) -> Result<fs::File, Error> {
    let destination = root.join(normalized(path)?);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?)
}

fn write_payload(
    root: &Path,
    path: &Path,
    reader: &mut impl Read,
    size: u64,
    mode: Option<u32>,
) -> Result<(), Error> {
    let mut file = output_file(root, path)?;
    let count = io::copy(&mut reader.take(size + 1), &mut file)?;
    if count != size {
        return Err(Error::Execution("archive entry size mismatch".into()));
    }
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(
            if mode.unwrap_or(0) & 0o111 != 0 {
                0o755
            } else {
                0o644
            },
        ))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

fn unpack(payload: &Path, archive: Archive, destination: &Path) -> Result<(), Error> {
    fs::create_dir_all(destination)?;
    match archive {
        Archive::File => files::copy(payload, &destination.join("payload"))?,
        Archive::Zip => {
            let mut zip = zip::ZipArchive::new(fs::File::open(payload)?)
                .map_err(|e| Error::Execution(e.to_string()))?;
            if zip.len() > FILE_LIMIT {
                return Err(Error::Execution("too many archive entries".into()));
            }
            let mut total = 0_u64;
            for index in 0..zip.len() {
                let mut entry = zip
                    .by_index(index)
                    .map_err(|e| Error::Execution(e.to_string()))?;
                let name = entry
                    .name()
                    .map_err(|e| Error::Execution(e.to_string()))?
                    .into_owned();
                let path = normalized(Path::new(&name))?;
                total = total
                    .checked_add(entry.size())
                    .ok_or_else(|| Error::Execution("archive size overflow".into()))?;
                if total > UNPACK_LIMIT {
                    return Err(Error::Execution("unpacked component exceeds 4 GiB".into()));
                }
                let mode = entry.unix_mode();
                if mode.is_some_and(|mode| mode & 0o170000 == 0o120000) {
                    return Err(Error::Execution(
                        "ZIP symlinks are unsupported; use a tar archive for frameworks".into(),
                    ));
                }
                if entry.is_dir() {
                    fs::create_dir_all(destination.join(path))?;
                } else {
                    let size = entry.size();
                    write_payload(destination, &path, &mut entry, size, mode)?;
                }
            }
        }
        Archive::TarGz => unpack_tar(
            flate2::read::GzDecoder::new(fs::File::open(payload)?),
            destination,
        )?,
        Archive::TarXz => {
            let stream = liblzma::stream::Stream::new_stream_decoder(128 * 1024 * 1024, 0)
                .map_err(|error| Error::Execution(error.to_string()))?;
            unpack_tar(
                liblzma::read::XzDecoder::new_stream(fs::File::open(payload)?, stream),
                destination,
            )?;
        }
    }
    Ok(())
}

fn unpack_tar(reader: impl Read, destination: &Path) -> Result<(), Error> {
    let mut archive = tar::Archive::new(reader);
    let mut total = 0_u64;
    let mut links = Vec::new();
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= FILE_LIMIT {
            return Err(Error::Execution("too many archive entries".into()));
        }
        let mut entry = entry?;
        let path = normalized(&entry.path()?)?;
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| Error::Execution("archive size overflow".into()))?;
        if total > UNPACK_LIMIT {
            return Err(Error::Execution("unpacked component exceeds 4 GiB".into()));
        }
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(destination.join(path))?;
        } else if kind.is_file() {
            let size = entry.size();
            let mode = entry.header().mode()?;
            write_payload(destination, &path, &mut entry, size, Some(mode))?;
        } else if kind.is_symlink() {
            let target = entry
                .link_name()?
                .ok_or_else(|| Error::Execution("symlink target missing".into()))?
                .into_owned();
            if target.is_absolute() || target.to_string_lossy().contains(['\\', ':']) {
                return Err(Error::Execution("absolute archive symlink".into()));
            }
            let mut depth = path.parent().map_or(0, |p| p.components().count());
            for part in target.components() {
                match part {
                    PathComponent::Normal(_) => depth += 1,
                    PathComponent::CurDir => {}
                    PathComponent::ParentDir if depth > 0 => depth -= 1,
                    _ => return Err(Error::Execution("archive symlink escapes root".into())),
                }
            }
            links.push((path, target));
        } else {
            return Err(Error::Execution(
                "special/hardlinked archive entry is unsupported".into(),
            ));
        }
    }
    // Create links only after ordinary entries so a link can never redirect writes.
    for (path, target) in links {
        for ancestor in path
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            if fs::symlink_metadata(destination.join(ancestor))
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(Error::Execution("archive link has symlink ancestor".into()));
            }
        }
        let output = destination.join(&path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, output)?;
        #[cfg(windows)]
        {
            let _ = (target, output);
            return Err(Error::Execution(
                "tar symlinks require a Unix component target".into(),
            ));
        }
    }
    // Validate link chains, including cycles and links through other links.
    validate_tree(destination, destination)?;
    Ok(())
}

fn validate_tree(root: &Path, path: &Path) -> Result<(), Error> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let canonical = fs::canonicalize(&path)?;
        if !canonical.starts_with(root) {
            return Err(Error::Execution("component link chain escapes root".into()));
        }
        if !entry.file_type()?.is_symlink() && entry.file_type()?.is_dir() {
            validate_tree(root, &path)?;
        }
    }
    Ok(())
}

pub(crate) fn architecture(path: &Path, target: &str) -> Result<(), Error> {
    if target == "universal-apple-darwin" {
        architecture(path, "aarch64-apple-darwin")?;
        return architecture(path, "x86_64-apple-darwin");
    }
    use object::{Object, read::macho::FatArch};
    if fs::metadata(path)?.len() > 512 * 1024 * 1024 {
        return Err(Error::Execution(
            "native component exceeds inspection bound".into(),
        ));
    }
    let bytes = fs::read(path)?;
    let architecture = target.split('-').next().unwrap_or("");
    let expected = match architecture {
        "aarch64" => object::Architecture::Aarch64,
        "x86_64" => object::Architecture::X86_64,
        "i686" | "i586" => object::Architecture::I386,
        value if value.starts_with("arm") => object::Architecture::Arm,
        _ => return Err(Error::Execution("unsupported component CPU".into())),
    };
    let format = match rake_dist_core::Platform::from_target(target)? {
        rake_dist_core::Platform::Macos => object::BinaryFormat::MachO,
        rake_dist_core::Platform::Windows => object::BinaryFormat::Pe,
        rake_dist_core::Platform::Linux => object::BinaryFormat::Elf,
        _ => {
            return Err(Error::Execution(
                "unsupported native component platform".into(),
            ));
        }
    };
    let result = match object::FileKind::parse(bytes.as_slice()) {
        Ok(object::FileKind::MachOFat32) if format == object::BinaryFormat::MachO => {
            object::read::macho::MachOFatFile32::parse(bytes.as_slice()).map(|fat| {
                fat.arches().iter().any(|arch| {
                    arch.architecture() == expected
                        && arch
                            .data(bytes.as_slice())
                            .ok()
                            .and_then(|data| object::File::parse(data).ok())
                            .is_some_and(|file| {
                                file.architecture() == expected && file.format() == format
                            })
                })
            })
        }
        Ok(object::FileKind::MachOFat64) if format == object::BinaryFormat::MachO => {
            object::read::macho::MachOFatFile64::parse(bytes.as_slice()).map(|fat| {
                fat.arches().iter().any(|arch| {
                    arch.architecture() == expected
                        && arch
                            .data(bytes.as_slice())
                            .ok()
                            .and_then(|data| object::File::parse(data).ok())
                            .is_some_and(|file| {
                                file.architecture() == expected && file.format() == format
                            })
                })
            })
        }
        _ => object::File::parse(bytes.as_slice())
            .map(|file| file.architecture() == expected && file.format() == format),
    }
    .map_err(|e| Error::Execution(format!("native component {}: {e}", path.display())))?;
    if !result {
        return Err(Error::Execution(format!(
            "component target mismatch: {} for {target}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_cpu_cannot_accept_another_operating_systems_binary() {
        let executable = std::env::current_exe().unwrap();
        let cpu = std::env::consts::ARCH;
        let target = if cfg!(windows) {
            format!("{cpu}-unknown-linux-gnu")
        } else {
            format!("{cpu}-pc-windows-msvc")
        };
        assert!(architecture(&executable, &target).is_err());
    }
    #[test]
    fn archive_paths_and_cached_payloads_are_verified() {
        for path in ["../outside", "/absolute", "C:\\escape", "one/../../two"] {
            assert!(normalized(Path::new(path)).is_err());
        }
        let root = tempfile::tempdir().unwrap();
        let hash = "a".repeat(64);
        fs::write(root.path().join(&hash), "corrupt").unwrap();
        assert!(download("https://example.invalid/", &hash, 100, root.path(), true).is_err());
        assert!(
            download(
                "https://example.invalid/",
                &"b".repeat(64),
                100,
                root.path(),
                true
            )
            .is_err()
        );
    }
    #[test]
    fn tar_symlink_cannot_redirect_an_extraction_write() {
        let directory = tempfile::tempdir().unwrap();
        let mut bytes = Vec::new();
        {
            let mut archive = tar::Builder::new(&mut bytes);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_cksum();
            archive
                .append_link(&mut header, "escape", "../outside")
                .unwrap();
            archive.finish().unwrap();
        }
        assert!(unpack_tar(bytes.as_slice(), directory.path()).is_err());
        assert!(!directory.path().join("escape").exists());
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    #[test]
    fn verified_cached_zip_unpacks_offline_and_records_tree() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut bytes);
            archive
                .start_file(
                    "payload/message.txt",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive.write_all(b"verified component").unwrap();
            archive.finish().unwrap();
        }
        let payload = directory.path().join("payload.zip");
        fs::write(&payload, bytes.into_inner()).unwrap();
        let hash = files::digest(&payload).unwrap();
        fs::rename(payload, cache.join(&hash)).unwrap();
        let components = vec![Component {
            name: "media".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            source: ComponentSource::Https {
                url: "https://example.invalid/payload.zip".into(),
                sha256: hash.clone(),
                archive: Archive::Zip,
                max_download_bytes: 1024,
            },
            licenses: Vec::new(),
        }];
        let result = resolve(
            &components,
            "x86_64-unknown-linux-gnu",
            directory.path(),
            &directory.path().join("stage"),
            &cache,
            true,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(result.roots["media"].join("payload/message.txt")).unwrap(),
            "verified component"
        );
        assert_eq!(result.locks[0].source_sha256, hash);
        let other = resolve(
            &components,
            "aarch64-unknown-linux-gnu",
            directory.path(),
            &directory.path().join("other"),
            &cache,
            true,
        )
        .unwrap();
        assert!(other.roots.is_empty());
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    #[test]
    fn empty_component_plans_need_no_lock_but_selected_inputs_remain_pinned() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rake-dist.lock");
        check_lock(&[], &path, true).unwrap();
        let component = ComponentLock {
            name: "media".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            source_sha256: "1".repeat(64),
            tree_sha256: "2".repeat(64),
        };
        assert!(check_lock(std::slice::from_ref(&component), &path, true).is_err());
        files::write_json(&path, &vec![component.clone()]).unwrap();
        check_lock(std::slice::from_ref(&component), &path, true).unwrap();
        let changed = ComponentLock {
            tree_sha256: "3".repeat(64),
            ..component
        };
        assert!(check_lock(&[changed], &path, true).is_err());
    }
}
