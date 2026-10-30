//! Staged native jobs. The CLI runs this coordinator inside an owned worker.
use crate::{
    Error, Format, Metadata, Platform, Worker, components, engine, files, linux,
    plan_metadata_with, recipes,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// Serializable request passed to an isolated packaging worker; contains no secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleRequest {
    /// Cargo manifest path.
    pub manifest: PathBuf,
    /// Explicit workspace package name.
    pub package: Option<String>,
    /// Native packaging platform.
    pub platform: Platform,
    /// Rust target; absent means the compiler host.
    pub target: Option<String>,
    /// Explicit output selection; empty uses configuration.
    pub formats: Vec<Format>,
    /// Output directory owned by this application.
    pub output: PathBuf,
    /// Cargo binary target, required if the package has several binaries.
    pub binary: Option<String>,
    /// Build the release Cargo profile.
    pub release: bool,
    /// Additional Cargo feature selections, merged with required native provider features.
    #[serde(default)]
    pub features: Vec<String>,
    /// Resolve remote components from verified cache only.
    #[serde(default)]
    pub offline_components: bool,
    /// Require all resolved inputs to match a checked-in rake-dist.lock.
    #[serde(default)]
    pub locked_components: bool,
    /// Preserve the legacy bundle.json and unpacked native application layout.
    #[serde(default)]
    pub compatibility: bool,
    /// Explicit macOS entitlements override; validated before final signing.
    #[serde(default)]
    pub entitlements: Option<PathBuf>,
    /// Public local signing references and release enforcement.
    #[serde(default)]
    pub trust: rake_dist_core::TrustOptions,
}

/// Final artifact with deterministic integrity information (not an update signature).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    /// Path relative to the distribution directory.
    pub path: PathBuf,
    /// Package format.
    pub format: Format,
    /// SHA-256 of the file or deterministic directory inventory.
    pub sha256: String,
    /// Native package version.
    pub package_version: String,
    /// Exact native component inputs and prepared tree hashes.
    #[serde(default)]
    pub components: Vec<components::ComponentLock>,
    /// Actual native verifier results for this artifact.
    #[serde(default)]
    pub trust: rake_dist_core::TrustRecord,
    /// Declared baseline and observed native ELF requirements.
    #[serde(default)]
    pub linux: Option<LinuxEvidence>,
    /// Native build counter (Sparkle CFBundleVersion), when distinct from Cargo version.
    #[serde(default)]
    pub build_number: Option<String>,
    /// Public managed package identity needed by native publication metadata.
    #[serde(default)]
    pub native_identity: std::collections::BTreeMap<String, String>,
    /// Public update policy used by this format's build and signed installation.
    #[serde(default)]
    pub update_policy: Option<rake_dist_core::UpdateBinding>,
}

/// Compatibility evidence from the actual Linux staged executable and libraries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinuxEvidence {
    /// Declared distribution baseline; absence makes no baseline promise.
    pub baseline: Option<rake_dist_core::LinuxBaseline>,
    /// Measured ELF requirements, absent for an SDK-owned Flatpak build.
    pub elf: Option<rake_dist_linux::ElfReport>,
}

/// Versioned result for every requested native artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Distribution {
    /// Distribution manifest revision.
    pub schema_version: u32,
    /// Application identity.
    pub identifier: String,
    /// Cargo version.
    pub version: String,
    /// Rust target triple.
    pub target: String,
    /// Engine name and pinned version.
    pub engine: String,
    /// Final artifact records.
    pub artifacts: Vec<Artifact>,
    /// Explicit custom backend outputs, with independent verification evidence.
    #[serde(default)]
    pub custom_artifacts: Vec<crate::CustomArtifact>,
}

/// Reads the compiler's native target triple.
/// # Errors
/// Returns process, compiler output or missing host errors.
pub fn host_target() -> Result<String, Error> {
    let output = Command::new("rustc").arg("-vV").output()?;
    if !output.status.success() {
        return Err(Error::Execution("rustc -vV failed".into()));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| Error::Execution("rustc host missing".into()))
}

/// Runs one bounded worker and reads its committed result.
/// # Errors
/// Returns worker failure, cancellation, timeout or invalid output errors.
pub fn bundle(
    request: &BundleRequest,
    executable: &Path,
    cancelled: &AtomicBool,
) -> Result<Distribution, Error> {
    let output = std::path::absolute(&request.output)?;
    let parent = output
        .parent()
        .ok_or_else(|| Error::Execution("output requires parent".into()))?;
    fs::create_dir_all(parent)?;
    // Parent owns staging so a forcibly terminated worker cannot leak its job.
    let workspace = tempfile::Builder::new()
        .prefix(".rake-worker-")
        .tempdir_in(parent)?;
    let request_file = workspace.path().join("request.json");
    let mut worker_request = request.clone();
    worker_request.manifest = std::path::absolute(&request.manifest)?;
    worker_request.output = output;
    files::write_json(&request_file, &worker_request)?;
    let mut command = Command::new(executable);
    command
        .arg("__distribution-worker")
        .arg(&request_file)
        .arg(workspace.path())
        .current_dir(workspace.path());
    let mut worker = Worker::spawn(&mut command)?;
    let status = worker.wait(cancelled, Duration::from_secs(1800))?;
    if !status.success() {
        return Err(Error::Execution(format!("worker exited {status}")));
    }
    Ok(serde_json::from_slice(&fs::read(
        request.output.join("distribution.json"),
    )?)?)
}

/// Executes a request in the current process. Call only in a dedicated worker.
/// # Errors
/// Returns build, confinement, engine and transactional-output errors.
pub fn execute_worker(request: &BundleRequest, job_parent: &Path) -> Result<Distribution, Error> {
    if request.platform != native_platform()? {
        return Err(Error::Execution(format!(
            "{:?} packaging requires a native runner",
            request.platform
        )));
    }
    let mut target = request.target.clone().map_or_else(host_target, Ok)?;
    let metadata_output = Command::new("cargo")
        .current_dir(
            request
                .manifest
                .parent()
                .ok_or_else(|| Error::Execution("manifest parent missing".into()))?,
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
        .arg(&request.manifest)
        .output()?;
    if !metadata_output.status.success() {
        return Err(Error::Cargo(
            String::from_utf8_lossy(&metadata_output.stderr).into(),
        ));
    }
    let mut plan = plan_metadata_with(
        &metadata_output.stdout,
        request.package.as_deref(),
        request.platform,
        Some(&target),
        (!request.formats.is_empty()).then_some(&request.formats),
    )?;
    if plan.platform() == Platform::Macos && plan.config().macos.universal {
        target = "universal-apple-darwin".into();
        plan = plan_metadata_with(
            &metadata_output.stdout,
            request.package.as_deref(),
            request.platform,
            Some(&target),
            (!request.formats.is_empty()).then_some(&request.formats),
        )?;
    }
    if request.compatibility
        && (plan.packages().len() != 1
            || !matches!(
                (plan.platform(), plan.packages()[0].format()),
                (Platform::Macos, Format::Zip) | (Platform::Linux, Format::Tar)
            ))
    {
        return Err(Error::Execution(
            "legacy compatibility requires one macOS ZIP or Linux tar format".into(),
        ));
    }
    if request.entitlements.is_some() && plan.platform() != Platform::Macos {
        return Err(Error::Execution("--entitlements requires macOS".into()));
    }
    let metadata: Metadata = serde_json::from_slice(&metadata_output.stdout)?;
    for item in plan.packages() {
        if item.format() == Format::Velopack {
            rake_dist_windows::verify_velopack_tool()
                .map_err(|error| Error::Execution(error.to_string()))?;
        }
        let trust = effective_trust(item.config(), &request.trust);
        match plan.platform() {
            Platform::Macos => {
                rake_dist_macos::preflight(&trust).map_err(|e| Error::Execution(e.to_string()))?
            }
            Platform::Windows => {
                rake_dist_windows::preflight(&trust, item.config().windows.timestamp_url.as_deref())
                    .map_err(|e| Error::Execution(e.to_string()))?
            }
            _ => {}
        }
    }
    let selected = metadata
        .packages
        .iter()
        .find(|p| {
            metadata.workspace_members.contains(&p.id)
                && request.package.as_deref().is_none_or(|name| name == p.name)
        })
        .ok_or(Error::Package)?;
    let canonical_root =
        dunce::canonicalize(selected.manifest_path.parent().ok_or(Error::Package)?)?;
    let root = canonical_root.as_path();
    if (request.platform == Platform::Windows
        || plan
            .packages()
            .iter()
            .any(|item| item.config().updates.enabled))
        && !selected.dependencies.iter().any(|dependency| {
            dependency.name == "rake-build" && dependency.kind.as_deref() == Some("build")
        })
    {
        return Err(Error::Execution("native resources/updates require [build-dependencies] rake-build and build.rs calling rake_build::build()?".into()));
    }
    let binaries: Vec<_> = selected
        .targets
        .iter()
        .filter(|t| {
            t.kind.iter().any(|k| k == "bin")
                && request.binary.as_deref().is_none_or(|name| name == t.name)
        })
        .collect();
    if binaries.len() != 1 {
        return Err(Error::Execution("select one executable with --bin".into()));
    }
    let binary = &binaries[0].name;
    let output = std::path::absolute(&request.output)?;
    let output_parent = output
        .parent()
        .ok_or_else(|| Error::Execution("output requires parent".into()))?;
    fs::create_dir_all(output_parent)?;
    let output = dunce::canonicalize(output_parent)?.join(
        output
            .file_name()
            .ok_or_else(|| Error::Execution("output name".into()))?,
    );
    if root.starts_with(&output) {
        return Err(Error::Execution(
            "output overlaps application source".into(),
        ));
    }
    reject_input_overlap(&plan, root, &output, &target)?;
    if let Some(path) = &request.entitlements {
        let path = dunce::canonicalize(path)?;
        if path.starts_with(&output) || output.starts_with(&path) {
            return Err(Error::Execution(
                "output overlaps entitlements input".into(),
            ));
        }
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::Execution("output requires parent".into()))?;
    fs::create_dir_all(parent)?;
    let lock_path = parent.join(format!(
        ".{}.rake-lock",
        output
            .file_name()
            .ok_or_else(|| Error::Execution("output name".into()))?
            .to_string_lossy()
    ));
    if lock_path.is_symlink() {
        return Err(Error::Execution("output lock is a symlink".into()));
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .map_err(|_| Error::Execution("another distribution job owns this output".into()))?;
    if output.exists()
        && !(request.compatibility && crate::compatibility::owns(&output, &selected.name))
        && (output.is_symlink()
            || fs::read_to_string(output.join(".rake-distribution"))
                .ok()
                .as_deref()
                != Some(plan.identifier()))
    {
        return Err(Error::Execution(
            "output is not owned by this application".into(),
        ));
    }
    let job = tempfile::Builder::new()
        .prefix(".rake-job-")
        .tempdir_in(job_parent)?;
    let result_dir = job.path().join("result");
    fs::create_dir(&result_dir)?;
    let mut artifacts = Vec::new();
    for (index, item) in plan.packages().iter().enumerate() {
        let stage = job.path().join(format!("package-{index}"));
        let bin = stage.join("bin");
        fs::create_dir_all(&bin)?;
        let suffix = if request.platform == Platform::Windows {
            ".exe"
        } else {
            ""
        };
        let config = item.config();
        let mut prepared = components::resolve(
            &config.components,
            &target,
            root,
            &stage.join("components"),
            &metadata.target_directory.join("rake-component-cache"),
            request.offline_components,
        )?;
        let sdk = crate::sparkle::prepare(
            item,
            &target,
            root,
            &stage,
            &metadata.target_directory.join("rake-component-cache"),
            request.offline_components,
            &mut prepared,
        )?;
        let build_context = crate::sparkle::BuildContext {
            distribution_target: &target,
            sparkle_framework: sdk.as_deref(),
            features: &request.features,
        };
        let build_for = |target| {
            build(
                selected,
                binary,
                target,
                request.release,
                item,
                selected
                    .dependencies
                    .iter()
                    .find(|dependency| dependency.name == "rake")
                    .map(|dependency| dependency.rename.as_deref().unwrap_or("rake")),
                &build_context,
            )
        };
        let mut legacy_libraries = Vec::new();
        if item.format() != Format::Flatpak {
            let built = if target == "universal-apple-darwin" {
                let arm = build_for("aarch64-apple-darwin")?;
                let intel = build_for("x86_64-apple-darwin")?;
                let output = stage.join("universal-binary");
                let result = Command::new("lipo")
                    .arg("-create")
                    .arg(&arm)
                    .arg(&intel)
                    .arg("-output")
                    .arg(&output)
                    .output()?;
                if !result.status.success() {
                    return Err(Error::Execution(
                        String::from_utf8_lossy(&result.stderr).into(),
                    ));
                }
                fs::set_permissions(&output, fs::metadata(&arm)?.permissions())?;
                components::architecture(&output, &target)?;
                output
            } else {
                build_for(&target)?
            };
            if request.compatibility && plan.is_legacy() && plan.platform() == Platform::Macos {
                legacy_libraries =
                    crate::compatibility::mac_libraries(&built, &stage.join("legacy-libraries"))?;
            }
            files::copy(&built, &bin.join(format!("{binary}{suffix}")))?;
        }
        let mut resources = HashMap::new();
        let resource_stage = stage.join("resources");
        fs::create_dir_all(&resource_stage)?;
        if plan.platform() == Platform::Macos {
            for (locale, labels) in &config.desktop.localizations {
                let mut text = String::new();
                let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
                if let Some(name) = &labels.name {
                    text.push_str(&format!(
                        "\"CFBundleDisplayName\" = \"{}\";\n",
                        escape(name)
                    ));
                }
                if let Some(description) = &labels.description {
                    text.push_str(&format!(
                        "\"CFBundleGetInfoString\" = \"{}\";\n",
                        escape(description)
                    ));
                }
                let relative = PathBuf::from(format!("{locale}.lproj/InfoPlist.strings"));
                let destination = resource_stage.join(&relative);
                fs::create_dir_all(
                    destination
                        .parent()
                        .ok_or_else(|| Error::Execution("localization parent".into()))?,
                )?;
                let bytes: Vec<u8> = std::iter::once(0xfeff_u16)
                    .chain(text.encode_utf16())
                    .flat_map(u16::to_le_bytes)
                    .collect();
                fs::write(&destination, bytes)?;
                resources.insert(
                    destination.to_string_lossy().into_owned(),
                    relative.to_string_lossy().into_owned(),
                );
            }
        }
        if let Some(assets) = &config.assets {
            let path = resource_stage.join("assets");
            files::copy(&files::confined(root, assets)?, &path)?;
            resources.insert(path.to_string_lossy().into_owned(), "assets/".into());
        }
        for resource in &config.resources {
            let path = resource_stage.join(&resource.to);
            files::copy(&files::confined(root, &resource.from)?, &path)?;
            resources.insert(
                path.to_string_lossy().into_owned(),
                resource.to.to_string_lossy().into_owned(),
            );
        }
        let mut descriptor = rake_dist_core::Installation {
            schema_version: 1,
            identifier: plan.identifier().into(),
            version: plan.version().into(),
            package_version: item.version().version.clone(),
            target: target.clone(),
            platform: plan.platform(),
            format: item.format(),
            update_owner: item.update_owner(),
            updates: config.updates.clone(),
            helpers: Default::default(),
            runtimes: Default::default(),
            frameworks: Default::default(),
            native_groups: Default::default(),
        };
        for helper in &config.sidecars {
            let path = PathBuf::from(helper.path.to_string_lossy().replace("{target}", &target));
            let relative = if plan.platform() == Platform::Windows {
                PathBuf::from("rake/helpers")
                    .join(&helper.name)
                    .join(format!("{}{suffix}", helper.name))
            } else {
                PathBuf::from("rake/helpers").join(format!("{}{suffix}", helper.name))
            };
            let destination = resource_stage.join(&relative);
            let source_root = recipes::root(helper.component.as_deref(), &prepared.roots, root)?;
            files::copy(&files::confined(source_root, &path)?, &destination)?;
            resources.insert(
                destination.to_string_lossy().into_owned(),
                relative.to_string_lossy().into_owned(),
            );
            components::architecture(&destination, &target)?;
            if let Some(component) = &helper.component {
                let member = if plan.platform() == Platform::Macos {
                    PathBuf::from("Resources").join(&relative)
                } else {
                    relative.clone()
                };
                descriptor
                    .native_groups
                    .entry(component.clone())
                    .or_default()
                    .push(member);
            }
            descriptor.helpers.insert(helper.name.clone(), relative);
        }
        let mut frameworks = recipes::runtimes(recipes::Input {
            config,
            package: root,
            roots: &prepared.roots,
            resource_stage: &resource_stage,
            target: &target,
            descriptor: &mut descriptor,
            resources: &mut resources,
        })?;
        frameworks.extend(legacy_libraries);
        if let Some(sdk) = &sdk {
            let framework = stage.join("frameworks/Sparkle.framework");
            files::copy(sdk, &framework)?;
            frameworks.push(framework);
            descriptor
                .frameworks
                .insert("sparkle".into(), PathBuf::from("Sparkle.framework"));
            descriptor.native_groups.insert(
                "rake-sparkle-sdk".into(),
                vec![PathBuf::from("Frameworks/Sparkle.framework")],
            );
            let license = resource_stage.join("rake/licenses/rake-sparkle-sdk/LICENSE");
            files::copy(
                &sdk.parent()
                    .ok_or_else(|| Error::Execution("SDK parent".into()))?
                    .join("LICENSE"),
                &license,
            )?;
            resources.insert(
                license.to_string_lossy().into_owned(),
                "rake/licenses/rake-sparkle-sdk/LICENSE".into(),
            );
        }
        for component in config
            .components
            .iter()
            .filter(|component| component.target == target)
        {
            let source_root = prepared
                .roots
                .get(&component.name)
                .ok_or_else(|| Error::Execution("prepared component missing".into()))?;
            for license in &component.licenses {
                let relative = PathBuf::from("rake/licenses")
                    .join(&component.name)
                    .join(license);
                let destination = resource_stage.join(&relative);
                files::copy(&files::confined(source_root, license)?, &destination)?;
                resources.insert(
                    destination.to_string_lossy().into_owned(),
                    relative.to_string_lossy().into_owned(),
                );
            }
        }
        for (provider, path) in &config.updates.public_keys {
            rake_dist_core::validate_relative_path(Path::new(provider))?;
            if Path::new(provider).components().count() != 1 {
                return Err(Error::Execution("invalid public key provider".into()));
            }
            let relative = PathBuf::from("rake/keys").join(format!("{provider}.pub"));
            let destination = resource_stage.join(&relative);
            files::copy(&files::confined(root, path)?, &destination)?;
            resources.insert(
                destination.to_string_lossy().into_owned(),
                relative.to_string_lossy().into_owned(),
            );
            descriptor
                .updates
                .public_keys
                .insert(provider.clone(), relative);
        }
        if plan.platform() == Platform::Windows {
            let crt = crate::windows_dlls::prepare(
                config,
                &descriptor,
                &resource_stage,
                &bin.join(format!("{binary}{suffix}")),
                &mut resources,
            )?;
            prepared.locks.extend(crt);
        }
        let component_lock = root.join("rake-dist.lock");
        if request.locked_components && !component_lock.is_file() {
            return Err(Error::Execution(
                "--locked-components requires rake-dist.lock".into(),
            ));
        }
        if component_lock.exists() {
            let locked: Vec<components::ComponentLock> =
                serde_json::from_slice(&fs::read(component_lock)?)?;
            for resolved in &prepared.locks {
                if !locked.contains(resolved) {
                    return Err(Error::Execution(format!(
                        "component {} differs from rake-dist.lock",
                        resolved.name
                    )));
                }
            }
        }
        let descriptor_path = resource_stage.join("rake-installation.json");
        if descriptor_path.exists() {
            return Err(Error::Execution(
                "rake-installation.json is reserved".into(),
            ));
        }
        descriptor.normalize_paths()?;
        files::write_json(&descriptor_path, &descriptor)?;
        resources.insert(
            descriptor_path.to_string_lossy().into_owned(),
            "rake-installation.json".into(),
        );
        if request.platform == Platform::Windows {
            let binary_path = bin.join(format!("{binary}{suffix}"));
            if let Some(manifest) = &config.windows.manifest {
                rake_dist_windows::apply_manifest(&binary_path, &files::confined(root, manifest)?)
                    .map_err(|error| Error::Execution(error.to_string()))?;
            } else {
                rake_dist_windows::apply_default_manifest(
                    &binary_path,
                    plan.identifier(),
                    &item.version().version,
                )
                .map_err(|error| Error::Execution(error.to_string()))?;
            }
        }
        let source_icon = config
            .icon
            .as_ref()
            .map(|path| files::confined(root, path))
            .transpose()?;
        let native_icns = plan.platform() == Platform::Macos
            && source_icon.as_ref().is_some_and(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "icns")
            });
        let icon_directory = rake_build::generate_icons(
            if native_icns {
                None
            } else {
                source_icon.as_deref()
            },
            &stage,
        )
        .map_err(|e| Error::Execution(format!("icon generation: {e}")))?;
        let mut icons: Vec<_> = ["icon.png", "icon.ico"]
            .into_iter()
            .map(|name| icon_directory.join(name).to_string_lossy().into_owned())
            .collect();
        if native_icns {
            let destination = icon_directory.join("icon.icns");
            files::copy(
                source_icon
                    .as_deref()
                    .ok_or_else(|| Error::Execution("ICNS input missing".into()))?,
                &destination,
            )?;
            icons.push(destination.to_string_lossy().into_owned());
        }
        let trust = effective_trust(config, &request.trust);
        if plan.platform() == Platform::Windows {
            sign_windows_tree(&bin, &trust, config.windows.timestamp_url.as_deref())?;
            sign_windows_tree(
                &resource_stage,
                &trust,
                config.windows.timestamp_url.as_deref(),
            )?;
        }
        let mut linux_evidence = None;
        let linux_metadata = if plan.platform() == Platform::Linux {
            linux::integration(&plan, index, binary, root, &stage, &icon_directory)?
        } else {
            HashMap::new()
        };
        if plan.platform() == Platform::Linux && item.format() != Format::Flatpak {
            let mut groups = descriptor.native_groups.clone();
            let name = config
                .linux
                .package_name
                .as_deref()
                .ok_or_else(|| Error::Execution("Linux package name missing".into()))?;
            for members in groups.values_mut() {
                for member in members {
                    *member = PathBuf::from("usr/lib").join(name).join(&*member);
                }
            }
            let payload = linux::payload(
                &stage,
                binary,
                &bin,
                &resource_stage,
                name,
                &linux_metadata,
                true,
            )?;
            let report =
                rake_dist_linux::normalize_elf(&payload, &groups, config.linux.baseline.as_ref())
                    .map_err(|error| Error::Execution(error.to_string()))?;
            files::write_json(&stage.join("elf-requirements.json"), &report)?;
            linux_evidence = Some(LinuxEvidence {
                baseline: config.linux.baseline.clone(),
                elf: Some(report),
            });
            // The engine consumes the exact relocated files, not the Cargo output.
            fs::copy(payload.join("usr/bin").join(binary), bin.join(binary))?;
            fs::remove_dir_all(&resource_stage)?;
            files::copy(&payload.join("usr/lib").join(name), &resource_stage)?;
        }
        let paths = if plan.platform() == Platform::Linux
            && matches!(
                item.format(),
                Format::Arch | Format::Flatpak | Format::Tar | Format::Rpm | Format::Deb
            ) {
            let name = config
                .linux
                .package_name
                .as_deref()
                .ok_or_else(|| Error::Execution("Linux package name missing".into()))?;
            let payload = stage.join("linux-payload");
            match item.format() {
                Format::Deb => vec![
                    rake_dist_linux::build_deb(&rake_dist_linux::ArchPackage {
                        name,
                        version: &item.version().version,
                        target: &target,
                        display_name: plan.name(),
                        dependencies: &linux::dependencies(config, item.format()),
                        payload: &payload,
                        output: &stage.join("deb"),
                    })
                    .map_err(|error| Error::Execution(error.to_string()))?,
                ],
                Format::Rpm => vec![
                    rake_dist_linux::build_rpm(&rake_dist_linux::ArchPackage {
                        name,
                        version: &item.version().version,
                        target: &target,
                        display_name: plan.name(),
                        dependencies: &linux::dependencies(config, item.format()),
                        payload: &payload,
                        output: &stage.join("rpm"),
                    })
                    .map_err(|error| Error::Execution(error.to_string()))?,
                ],
                Format::Arch => rake_dist_linux::build_arch(&rake_dist_linux::ArchPackage {
                    name,
                    version: &item.version().version,
                    target: &target,
                    display_name: plan.name(),
                    dependencies: &linux::dependencies(config, item.format()),
                    payload: &payload,
                    output: &stage.join("arch"),
                })
                .map_err(|error| Error::Execution(error.to_string()))?,
                Format::Flatpak => {
                    let payload = linux::payload(
                        &stage,
                        binary,
                        &bin,
                        &resource_stage,
                        name,
                        &linux_metadata,
                        false,
                    )?;
                    linux::flatpak(
                        &plan,
                        index,
                        binary,
                        selected,
                        &metadata,
                        &target,
                        &payload,
                        &stage,
                        request.offline_components,
                        &request.features,
                    )?
                }
                Format::Tar => {
                    let path = stage.join(format!("{name}-{}.tar.gz", item.version().version));
                    let encoder = flate2::write::GzEncoder::new(
                        fs::File::create(&path)?,
                        flate2::Compression::default(),
                    );
                    let mut archive = tar::Builder::new(encoder);
                    archive.follow_symlinks(false);
                    if request.compatibility {
                        let application = crate::compatibility::linux_payload(
                            &payload,
                            &result_dir,
                            plan.identifier(),
                            name,
                        )?;
                        archive.append_dir_all(&selected.name, &application)?;
                    } else {
                        archive.append_dir_all(".", &payload)?;
                    }
                    archive.into_inner()?.finish()?;
                    vec![path]
                }
                _ => unreachable!(),
            }
        } else if plan.platform() == Platform::Windows
            && matches!(item.format(), Format::Msix | Format::Velopack | Format::Zip)
        {
            let payload = stage.join("payload");
            fs::create_dir_all(&payload)?;
            files::copy(
                &bin.join(format!("{binary}.exe")),
                &payload.join(format!("{binary}.exe")),
            )?;
            for (source, relative) in &resources {
                files::copy(Path::new(source), &payload.join(relative))?;
            }
            windows_package(
                &plan,
                index,
                &payload,
                &stage,
                binary,
                &target,
                &icon_directory,
                root,
                &trust,
            )?
        } else {
            engine::package(engine::Input {
                plan: &plan,
                index,
                bin: &bin,
                binary,
                target: &target,
                root,
                resources,
                icons,
                helpers: Vec::new(),
                frameworks,
                linux_files: linux_metadata,
            })?
        };
        for path in paths {
            let (path, trust_record) = if plan.platform() == Platform::Macos {
                let app = path.clone();
                if request.compatibility {
                    crate::compatibility::mac_icon(&app)?;
                }
                let entitlements = config
                    .macos
                    .entitlements
                    .as_ref()
                    .map(|path| files::confined(root, path))
                    .transpose()?;
                let entitlements = request.entitlements.as_deref().or(entitlements.as_deref());
                let packaged = rake_dist_macos::package(
                    &path,
                    item.format(),
                    &stage,
                    config.name.as_deref().unwrap_or(plan.name()),
                    &trust,
                    entitlements,
                )
                .map_err(|error| Error::Execution(error.to_string()))?;
                if request.compatibility {
                    let destination = result_dir.join("application").join(
                        app.file_name()
                            .ok_or_else(|| Error::Execution("app filename".into()))?,
                    );
                    files::copy(&app, &destination)?;
                    rake_dist_macos::verify_artifact(&destination, Format::App, &packaged.1)
                        .map_err(|error| Error::Execution(error.to_string()))?;
                }
                packaged
            } else if plan.platform() == Platform::Windows
                && path.extension().is_some_and(|extension| {
                    matches!(extension.to_str(), Some("exe" | "msi" | "msix"))
                })
            {
                let record =
                    rake_dist_windows::sign(&path, &trust, config.windows.timestamp_url.as_deref())
                        .map_err(|error| Error::Execution(error.to_string()))?;
                (path, record)
            } else {
                (path, rake_dist_core::TrustRecord::default())
            };
            if !dunce::canonicalize(&path)?.starts_with(dunce::canonicalize(&stage)?) {
                return Err(Error::Execution("engine output escaped job".into()));
            }
            let name = path
                .file_name()
                .ok_or_else(|| Error::Execution("artifact filename missing".into()))?;
            let relative = rake_dist_core::portable_relative_path(
                &PathBuf::from(format!("{index}")).join(name),
            )?;
            let destination = result_dir.join(&relative);
            fs::create_dir_all(
                destination
                    .parent()
                    .ok_or_else(|| Error::Execution("artifact parent".into()))?,
            )?;
            files::copy(&path, &destination)?;
            if plan.platform() == Platform::Macos {
                rake_dist_macos::verify_artifact(&destination, item.format(), &trust_record)
                    .map_err(|error| Error::Execution(error.to_string()))?;
            }
            artifacts.push(Artifact {
                path: relative,
                format: item.format(),
                sha256: files::digest(&destination)?,
                package_version: item.version().version.clone(),
                components: prepared.locks.clone(),
                trust: trust_record,
                linux: linux_evidence.clone(),
                build_number: config.build_number.clone(),
                update_policy: Some(rake_dist_core::UpdateBinding {
                    schema_version: 1,
                    identifier: plan.identifier().into(),
                    version: plan.version().into(),
                    package_version: item.version().version.clone(),
                    target: target.clone(),
                    format: item.format(),
                    owner: item.update_owner(),
                    channel: config
                        .updates
                        .channel
                        .clone()
                        .unwrap_or_else(|| "stable".into()),
                    base_url: config.updates.base_url.clone(),
                    public_keys: descriptor
                        .updates
                        .public_keys
                        .iter()
                        .map(|(provider, path)| {
                            Ok((
                                provider.clone(),
                                fs::read(files::confined(&resource_stage, path)?)?,
                            ))
                        })
                        .collect::<Result<_, Error>>()?,
                }),
                native_identity: [
                    ("msix_name", Some(plan.identifier())),
                    ("msix_publisher", config.windows.msix.publisher.as_deref()),
                ]
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key.into(), value.into())))
                .collect(),
            });
        }
    }
    let custom_artifacts = crate::extensions::package_custom(
        &plan,
        &target,
        root,
        &result_dir,
        &artifacts,
        &effective_trust(plan.config(), &request.trust),
    )?;
    let result = Distribution {
        schema_version: 1,
        identifier: plan.identifier().into(),
        version: plan.version().into(),
        target,
        engine: "tauri-bundler 2.10.4".into(),
        custom_artifacts,
        artifacts,
    };
    let mut locks = Vec::new();
    for artifact in &result.artifacts {
        for component in &artifact.components {
            if !locks.contains(component) {
                locks.push(component.clone());
            }
        }
    }
    files::write_json(&result_dir.join("components.lock.json"), &locks)?;
    files::write_json(&result_dir.join("distribution.json"), &result)?;
    if request.compatibility {
        crate::compatibility::write(
            &plan,
            &selected.name,
            binary,
            &result_dir,
            &result.artifacts,
            request.release,
            &result.target,
        )?;
    }
    if request.compatibility {
        files::commit_compatible(
            &result_dir,
            &output,
            plan.identifier(),
            Some(&selected.name),
        )?;
    } else {
        files::commit(&result_dir, &output, plan.identifier())?;
    }
    Ok(result)
}

fn reject_input_overlap(
    plan: &crate::Plan,
    root: &Path,
    output: &Path,
    target: &str,
) -> Result<(), Error> {
    for package in plan.packages() {
        let config = package.config();
        let mut paths = [
            &config.assets,
            &config.icon,
            &config.macos.info_plist,
            &config.macos.entitlements,
            &config.windows.manifest,
            &config.windows.msix.manifest,
            &config.linux.desktop_entry,
            &config.linux.appstream,
            &config.linux.flatpak.manifest,
        ]
        .into_iter()
        .filter_map(Option::as_ref)
        .cloned()
        .collect::<Vec<_>>();
        paths.extend(
            config
                .resources
                .iter()
                .map(|resource| resource.from.clone()),
        );
        paths.extend(config.updates.public_keys.values().cloned());
        paths.extend(
            config
                .runtimes
                .iter()
                .filter(|runtime| !runtime.system && runtime.component.is_none())
                .map(|runtime| runtime.source.clone()),
        );
        paths.extend(
            config
                .sidecars
                .iter()
                .filter(|helper| helper.component.is_none())
                .map(|helper| {
                    PathBuf::from(helper.path.to_string_lossy().replace("{target}", target))
                }),
        );
        paths.extend(
            config
                .components
                .iter()
                .filter(|component| component.target == target)
                .filter_map(|component| match &component.source {
                    rake_dist_core::ComponentSource::Local { path } => Some(path.clone()),
                    _ => None,
                }),
        );
        for path in paths {
            let input = files::confined(root, &path)?;
            if output.starts_with(&input) || input.starts_with(output) {
                return Err(Error::Execution(format!(
                    "output overlaps packaging input {}",
                    input.display()
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn native_platform() -> Result<Platform, Error> {
    match std::env::consts::OS {
        "macos" => Ok(Platform::Macos),
        "windows" => Ok(Platform::Windows),
        "linux" => Ok(Platform::Linux),
        _ => Err(Error::Execution("unsupported native host".into())),
    }
}

fn build(
    selected: &crate::Package,
    binary: &str,
    target: &str,
    release: bool,
    package_plan: &rake_dist_core::PackagePlan,
    facade: Option<&str>,
    context: &crate::sparkle::BuildContext<'_>,
) -> Result<PathBuf, Error> {
    let manifest = &selected.manifest_path;
    let package = selected.name.as_str();
    let format = package_plan.format();
    let mut command = Command::new("cargo");
    command.current_dir(
        manifest
            .parent()
            .ok_or_else(|| Error::Execution("manifest parent missing".into()))?,
    );
    command
        .args([
            "build",
            "--locked",
            "--message-format=json-render-diagnostics",
            "--manifest-path",
        ])
        .arg(manifest)
        .args(["--package", package, "--bin", binary, "--target", target]);
    if !context.features.is_empty() {
        command.arg("--features").arg(context.features.join(","));
    }
    let feature = if format == Format::Velopack {
        Some("updates-velopack")
    } else if package_plan.update_owner() == rake_dist_core::UpdateOwner::Sparkle {
        Some("updates-sparkle")
    } else if package_plan.update_owner() == rake_dist_core::UpdateOwner::Appimage {
        Some("updates-appimage")
    } else if package_plan.config().updates.enabled {
        Some("updates")
    } else {
        None
    };
    if let Some(feature) = feature
        && let Some(facade) = facade
    {
        command.arg("--features").arg(format!("{facade}/{feature}"));
    }
    if format == Format::Velopack && facade.is_none() {
        return Err(Error::Execution(
            "Velopack requires the Rake facade's early startup integration".into(),
        ));
    }
    command.env("RAKE_DISTRIBUTION_TARGET", context.distribution_target);
    if let Some(framework) = context.sparkle_framework {
        command.env("RAKE_SPARKLE_FRAMEWORK", framework);
    }
    if let Some(minimum) = &package_plan.config().minimum_system_version {
        command.env("MACOSX_DEPLOYMENT_TARGET", minimum);
    }
    command.env(
        "RAKE_DISTRIBUTION_FORMAT",
        serde_json::to_value(format)?
            .as_str()
            .ok_or_else(|| Error::Execution("format serialization".into()))?,
    );
    if release {
        command.arg("--release");
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Execution("cargo output unavailable".into()))?;
    let mut executable = None;
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        let count = (&mut reader)
            .take(4 * 1024 * 1024 + 1)
            .read_line(&mut line)?;
        if count == 0 {
            break;
        }
        if count > 4 * 1024 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Execution("Cargo message exceeds bound".into()));
        }
        let message: serde_json::Value = serde_json::from_str(&line)?;
        if message["reason"] == "compiler-message"
            && let Some(rendered) = message["message"]["rendered"].as_str()
        {
            eprint!("{rendered}");
        }
        if message["reason"] == "compiler-artifact"
            && message["target"]["name"] == binary
            && let Some(path) = message["executable"].as_str()
        {
            executable = Some(PathBuf::from(path));
        }
    }
    if !child.wait()?.success() {
        return Err(Error::Execution("Cargo build failed".into()));
    }
    executable.ok_or_else(|| Error::Execution("Cargo did not report an executable".into()))
}

fn effective_trust(
    config: &crate::Config,
    overrides: &rake_dist_core::TrustOptions,
) -> rake_dist_core::TrustOptions {
    rake_dist_core::TrustOptions {
        required: overrides.required,
        macos_identity: overrides
            .macos_identity
            .clone()
            .or_else(|| config.macos.signing_identity.clone()),
        notary_profile: overrides
            .notary_profile
            .clone()
            .or_else(|| config.macos.notary_profile.clone()),
        windows_thumbprint: overrides
            .windows_thumbprint
            .clone()
            .or_else(|| config.windows.certificate_thumbprint.clone()),
    }
}

fn sign_windows_tree(
    root: &Path,
    trust: &rake_dist_core::TrustOptions,
    timestamp: Option<&str>,
) -> Result<(), Error> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sign_windows_tree(&entry.path(), trust, timestamp)?;
        } else if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| matches!(extension.to_str(), Some("exe" | "dll")))
        {
            rake_dist_windows::sign(&entry.path(), trust, timestamp)
                .map_err(|e| Error::Execution(e.to_string()))?;
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "one private platform dispatch boundary"
)]
fn windows_package(
    plan: &crate::Plan,
    index: usize,
    payload: &Path,
    stage: &Path,
    binary: &str,
    target: &str,
    icons: &Path,
    root: &Path,
    trust: &rake_dist_core::TrustOptions,
) -> Result<Vec<PathBuf>, Error> {
    let item = &plan.packages()[index];
    let config = item.config();
    match item.format() {
        Format::Msix => {
            let publisher = config
                .windows
                .msix
                .publisher
                .as_deref()
                .ok_or_else(|| Error::Execution("MSIX publisher missing".into()))?;
            if let Some(thumbprint) = &trust.windows_thumbprint {
                let subject = rake_dist_windows::certificate_subject(thumbprint)
                    .map_err(|e| Error::Execution(e.to_string()))?;
                if subject != publisher {
                    return Err(Error::Execution(
                        "MSIX publisher differs from signing certificate subject".into(),
                    ));
                }
            }
            rake_build::generate_msix_assets(icons, payload)
                .map_err(|e| Error::Execution(e.to_string()))?;
            let custom = config
                .windows
                .msix
                .manifest
                .as_ref()
                .map(|path| files::confined(root, path))
                .transpose()?;
            let settings = rake_dist_windows::Msix {
                identifier: plan.identifier(),
                publisher,
                version: &item.version().version,
                target,
                executable: &format!("{binary}.exe"),
                name: plan.name(),
                manifest: custom.as_deref(),
            };
            Ok(vec![
                rake_dist_windows::package_msix_with_metadata(
                    payload,
                    stage,
                    &settings,
                    &config.desktop,
                )
                .map_err(|e| Error::Execution(e.to_string()))?,
            ])
        }
        Format::Velopack => {
            let output = stage.join("velopack");
            fs::create_dir_all(&output)?;
            rake_dist_windows::package_velopack(
                payload,
                &output,
                plan.identifier(),
                &item.version().version,
                &format!("{binary}.exe"),
                config.updates.channel.as_deref().unwrap_or("stable"),
            )
            .map_err(|e| Error::Execution(e.to_string()))
        }
        Format::Zip => {
            let output = stage.join(format!("{binary}.zip"));
            crate::archives::zip(payload, &output)?;
            Ok(vec![output])
        }
        _ => Err(Error::Execution(
            "unsupported dedicated Windows format".into(),
        )),
    }
}

#[cfg(test)]
mod overlap_tests {
    use super::*;
    #[test]
    fn packaging_output_cannot_contain_or_be_inside_an_input_tree() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = dunce::canonicalize(root.path()).unwrap();
        let root = canonical_root.as_path();
        fs::create_dir(root.join("assets")).unwrap();
        let config = rake_dist_core::Config {
            assets: Some("assets".into()),
            ..Default::default()
        };
        let plan = config.plan("app", "1.0.0", Platform::Macos).unwrap();
        for output in [root.to_owned(), root.join("assets/distribution")] {
            assert!(reject_input_overlap(&plan, root, &output, "aarch64-apple-darwin").is_err());
        }
        reject_input_overlap(&plan, root, &root.join("dist"), "aarch64-apple-darwin").unwrap();
    }
}
