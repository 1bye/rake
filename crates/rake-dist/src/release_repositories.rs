//! Signed native repository handoff from Linux builders to the final publisher.
use super::{Input, ReleaseRequest, cancelled, fail, files, inputs, package_payload, public_root};
use crate::{Distribution, Error, Format};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema_version: u32,
    identifier: String,
    version: String,
    channel: String,
    public_root: String,
    gpg_key: Option<String>,
    repositories: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    target: String,
    format: Format,
    inputs: BTreeSet<String>,
    files: BTreeMap<PathBuf, File>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct File {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}
type Repositories = BTreeMap<(String, Format), BTreeMap<String, File>>;
fn hashes(inputs: &[Input], target: &str, format: Format) -> BTreeSet<String> {
    inputs
        .iter()
        .filter(|input| {
            input.distribution.target == target
                && input.artifact.format == format
                && package_payload(format, &input.source)
        })
        .map(|input| input.artifact.sha256.clone())
        .collect()
}
fn kind(format: Format) -> Result<String, Error> {
    Ok(serde_json::to_value(format)?
        .as_str()
        .ok_or_else(|| fail("repository format"))?
        .to_owned())
}

/// Exports signed Linux repositories without publishing or loading other OS tools.
///
/// Run on the native Linux worker after bundling. Only repositories for supplied
/// targets are prepared; final promotion still checks the complete release matrix.
/// The output must not exist. Transport it with its original distribution records
/// over trusted CI artifact storage. Only public GPG references and keys are exported.
/// # Errors
/// Rejects changed/invalid packages, existing output, missing GPG/native tools or
/// repository failures. Cancellation leaves the destination absent.
pub fn prepare_repositories(
    request: &ReleaseRequest,
    output: &Path,
    flag: &AtomicBool,
) -> Result<(), Error> {
    if output.exists() || output.is_symlink() {
        return Err(fail("repository export output must not exist"));
    }
    let mut partial = request.clone();
    let mut targets = BTreeSet::new();
    for directory in &request.distributions {
        let distribution: Distribution =
            serde_json::from_slice(&fs::read(directory.join("distribution.json"))?)?;
        targets.insert(distribution.target);
    }
    partial
        .config
        .required
        .retain(|target, _| targets.contains(target));
    partial
        .config
        .custom_required
        .retain(|target, _| targets.contains(target));
    let mut inputs = inputs(&partial)?.native;
    let parent = output
        .parent()
        .ok_or_else(|| fail("repository export needs a parent"))?;
    fs::create_dir_all(parent)?;
    let staged = tempfile::tempdir_in(parent)?;
    let packages = tempfile::tempdir_in(parent)?;
    for (index, input) in inputs.iter_mut().enumerate() {
        let snapshot = packages.path().join(index.to_string()).join(
            input
                .source
                .file_name()
                .ok_or_else(|| fail("repository package filename"))?,
        );
        files::copy(&input.source, &snapshot)?;
        if files::digest(&snapshot)? != input.artifact.sha256 {
            return Err(fail("repository package changed during copying"));
        }
        input.source = snapshot;
    }
    let mut snapshot = Snapshot {
        schema_version: 1,
        identifier: request.identifier.clone(),
        version: request.version.clone(),
        channel: request.channel.clone(),
        public_root: public_root(&request.config)?,
        gpg_key: request.config.gpg_key.clone(),
        repositories: Vec::new(),
    };
    for (target, formats) in &request.config.repositories {
        if !targets.contains(target) {
            continue;
        }
        for format in formats {
            cancelled(flag)?;
            let hashes = hashes(&inputs, target, *format);
            if hashes.is_empty() {
                return Err(fail("repository export has no native packages"));
            }
            let kind = kind(*format)?;
            let directory = staged.path().join(target).join(&kind);
            let packages = inputs
                .iter()
                .filter(|input| {
                    input.distribution.target == *target
                        && input.artifact.format == *format
                        && package_payload(*format, &input.source)
                })
                .map(|input| input.source.clone())
                .collect::<Vec<_>>();
            let paths = rake_dist_linux::build_repository(&rake_dist_linux::Repository {
                format: *format,
                identifier: &request.identifier,
                name: &format!("rake-{kind}"),
                packages: &packages,
                output: &directory,
                gpg_key: request
                    .config
                    .gpg_key
                    .as_deref()
                    .ok_or_else(|| fail("repository GPG key missing"))?,
                public_url: &format!(
                    "{}/{}/{target}/{kind}",
                    snapshot.public_root, request.channel
                ),
                branch: "master",
            })
            .map_err(|error| fail(error.to_string()))?;
            let mut files = BTreeMap::new();
            for path in paths {
                let relative = path
                    .strip_prefix(&directory)
                    .map_err(|_| fail("repository output escapes export"))?
                    .to_owned();
                let stored = path
                    .strip_prefix(staged.path())
                    .map_err(|_| fail("repository export path"))?
                    .to_owned();
                files.insert(
                    relative,
                    File {
                        path: stored,
                        sha256: files::digest(&path)?,
                        size: fs::metadata(&path)?.len(),
                    },
                );
            }
            snapshot.repositories.push(Entry {
                target: target.clone(),
                format: *format,
                inputs: hashes,
                files,
            });
        }
    }
    files::write_json(&staged.path().join("repositories.json"), &snapshot)?;
    cancelled(flag)?;
    fs::rename(staged.path(), output)?;
    Ok(())
}
pub(super) fn load(
    request: &ReleaseRequest,
    inputs: &[Input],
    directories: &[PathBuf],
) -> Result<Repositories, Error> {
    let mut result = BTreeMap::new();
    for directory in directories {
        let manifest = fs::File::open(directory.join("repositories.json"))?;
        let mut bytes = Vec::new();
        manifest
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(fail("repository snapshot exceeds manifest bound"));
        }
        let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
        if snapshot.schema_version != 1
            || snapshot.identifier != request.identifier
            || snapshot.version != request.version
            || snapshot.channel != request.channel
            || snapshot.public_root != public_root(&request.config)?
            || snapshot.gpg_key != request.config.gpg_key
        {
            return Err(fail("prepared repository context mismatch"));
        }
        for entry in snapshot.repositories {
            if !request
                .config
                .repositories
                .get(&entry.target)
                .is_some_and(|formats| formats.contains(&entry.format))
                || entry.inputs.is_empty()
                || entry.inputs != hashes(inputs, &entry.target, entry.format)
                || entry.files.is_empty()
            {
                return Err(fail("prepared repository source/coverage mismatch"));
            }
            let kind = kind(entry.format)?;
            let mut files = BTreeMap::new();
            for (relative, mut file) in entry.files {
                rake_dist_core::validate_relative_path(&relative)?;
                let path = files::confined(directory, &file.path)?;
                if !path.is_file()
                    || files::digest(&path)? != file.sha256
                    || fs::metadata(&path)?.len() != file.size
                {
                    return Err(fail("prepared repository file hash mismatch"));
                }
                file.path = path;
                files.insert(
                    format!(
                        "{}/{}/{kind}/{}",
                        request.channel,
                        entry.target,
                        relative.to_string_lossy().replace('\\', "/")
                    ),
                    file,
                );
            }
            if result.insert((entry.target, entry.format), files).is_some() {
                return Err(fail("duplicate prepared repository"));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::publish_with_repositories;
    use super::*;
    #[test]
    fn handoff_binds_context_packages_and_files_before_cross_host_promotion() {
        let root = tempfile::tempdir().unwrap();
        let mut request = super::super::tests::fixture(root.path(), "1.0.0");
        let distribution_path = request.distributions[0].join("distribution.json");
        let mut distribution: Distribution =
            serde_json::from_slice(&fs::read(&distribution_path).unwrap()).unwrap();
        fs::rename(
            request.distributions[0].join("app.tar.gz"),
            request.distributions[0].join("app.deb"),
        )
        .unwrap();
        distribution.artifacts[0].path = "app.deb".into();
        distribution.artifacts[0].format = Format::Deb;
        files::write_json(&distribution_path, &distribution).unwrap();
        let target = distribution.target.clone();
        request.config.required = BTreeMap::from([(target.clone(), vec![Format::Deb])]);
        request.config.repositories = request.config.required.clone();
        request.config.gpg_key = Some("A".repeat(40));
        let selected = inputs(&request).unwrap().native;
        let directory = root.path().join("native-export");
        fs::create_dir(&directory).unwrap();
        let index = directory.join("Packages");
        fs::write(&index, b"signed repository fixture").unwrap();
        let mut snapshot = Snapshot {
            schema_version: 1,
            identifier: request.identifier.clone(),
            version: request.version.clone(),
            channel: request.channel.clone(),
            public_root: public_root(&request.config).unwrap(),
            gpg_key: request.config.gpg_key.clone(),
            repositories: vec![Entry {
                target: target.clone(),
                format: Format::Deb,
                inputs: hashes(&selected, &target, Format::Deb),
                files: BTreeMap::from([(
                    PathBuf::from("Packages"),
                    File {
                        path: PathBuf::from("Packages"),
                        sha256: files::digest(&index).unwrap(),
                        size: fs::metadata(&index).unwrap().len(),
                    },
                )]),
            }],
        };
        let manifest = directory.join("repositories.json");
        files::write_json(&manifest, &snapshot).unwrap();
        let directories = vec![directory.clone()];
        let state =
            publish_with_repositories(&request, &directories, &AtomicBool::new(false)).unwrap();
        assert!(
            state
                .files
                .contains_key(&format!("stable/{target}/deb/Packages"))
        );
        fs::write(&index, b"changed repository").unwrap();
        assert!(load(&request, &selected, &directories).is_err());
        fs::write(&index, b"signed repository fixture").unwrap();
        snapshot.channel = "beta".into();
        files::write_json(&manifest, &snapshot).unwrap();
        assert!(load(&request, &selected, &directories).is_err());
        snapshot.channel = request.channel.clone();
        snapshot.repositories[0].inputs = BTreeSet::from(["0".repeat(64)]);
        files::write_json(&manifest, &snapshot).unwrap();
        assert!(load(&request, &selected, &directories).is_err());
        snapshot.repositories[0].inputs = hashes(&selected, &target, Format::Deb);
        snapshot.repositories[0]
            .files
            .get_mut(Path::new("Packages"))
            .unwrap()
            .path = "../escape".into();
        files::write_json(&manifest, &snapshot).unwrap();
        assert!(load(&request, &selected, &directories).is_err());
        assert!(prepare_repositories(&request, &directory, &AtomicBool::new(false)).is_err());
    }
}
