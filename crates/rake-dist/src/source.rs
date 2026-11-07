//! Standalone configuration selection, strict schema validation and read-only plans.

use crate::{Config, Error, Format, Metadata, Package, Plan, Platform, files};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Reserved prebuilt component cache directory, never an application payload input.
pub(crate) const PREBUILT_COMPONENT_CACHE: &str = ".rake-component-cache";

/// Main executable input. Distribution settings remain in [`Config`].
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
#[non_exhaustive]
pub enum Program {
    /// One Cargo package and binary, resolved without building.
    Cargo {
        /// Root-relative Cargo manifest; defaults to Cargo.toml.
        #[serde(default = "default_manifest")]
        manifest: PathBuf,
        /// Required when workspace package selection is ambiguous.
        #[serde(default)]
        package: Option<String>,
        /// Required when the package has multiple binaries.
        #[serde(default)]
        bin: Option<String>,
        /// Cargo features, augmented by invocation features.
        #[serde(default)]
        features: Vec<String>,
    },
    /// A loose native executable; never invokes Cargo or rustc.
    Prebuilt {
        /// Confined root-relative path with optional {target} and {exe} placeholders.
        path: PathBuf,
        /// Explicit semantic version.
        version: String,
        /// Exact target, or supplied by the invocation.
        #[serde(default)]
        target: Option<String>,
    },
}

fn default_manifest() -> PathBuf {
    PathBuf::from("Cargo.toml")
}

/// Where an authoritative configuration originated.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum SourceOrigin {
    /// Selected standalone file (not reread by the worker).
    File {
        /// Canonical selected path.
        path: PathBuf,
    },
    /// An embedding caller supplied the source and explicit lock path.
    Programmatic,
}

/// Owned source envelope; the complete existing configuration has one owner.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSource {
    /// Canonical input root. All payload inputs are confined here.
    pub root: PathBuf,
    /// Explicit lock origin, including for programmatic sources.
    pub lock_path: PathBuf,
    /// File provenance, separate from payload path resolution.
    pub origin: SourceOrigin,
    /// Main executable input.
    pub program: Program,
    /// Existing distribution model, without a second configuration type.
    pub config: Config,
}

/// Execution overrides for a standalone source, serialized into the owned worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBundleRequest {
    /// Selected source snapshot; the worker never reloads Bundle.toml.
    pub source: BundleSource,
    /// Native package platform.
    pub platform: Platform,
    /// Exact native target override.
    pub target: Option<String>,
    /// Explicit format selection; empty retains configuration defaults.
    pub formats: Vec<Format>,
    /// Absolute output; defaults to the input root's dist directory.
    pub output: PathBuf,
    /// Cargo release profile; prebuilt requires true.
    pub release: bool,
    /// Additional Cargo features; forbidden for prebuilt inputs.
    pub features: Vec<String>,
    /// Use verified component cache only.
    pub offline_components: bool,
    /// Require the declared lock file to match component inputs.
    pub locked_components: bool,
    /// Public native signing references.
    pub trust: rake_dist_core::TrustOptions,
    /// Explicit macOS entitlements override.
    pub entitlements: Option<PathBuf>,
}

impl SourceBundleRequest {
    /// Creates a release request with root-relative default output and no overrides.
    pub fn new(source: BundleSource, platform: Platform) -> Self {
        Self {
            output: source.root.join("dist"),
            source,
            platform,
            target: None,
            formats: Vec::new(),
            release: true,
            features: Vec::new(),
            offline_components: false,
            locked_components: false,
            trust: Default::default(),
            entitlements: None,
        }
    }
}

/// Resolved main executable and Cargo selection, without performing a build.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ResolvedProgram {
    /// Locked/offline Cargo selection.
    Cargo {
        /// Selected package manifest.
        manifest: PathBuf,
        /// Selected workspace package.
        package: String,
        /// Selected main binary.
        bin: String,
        /// Cargo semantic version.
        version: String,
        /// Selected Cargo profile (release or debug).
        profile: String,
        /// Combined declared and invocation features.
        features: Vec<String>,
    },
    /// Inspected existing executable.
    Prebuilt {
        /// Canonical executable path.
        path: PathBuf,
        /// Explicit semantic version.
        version: String,
    },
}

/// Complete read-only source plan with path and selection provenance.
#[derive(Clone, Debug, Serialize)]
pub struct SourcePlan {
    /// Authoritative normalized source snapshot.
    pub source: BundleSource,
    /// Resolved executable selection.
    pub program: ResolvedProgram,
    /// Exact target used by configuration overrides and binary inspection.
    pub target: String,
    /// Resolved output directory (no directory is created by planning).
    pub output: PathBuf,
    /// Cargo metadata sources shadowed by the selected file.
    pub shadowed_metadata: Vec<PathBuf>,
    /// Effective portable distribution plan.
    pub plan: Plan,
    /// Native tools/signatures are deliberately not checked during planning.
    pub native_validation: String,
}

/// Returns the standalone document schema; [`crate::configuration_schema`] stays legacy.
pub fn bundle_file_schema() -> serde_json::Value {
    let mut schema = crate::configuration_schema();
    let program = schemars::schema_for!(Program).to_value();
    if let Some(definitions) = program.get("$defs").and_then(|v| v.as_object()) {
        schema["$defs"]
            .as_object_mut()
            .expect("Config schema has definitions")
            .extend(definitions.clone());
    }
    schema["title"] = "Bundle.toml".into();
    schema["properties"]["program"] = program;
    schema["properties"]["root"] = serde_json::json!({"type":"string", "description":"Input root relative to the config directory; must contain the config"});
    schema["properties"]["schema_version"] = serde_json::json!({"const":1});
    schema["properties"]["name"] = serde_json::json!({"type":"string", "minLength":1});
    schema["properties"]["identifier"] = serde_json::json!({"type":"string", "minLength":1});
    schema["required"] = serde_json::json!(["schema_version", "name", "identifier", "program"]);
    schema
}

/// Selects an explicit file, then exactly invocation-directory Bundle.toml.
///
/// An explicit legacy manifest suppresses discovery. A selected file conflicts
/// with package/bin selectors; metadata is never merged.
/// # Errors
/// Returns selector conflicts, malformed files and unsafe root/path errors.
pub fn select_bundle_file(
    directory: &Path,
    config: Option<&Path>,
    manifest: Option<&Path>,
    package: Option<&str>,
    bin: Option<&str>,
) -> Result<Option<BundleSource>, Error> {
    if config.is_some() && manifest.is_some() {
        return Err(source_error("--config conflicts with --manifest-path"));
    }
    let selected = if let Some(path) = config {
        Some(directory.join(path))
    } else if manifest.is_none() {
        let path = directory.join("Bundle.toml");
        match fs::symlink_metadata(&path) {
            Ok(_) => Some(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        }
    } else {
        None
    };
    let Some(path) = selected else {
        return Ok(None);
    };
    if package.is_some() || bin.is_some() {
        return Err(source_error(
            "a selected Bundle.toml conflicts with --package/--bin; put selection in [program]",
        ));
    }
    load_bundle_file(&path).map(Some)
}

/// Loads the standalone envelope and deserializes all distribution fields as [`Config`].
/// # Errors
/// Returns file-context diagnostics for unknown keys, invalid program forms,
/// missing identity/schema, or paths escaping the explicit input root.
pub fn load_bundle_file(path: &Path) -> Result<BundleSource, Error> {
    let result = || -> Result<BundleSource, Error> {
        let parent = dunce::canonicalize(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let path = dunce::canonicalize(path)?;
        if !path.starts_with(&parent) {
            return Err(source_error("config symlink escapes its directory"));
        }
        let mut value: serde_json::Value = serde_json::to_value(
            toml::from_str::<toml::Value>(&fs::read_to_string(&path)?)
                .map_err(|e| source_error(e.to_string()))?,
        )?;
        let schema = bundle_file_schema();
        check_keys(&value, &schema, &schema, "")?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| source_error("expected a TOML table"))?;
        let root = object
            .remove("root")
            .map(serde_json::from_value::<PathBuf>)
            .transpose()
            .map_err(|e| source_error(format!("root: {e}")))?;
        let root = dunce::canonicalize(parent.join(root.unwrap_or_else(|| PathBuf::from("."))))?;
        if !root.is_dir() || !path.starts_with(&root) {
            return Err(source_error("root must contain the selected config"));
        }
        let program = serde_json::from_value(
            object
                .remove("program")
                .ok_or_else(|| source_error("missing [program]"))?,
        )
        .map_err(|e| source_error(format!("program: {e}")))?;
        let config: Config =
            serde_json::from_value(value).map_err(|e| source_error(e.to_string()))?;
        validate_identity(&config)?;
        Ok(BundleSource {
            root,
            lock_path: path.with_extension("lock"),
            origin: SourceOrigin::File { path },
            program,
            config,
        })
    };
    result().map_err(|error| source_error(format!("{}: {error}", path.display())))
}

fn validate_identity(config: &Config) -> Result<(), Error> {
    if config.schema_version != Some(1) {
        return Err(source_error("standalone schema_version must be 1"));
    }
    for (key, value) in [("name", &config.name), ("identifier", &config.identifier)] {
        if value.as_ref().is_none_or(|v| v.trim().is_empty()) {
            return Err(source_error(format!("standalone {key} is required")));
        }
    }
    Ok(())
}

pub(crate) fn source_error(message: impl Into<String>) -> Error {
    Error::Source(message.into())
}

// Schema properties also drive diagnostics, so newly added Config sections cannot
// accidentally become an ignored-key surface in the standalone adapter.
fn check_keys(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    document: &serde_json::Value,
    path: &str,
) -> Result<(), Error> {
    if let Some(reference) = schema
        .get("$ref")
        .and_then(|v| v.as_str())
        .and_then(|r| r.strip_prefix('#'))
        && let Some(resolved) = document.pointer(reference)
    {
        return check_keys(value, resolved, document, path);
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(variants) = schema.get(keyword).and_then(|v| v.as_array()) {
            let variant = variants
                .iter()
                .find(|s| {
                    s.get("properties")
                        .and_then(|p| p.as_object())
                        .is_some_and(|properties| {
                            properties.iter().any(|(key, property)| {
                                property
                                    .get("const")
                                    .is_some_and(|tag| Some(tag) == value.get(key))
                            })
                        })
                })
                .or_else(|| {
                    variants
                        .iter()
                        .find(|s| s.get("type").is_none_or(|t| t != "null"))
                });
            if let Some(variant) = variant {
                return check_keys(value, variant, document, path);
            }
        }
    }
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let field = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            if let Some(child) = schema.get("properties").and_then(|v| v.get(key)) {
                check_keys(value, child, document, &field)?;
            } else if let Some(additional) =
                schema.get("additionalProperties").filter(|v| v.is_object())
            {
                check_keys(value, additional, document, &field)?;
            } else if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
                let closest = schema
                    .get("properties")
                    .and_then(|v| v.as_object())
                    .and_then(|p| {
                        p.keys()
                            .min_by_key(|candidate| edit_distance(key, candidate))
                    });
                let hint = closest
                    .map(|key| format!("; did you mean `{key}`?"))
                    .unwrap_or_default();
                return Err(source_error(format!("unknown key `{field}`{hint}")));
            }
        }
    } else if let Some(array) = value.as_array()
        && let Some(items) = schema.get("items")
    {
        for (index, value) in array.iter().enumerate() {
            check_keys(value, items, document, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut row: Vec<usize> = (0..=right.chars().count()).collect();
    for (i, a) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in right.chars().enumerate() {
            let previous = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(previous + 1)
                .min(diagonal + usize::from(a != b));
            diagonal = previous;
        }
    }
    *row.last().expect("distance row contains its origin")
}

pub(crate) fn cargo_metadata(manifest: &Path) -> Result<Metadata, Error> {
    let output = Command::new("cargo")
        .current_dir(
            manifest
                .parent()
                .ok_or_else(|| source_error("manifest has no parent"))?,
        )
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(Error::Cargo(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub(crate) fn select_package<'a>(
    metadata: &'a Metadata,
    package: Option<&str>,
) -> Result<&'a Package, Error> {
    let candidates: Vec<_> = metadata
        .packages
        .iter()
        .filter(|p| metadata.workspace_members.contains(&p.id))
        .collect();
    let selected: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|p| package.is_none_or(|name| name == p.name))
        .collect();
    if selected.len() != 1 {
        return Err(source_error(format!(
            "select program.package from: {}",
            candidates
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(selected[0])
}

/// Resolves a source without builds, downloads, signing, credentials or publication.
///
/// Cargo inspection is locked/offline. Prebuilt inspection reads the binary
/// directly and never invokes Cargo/rustc. Native trust checks remain pending.
/// # Errors
/// Returns incompatible selections, unsafe paths, malformed/wrong-architecture
/// executables, Cargo ambiguity, or existing effective Config validation errors.
pub fn plan_source(request: &SourceBundleRequest) -> Result<SourcePlan, Error> {
    let source = &request.source;
    validate_identity(&source.config)?;
    if !source.root.is_absolute()
        || dunce::canonicalize(&source.root)? != source.root
        || !source.lock_path.is_absolute()
    {
        return Err(source_error(
            "source root must be canonical and lock_path must be absolute",
        ));
    }
    if let SourceOrigin::File { path } = &source.origin
        && (!path.starts_with(&source.root)
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            || path
                .parent()
                .map(dunce::canonicalize)
                .transpose()?
                .as_deref()
                != path.parent()
            || source.lock_path != path.with_extension("lock"))
    {
        return Err(source_error(
            "file source/lock provenance does not match root",
        ));
    }
    let (program, package, version, target, shadowed_metadata) = match &source.program {
        Program::Cargo {
            manifest,
            package,
            bin,
            features,
        } => {
            let manifest = files::confined(&source.root, manifest)?;
            let metadata = cargo_metadata(&manifest)?;
            let selected = select_package(&metadata, package.as_deref())?;
            // A workspace manifest cannot select an application outside the declared root.
            if !dunce::canonicalize(&selected.manifest_path)?.starts_with(&source.root) {
                return Err(source_error("selected Cargo package escapes input root"));
            }
            let binaries: Vec<_> = selected
                .targets
                .iter()
                .filter(|t| t.kind.iter().any(|k| k == "bin"))
                .map(|t| t.name.as_str())
                .collect();
            let selected_bins: Vec<_> = binaries
                .iter()
                .copied()
                .filter(|name| bin.as_deref().is_none_or(|b| b == *name))
                .collect();
            if selected_bins.len() != 1 {
                return Err(source_error(format!(
                    "select program.bin from: {}",
                    binaries.join(", ")
                )));
            }
            let mut features = features.clone();
            for feature in &request.features {
                if !features.contains(feature) {
                    features.push(feature.clone());
                }
            }
            let target = request.target.clone().map_or_else(crate::host_target, Ok)?;
            let shadowed = selected
                .metadata
                .pointer("/rake/bundle")
                .map(|_| vec![selected.manifest_path.clone()])
                .unwrap_or_default();
            (
                ResolvedProgram::Cargo {
                    manifest: selected.manifest_path.clone(),
                    package: selected.name.clone(),
                    bin: selected_bins[0].into(),
                    version: selected.version.clone(),
                    profile: if request.release { "release" } else { "debug" }.into(),
                    features,
                },
                selected.name.clone(),
                selected.version.clone(),
                target,
                shadowed,
            )
        }
        Program::Prebuilt {
            path,
            version,
            target,
        } => {
            if !request.release || !request.features.is_empty() {
                return Err(source_error(
                    "prebuilt programs do not support Cargo-only --debug/--features",
                ));
            }
            if source.config.macos.universal {
                return Err(source_error(
                    "prebuilt programs do not support macos.universal",
                ));
            }
            if target.is_some() && request.target.is_some() && target != &request.target {
                return Err(source_error(
                    "program.target conflicts with --target-triple",
                ));
            }
            let target = target
                .as_ref()
                .or(request.target.as_ref())
                .ok_or_else(|| source_error("prebuilt requires program.target or --target-triple"))?
                .clone();
            Platform::from_target(&target)?;
            semver::Version::parse(version)
                .map_err(|e| source_error(format!("program.version: {e}")))?;
            let expanded = path.to_string_lossy().replace("{target}", &target).replace(
                "{exe}",
                if request.platform == Platform::Windows {
                    ".exe"
                } else {
                    ""
                },
            );
            let path = files::confined(&source.root, Path::new(&expanded))?;
            if !path.is_file() {
                return Err(source_error("program.path must be a loose executable file"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if request.platform != Platform::Windows
                    && fs::metadata(&path)?.permissions().mode() & 0o111 == 0
                {
                    return Err(source_error("program.path is not executable"));
                }
            }
            crate::components::architecture(&path, &target)?;
            validate_executable_image(&path)?;
            let package = executable_name(&path, request.platform)?;
            let manifest = source.root.join("Cargo.toml");
            let shadowed = if manifest.is_file() {
                let manifest = files::confined(&source.root, Path::new("Cargo.toml"))?;
                let value: toml::Value = toml::from_str(&fs::read_to_string(&manifest)?)
                    .map_err(|e| source_error(e.to_string()))?;
                value
                    .get("package")
                    .and_then(|v| v.get("metadata"))
                    .and_then(|v| v.get("rake"))
                    .and_then(|v| v.get("bundle"))
                    .map(|_| vec![manifest])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            (
                ResolvedProgram::Prebuilt {
                    path,
                    version: version.clone(),
                },
                package,
                version.clone(),
                target,
                shadowed,
            )
        }
    };
    if request.entitlements.is_some() && request.platform != Platform::Macos {
        return Err(source_error("--entitlements requires macOS"));
    }
    let target = if source.config.macos.universal && request.platform == Platform::Macos {
        "universal-apple-darwin".into()
    } else {
        target
    };
    let plan = source.config.plan_for(
        &package,
        &version,
        request.platform,
        Some(&target),
        (!request.formats.is_empty()).then_some(&request.formats),
    )?;
    for package in plan.packages() {
        let config = package.config();
        if matches!(source.program, Program::Prebuilt { .. })
            && (config.web.base_path.is_some()
                || config.web.routing.is_some()
                || config.web.offline)
        {
            return Err(source_error(
                "prebuilt programs do not support Web configuration",
            ));
        }
        if config.updates.enabled || package.format() == Format::Velopack {
            return Err(source_error(
                "standalone updates/Velopack require the build-context integration (RFC step 2), which is not supported yet",
            ));
        }
        if package.format() == Format::Flatpak {
            return Err(source_error(
                "standalone Flatpak requires source/build-context integration, which is not supported yet",
            ));
        }
    }
    let output = absolute_output(&request.output)?;
    let mut protected = vec![source.lock_path.as_path()];
    if let SourceOrigin::File { path } = &source.origin {
        protected.push(path);
    }
    if let ResolvedProgram::Cargo { manifest, .. } = &program {
        protected.push(manifest);
    }
    if protected.iter().any(|path| path.starts_with(&output)) {
        return Err(source_error(
            "output overlaps configuration, manifest or lock input",
        ));
    }
    if source.root.starts_with(&output) {
        return Err(source_error("output overlaps application source"));
    }
    if let ResolvedProgram::Prebuilt { path, .. } = &program {
        let cache = source.root.join(PREBUILT_COMPONENT_CACHE);
        if fs::symlink_metadata(&cache).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(source_error(
                "prebuilt component cache must not be a symlink",
            ));
        }
        if path.starts_with(&cache) || output.starts_with(&cache) || cache.starts_with(&output) {
            return Err(source_error(
                "prebuilt component cache cannot be used as program input or overlap output",
            ));
        }
        crate::execution::reject_input_overlap(
            &plan,
            &source.root,
            &cache,
            &target,
            "prebuilt component cache",
        )?;
    }
    crate::execution::reject_input_overlap(&plan, &source.root, &output, &target, "output")?;
    if let ResolvedProgram::Prebuilt { path, .. } = &program
        && (path.starts_with(&output) || output.starts_with(path))
    {
        return Err(source_error("output overlaps prebuilt executable"));
    }
    validate_lock(&source.lock_path)?;
    Ok(SourcePlan {
        source: source.clone(),
        program,
        target,
        output,
        shadowed_metadata,
        plan,
        native_validation: "pending".into(),
    })
}

fn absolute_output(path: &Path) -> Result<PathBuf, Error> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(source_error("output must not be a symlink"));
    }
    let absolute = std::path::absolute(path)?;
    // Resolve existing ancestors without creating output, including symlinks.
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let component = ancestor
            .components()
            .next_back()
            .ok_or_else(|| source_error("invalid output path"))?;
        missing.push(component.as_os_str().to_owned());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| source_error("invalid output parent"))?;
    }
    let mut result = dunce::canonicalize(ancestor)?;
    for name in missing.into_iter().rev() {
        if name == ".." {
            result.pop();
        } else if name != "." {
            result.push(name);
        }
    }
    Ok(result)
}

fn validate_lock(path: &Path) -> Result<(), Error> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(source_error("component lock must not be a symlink"));
    }
    Ok(())
}

pub(crate) fn executable_name(path: &Path, platform: Platform) -> Result<String, Error> {
    let name = path
        .file_name()
        .and_then(|p| p.to_str())
        .ok_or_else(|| source_error("program.path needs a UTF-8 executable name"))?;
    let name = if platform == Platform::Windows && name.to_ascii_lowercase().ends_with(".exe") {
        &name[..name.len() - 4]
    } else {
        name
    };
    if name.is_empty() {
        return Err(source_error("program.path needs an executable name"));
    }
    Ok(name.into())
}

fn validate_executable_image(path: &Path) -> Result<(), Error> {
    use object::{Object, read::macho::FatArch};
    fn executable(bytes: &[u8]) -> bool {
        object::File::parse(bytes).is_ok_and(|file| {
            file.kind() == object::ObjectKind::Executable
                || (file.format() == object::BinaryFormat::Elf
                    && file.kind() == object::ObjectKind::Dynamic
                    && file.entry() != 0)
        })
    }
    let bytes = fs::read(path)?;
    let valid = match object::FileKind::parse(bytes.as_slice()) {
        Ok(object::FileKind::MachOFat32) => {
            object::read::macho::MachOFatFile32::parse(bytes.as_slice()).is_ok_and(|fat| {
                fat.arches()
                    .iter()
                    .all(|arch| arch.data(bytes.as_slice()).is_ok_and(executable))
            })
        }
        Ok(object::FileKind::MachOFat64) => {
            object::read::macho::MachOFatFile64::parse(bytes.as_slice()).is_ok_and(|fat| {
                fat.arches()
                    .iter()
                    .all(|arch| arch.data(bytes.as_slice()).is_ok_and(executable))
            })
        }
        _ => executable(&bytes),
    };
    if !valid {
        return Err(source_error(
            "program.path must be a native executable image, not an object or shared library",
        ));
    }
    Ok(())
}
