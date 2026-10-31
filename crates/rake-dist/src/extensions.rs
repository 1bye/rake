//! Explicit native extensions share the coordinator's owned staging and process lifetime.
use crate::{Error, files};
use object::read::macho::FatArch;

use rake_dist_core::{
    BackendOutput, BackendRequest, ComponentRecipe, ExtensionError, PackageBackend,
    PreparedComponent, RecipeRequest, Verification, VerifyRequest,
};

use serde::{Deserialize, Serialize};
use std::io::Read;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// Explicit argv extension adapter. Calls run in owned process groups/Windows Jobs;
/// JSON goes through stdin/stdout and diagnostics through stderr. No shell is used.
#[derive(Debug)]
pub struct CommandExtension<'a> {
    argv: &'a [String],
    package_root: &'a Path,
    cancelled: &'a AtomicBool,
}

impl<'a> CommandExtension<'a> {
    /// Binds an explicit command to the calling job's cancellation scope.
    /// # Errors
    /// Rejects empty argv or NUL-containing arguments.
    pub fn new(
        argv: &'a [String],
        package_root: &'a Path,
        cancelled: &'a AtomicBool,
    ) -> Result<Self, ExtensionError> {
        if argv.is_empty() || argv[0].is_empty() || argv.iter().any(|arg| arg.contains('\0')) {
            return Err(ExtensionError::Contract("explicit argv is required".into()));
        }

        Ok(Self {
            argv,
            package_root,
            cancelled,
        })
    }

    fn invoke<T: serde::de::DeserializeOwned>(
        &self,
        request: &impl Serialize,
    ) -> Result<T, ExtensionError> {
        let work = tempfile::tempdir()?;
        let input = work.path().join("request.json");

        let bytes = serde_json::to_vec(request)
            .map_err(|error| ExtensionError::Contract(error.to_string()))?;

        if bytes.len() > 4 * 1024 * 1024 {
            return Err(ExtensionError::Contract(
                "extension request exceeds bound".into(),
            ));
        }

        fs::write(&input, bytes)?;
        let executable = Path::new(&self.argv[0]);

        let executable = if !executable.is_absolute() && executable.components().count() > 1 {
            self.package_root.join(executable)
        } else {
            executable.to_owned()
        };

        let mut command = Command::new(executable);

        command
            .args(&self.argv[1..])
            .current_dir(self.package_root)
            .stdin(fs::File::open(input)?)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut worker = crate::Worker::spawn_with_io(&mut command)?;

        let stdout = worker
            .take_stdout()
            .ok_or_else(|| ExtensionError::Tool("extension stdout missing".into()))?;

        let abort = AtomicBool::new(false);

        let bytes = std::thread::scope(|scope| -> Result<Vec<u8>, ExtensionError> {
            let capture = scope.spawn(|| {
                let mut bytes = Vec::new();
                let result = stdout.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes);

                if result.is_err() || bytes.len() > 4 * 1024 * 1024 {
                    abort.store(true, std::sync::atomic::Ordering::Release);
                }

                result.map(|_| bytes)
            });

            let status = worker.wait_with_abort(self.cancelled, Duration::from_secs(600), &abort);

            let bytes = capture
                .join()
                .map_err(|_| ExtensionError::Tool("extension stdout reader failed".into()))??;

            if bytes.len() > 4 * 1024 * 1024 {
                return Err(ExtensionError::Contract(
                    "extension response exceeds bound".into(),
                ));
            }

            if !status?.success() {
                return Err(ExtensionError::Tool(
                    "native extension command failed".into(),
                ));
            }

            Ok(bytes)
        })?;

        serde_json::from_slice(&bytes).map_err(|error| {
            ExtensionError::Contract(format!(
                "invalid extension JSON; logs belong on stderr: {error}"
            ))
        })
    }
}

impl PackageBackend for CommandExtension<'_> {
    fn package(&self, request: &BackendRequest) -> Result<BackendOutput, ExtensionError> {
        self.invoke(request)
    }
}

impl ComponentRecipe for CommandExtension<'_> {
    fn prepare(&self, request: &RecipeRequest) -> Result<PreparedComponent, ExtensionError> {
        self.invoke(request)
    }
}

/// One extension output whose path and bytes have been checked by the coordinator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckedArtifact {
    /// Canonical path confined to the backend's owned output root.
    pub path: PathBuf,
    /// Independently measured SHA-256 (or deterministic directory inventory).
    pub sha256: String,
}

/// Calls an explicitly registered Rust backend and checks every declared path.
/// The caller owns the input/output staging and must isolate native execution in
/// its job worker. A backend never supplies its own hash or trust declaration.
/// # Errors
/// Rejects protocol revisions, empty/duplicate/escaping output and backend failure.
pub fn run_package_backend(
    backend: &dyn PackageBackend,
    request: &BackendRequest,
) -> Result<Vec<CheckedArtifact>, Error> {
    if request.schema_version != 1 || !request.input.is_absolute() || !request.output.is_absolute()
    {
        return Err(Error::Execution("invalid package backend request".into()));
    }

    let output = backend
        .package(request)
        .map_err(|error| Error::Execution(error.to_string()))?;

    if output.schema_version != 1 || output.artifacts.is_empty() || output.artifacts.len() > 1000 {
        return Err(Error::Execution(
            "invalid backend artifact inventory".into(),
        ));
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut checked = Vec::new();

    for path in output.artifacts {
        let path = files::confined(&request.output, &path)?;

        if !seen.insert(path.clone()) {
            return Err(Error::Execution("duplicate backend artifact".into()));
        }

        files::validate_tree(&path)?;
        checked.push(CheckedArtifact {
            sha256: files::digest(&path)?,
            path,
        });
    }

    Ok(checked)
}

/// Checks a concrete Rust recipe's declared native inventory and confined output.
/// The caller supplies a copy of the original source and owns the staging scope.
/// # Errors
/// Rejects escaped paths, omitted/duplicate native code and wrong-target binaries.
pub fn run_component_recipe(
    recipe: &dyn ComponentRecipe,
    request: &RecipeRequest,
) -> Result<PathBuf, Error> {
    if request.schema_version != 1 || !request.input.is_absolute() || !request.output.is_absolute()
    {
        return Err(Error::Execution("invalid component recipe request".into()));
    }

    let prepared = recipe
        .prepare(request)
        .map_err(|error| Error::Execution(error.to_string()))?;

    if prepared.schema_version != 1 {
        return Err(Error::Execution("unknown component recipe revision".into()));
    }

    let root = files::confined(&request.output, &prepared.root)?;

    if !root.is_dir() {
        return Err(Error::Execution(
            "prepared runtime must be a directory".into(),
        ));
    }

    files::validate_tree(&root)?;
    let mut declared = std::collections::BTreeSet::new();

    for path in prepared.native_code {
        let path = files::confined(&root, &path)?;

        crate::components::architecture(&path, &request.target)?;

        if !declared.insert(path) {
            return Err(Error::Execution("duplicate recipe native code".into()));
        }
    }

    let actual = native_inventory(&root)?;

    if actual != declared {
        return Err(Error::Execution(
            "component recipe native inventory does not match prepared files".into(),
        ));
    }

    crate::recipes::inspect_tree(&root, &request.target)?;

    Ok(root)
}

fn native_inventory(root: &Path) -> Result<std::collections::BTreeSet<PathBuf>, Error> {
    let mut output = std::collections::BTreeSet::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();

        if entry.file_type()?.is_dir() {
            output.extend(native_inventory(&path)?);
        } else if entry.file_type()?.is_file() {
            let native = is_native(&path)?;

            if native {
                output.insert(fs::canonicalize(path)?);
            }
        }
    }

    Ok(output)
}

fn is_native(path: &Path) -> Result<bool, Error> {
    let mut magic = [0; 4];

    if fs::File::open(path)?.read(&mut magic)? != 4 {
        return Ok(false);
    }

    if !matches!(
        magic,
        [0x7f, b'E', b'L', b'F']
            | [b'M', b'Z', _, _]
            | [0xfe, 0xed, 0xfa, _]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xca, 0xfe, 0xba, 0xbf]
    ) {
        return Ok(false);
    }

    let bytes = fs::read(path)?;

    let thin = |data: &[u8]| {
        object::File::parse(data).is_ok_and(|file| {
            matches!(
                file.format(),
                object::BinaryFormat::Pe | object::BinaryFormat::Elf | object::BinaryFormat::MachO
            )
        })
    };

    if thin(&bytes) {
        return Ok(true);
    }

    if let Ok(fat) = object::read::macho::MachOFatFile32::parse(bytes.as_slice()) {
        return Ok(!fat.arches().is_empty()
            && fat.arches().iter().all(|arch| {
                arch.data(bytes.as_slice()).is_ok_and(|data| {
                    object::File::parse(data)
                        .is_ok_and(|file| file.format() == object::BinaryFormat::MachO)
                })
            }));
    }

    if let Ok(fat) = object::read::macho::MachOFatFile64::parse(bytes.as_slice()) {
        return Ok(!fat.arches().is_empty()
            && fat.arches().iter().all(|arch| {
                arch.data(bytes.as_slice()).is_ok_and(|data| {
                    object::File::parse(data)
                        .is_ok_and(|file| file.format() == object::BinaryFormat::MachO)
                })
            }));
    }

    Ok(false)
}

/// Completed custom format. Native trust only comes from its independent verifier.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomArtifact {
    /// Explicit configured backend slug.
    pub format: String,
    /// Path relative to the distribution root.
    pub path: PathBuf,
    /// Coordinator-measured exact bytes/inventory hash.
    pub sha256: String,
    /// Independent native verifier evidence, absent for local-only unverified output.
    pub verification: Option<Verification>,
    /// Public argv of the independent verifier, so release policy can identify it.
    pub verifier: Option<Vec<String>>,
}

pub(crate) fn package_custom(
    plan: &crate::Plan,
    target: &str,
    root: &Path,
    result: &Path,
    artifacts: &[crate::Artifact],
    trust: &rake_dist_core::TrustOptions,
) -> Result<Vec<CustomArtifact>, Error> {
    let mut output = Vec::new();
    let cancelled = AtomicBool::new(false);

    for backend in plan
        .config()
        .custom_formats
        .iter()
        .filter(|backend| backend.platform == plan.platform())
    {
        let selected = artifacts
            .iter()
            .filter(|artifact| {
                artifact.format == backend.input_format
                    && (artifact.format == crate::Format::App
                        || crate::release::package_payload(artifact.format, &artifact.path))
            })
            .collect::<Vec<_>>();

        if selected.is_empty() {
            return Err(Error::Execution(format!(
                "custom format {} requires its selected input_format",
                backend.name
            )));
        }

        for (index, artifact) in selected.into_iter().enumerate() {
            let work = tempfile::tempdir_in(
                result
                    .parent()
                    .ok_or_else(|| Error::Execution("custom stage parent".into()))?,
            )?;

            let input = work.path().join("input").join(
                artifact
                    .path
                    .file_name()
                    .ok_or_else(|| Error::Execution("custom input filename".into()))?,
            );

            files::copy(&files::confined(result, &artifact.path)?, &input)?;
            let destination = work.path().join("output");

            fs::create_dir(&destination)?;
            let references = [
                ("macos_identity", trust.macos_identity.as_ref()),
                ("notary_profile", trust.notary_profile.as_ref()),
                ("windows_thumbprint", trust.windows_thumbprint.as_ref()),
            ]
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key.into(), value.clone())))
            .collect();

            let request = BackendRequest {
                schema_version: 1,
                name: backend.name.clone(),
                identifier: plan.identifier().into(),
                version: plan.version().into(),
                target: target.into(),
                input,
                output: destination,
                tool_references: references,
            };

            let command = CommandExtension::new(&backend.command, root, &cancelled)
                .map_err(|error| Error::Execution(error.to_string()))?;

            for checked in run_package_backend(&command, &request)? {
                let verification = if let Some(argv) = &backend.verify_command {
                    let command = CommandExtension::new(argv, root, &cancelled)
                        .map_err(|error| Error::Execution(error.to_string()))?;

                    let verification: Verification = command
                        .invoke(&VerifyRequest {
                            schema_version: 1,
                            name: backend.name.clone(),
                            identifier: plan.identifier().into(),
                            target: target.into(),
                            version: plan.version().into(),
                            tool_references: request.tool_references.clone(),
                            artifact: checked.path.clone(),
                            sha256: checked.sha256.clone(),
                        })
                        .map_err(|error| Error::Execution(error.to_string()))?;

                    if verification.schema_version != 1
                        || verification.sha256 != checked.sha256
                        || verification.policy.trim().is_empty()
                        || files::digest(&checked.path)? != checked.sha256
                    {
                        return Err(Error::Execution(
                            "independent custom verifier rejected or mutated artifact".into(),
                        ));
                    }

                    Some(verification)
                } else {
                    None
                };

                let relative = PathBuf::from("custom")
                    .join(&backend.name)
                    .join(index.to_string())
                    .join(
                        checked
                            .path
                            .file_name()
                            .ok_or_else(|| Error::Execution("custom filename".into()))?,
                    );

                files::copy(&checked.path, &result.join(&relative))?;

                if files::digest(&result.join(&relative))? != checked.sha256 {
                    return Err(Error::Execution(
                        "custom output changed after verification".into(),
                    ));
                }

                output.push(CustomArtifact {
                    format: backend.name.clone(),
                    path: relative,
                    sha256: checked.sha256,
                    verification,
                    verifier: backend.verify_command.clone(),
                });
            }
        }
    }

    Ok(output)
}

pub(crate) fn recipe(
    command: &[String],
    options: std::collections::BTreeMap<String, String>,
    name: &str,
    target: &str,
    source: &Path,
    destination: &Path,
    package: &Path,
) -> Result<(), Error> {
    let work = tempfile::tempdir_in(
        destination
            .parent()
            .ok_or_else(|| Error::Execution("recipe stage parent".into()))?,
    )?;

    let input = work.path().join("input");

    files::copy(source, &input)?;
    let output = work.path().join("output");

    fs::create_dir(&output)?;
    let request = RecipeRequest {
        schema_version: 1,
        name: name.into(),
        target: target.into(),
        input,
        output,
        options,
    };

    let cancelled = AtomicBool::new(false);

    let adapter = CommandExtension::new(command, package, &cancelled)
        .map_err(|error| Error::Execution(error.to_string()))?;

    let prepared = run_component_recipe(&adapter, &request)?;

    files::copy(&prepared, destination)
}
