//! Immutable publication and all-target channel promotion.
use crate::{Artifact, Distribution, Error, Format, files};
use fs2::FileExt;
use rake_dist_core::{
    NativeFeedSignature, Publisher, ReleaseConfig, SignedUpdateFeed, UpdateFeed, UpdateRelease,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// Inputs to an explicit release. Directories may include retained older versions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseRequest {
    /// Selected Cargo application identity.
    pub identifier: String,
    /// Version being promoted; every required target must provide this version.
    pub version: String,
    /// Bound channel, usually stable.
    pub channel: String,
    /// Existing bundle output directories containing distribution.json.
    pub distributions: Vec<PathBuf>,
    /// Public publication policy and credential references.
    pub config: ReleaseConfig,
    /// Package root for local signing commands and configured paths.
    pub package_root: PathBuf,
    /// Public verification keys, indexed by provider. Signatures are checked
    /// before publication so selecting the wrong local profile fails safely.
    pub public_keys: BTreeMap<String, Vec<u8>>,
    /// Unix publication time, supplied once by the CLI.
    pub published_at: u64,
    /// Generate configured provider feeds as well as immutable downloads.
    pub updates_enabled: bool,
    /// Independently configured native verifiers accepted for custom formats.
    pub custom_verifiers: BTreeMap<String, Vec<String>>,
}
/// Final channel state. Native feeds remain the provider's native protocol.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedRelease {
    /// State contract revision.
    pub schema_version: u32,
    /// Application identity.
    pub identifier: String,
    /// Selected Cargo version.
    pub version: String,
    /// Release channel.
    pub channel: String,
    /// Hash of the channel state replaced, or None for first publication.
    pub previous: Option<String>,
    /// Every promoted native feed path and immutable object SHA-256.
    pub files: BTreeMap<String, PublishedFile>,
}
/// One immutable, independently downloadable object.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedFile {
    /// Public URL, independent of the local build path.
    pub url: String,
    /// Exact bytes SHA-256.
    pub sha256: String,
    /// Exact byte size.
    pub size: u64,
}
struct Input {
    distribution: Distribution,
    artifact: Artifact,
    source: PathBuf,
}
struct CustomInput {
    artifact: crate::CustomArtifact,
    source: PathBuf,
}
struct Inputs {
    native: Vec<Input>,
    custom: Vec<CustomInput>,
}
struct Prepared {
    directory: tempfile::TempDir,
    state: PublishedRelease,
    objects: BTreeMap<String, PathBuf>,
}
fn fail(message: impl Into<String>) -> Error {
    Error::Execution(message.into())
}
fn segment(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err(fail("invalid release path segment"));
    }
    Ok(())
}
fn cancelled(flag: &AtomicBool) -> Result<(), Error> {
    if flag.load(Ordering::Acquire) {
        Err(fail("release cancelled before promotion"))
    } else {
        Ok(())
    }
}
fn inputs(request: &ReleaseRequest) -> Result<Inputs, Error> {
    request.config.validate()?;
    segment(&request.channel)?;
    semver::Version::parse(&request.version).map_err(|error| fail(error.to_string()))?;
    let mut output = Vec::new();
    let mut custom = Vec::new();
    let mut matrix = BTreeSet::new();
    let mut custom_matrix = BTreeSet::new();
    for directory in &request.distributions {
        let distribution: Distribution =
            serde_json::from_slice(&fs::read(directory.join("distribution.json"))?)?;
        if distribution.schema_version != 1 || distribution.identifier != request.identifier {
            return Err(fail("release distribution identity/schema mismatch"));
        }
        segment(&distribution.target)?;
        semver::Version::parse(&distribution.version).map_err(|error| fail(error.to_string()))?;
        for artifact in &distribution.custom_artifacts {
            let source = files::confined(directory, &artifact.path)?;
            let proof = artifact.verification.as_ref().ok_or_else(|| {
                fail("custom format requires an independent native verifier before release")
            })?;
            if !source.is_file()
                || files::digest(&source)? != artifact.sha256
                || proof.schema_version != 1
                || proof.sha256 != artifact.sha256
                || proof.policy.trim().is_empty()
                || artifact.verifier.as_ref() != request.custom_verifiers.get(&artifact.format)
            {
                return Err(fail(
                    "custom format verification or output identity mismatch",
                ));
            }
            if distribution.version == request.version {
                custom_matrix.insert((distribution.target.clone(), artifact.format.clone()));
            }
            custom.push(CustomInput {
                artifact: artifact.clone(),
                source,
            });
        }
        for artifact in &distribution.artifacts {
            let source = files::confined(directory, &artifact.path)?;
            if files::digest(&source)? != artifact.sha256 {
                return Err(fail("release artifact hash mismatch"));
            }
            if artifact.format == Format::App {
                continue;
            } // directories are published through ZIP/DMG instead.
            if !source.is_file() {
                return Err(fail("release requires regular-file artifacts"));
            }
            if request.updates_enabled {
                let policy = artifact
                    .update_policy
                    .as_ref()
                    .ok_or_else(|| fail("release update policy missing; rebuild the artifact"))?;
                policy.validate()?;
                let root = match request
                    .config
                    .publisher
                    .as_ref()
                    .ok_or_else(|| fail("publisher missing"))?
                {
                    Publisher::Github { repository, .. } => {
                        format!("https://github.com/{repository}/releases/latest/download")
                    }
                    Publisher::Directory { public_url, .. }
                    | Publisher::Https { public_url, .. } => {
                        public_url.trim_end_matches('/').to_owned()
                    }
                };
                if policy.identifier != request.identifier
                    || policy.version != distribution.version
                    || policy.target != distribution.target
                    || policy.format != artifact.format
                    || policy.channel != request.channel
                    || policy
                        .base_url
                        .as_deref()
                        .is_none_or(|value| value.trim_end_matches('/') != root)
                {
                    return Err(fail(
                        "release destination/channel conflicts with compiled artifact update policy",
                    ));
                }
                for (provider, key) in &policy.public_keys {
                    if request.public_keys.get(provider) != Some(key) {
                        return Err(fail(
                            "release signing public key conflicts with compiled artifact policy",
                        ));
                    }
                }
            }
            let platform = if distribution.target == "universal-apple-darwin" {
                crate::Platform::Macos
            } else {
                crate::Platform::from_target(&distribution.target)?
            };
            if !artifact.format.supports(platform) {
                return Err(fail("release artifact format/target mismatch"));
            }
            if platform == crate::Platform::Macos
                && (!artifact.trust.notarized
                    || !artifact.trust.stapled
                    || artifact.trust.signature.as_deref() != Some("developer_id"))
            {
                return Err(fail(
                    "public macOS releases require verified Developer ID, notarization and stapling",
                ));
            }
            if platform == crate::Platform::Windows
                && matches!(
                    source.extension().and_then(|value| value.to_str()),
                    Some("exe" | "msi" | "msix")
                )
                && artifact.trust.signature.as_deref() != Some("authenticode")
            {
                return Err(fail(
                    "public Windows installers require verified Authenticode",
                ));
            }
            if distribution.version == request.version && package_payload(artifact.format, &source)
            {
                matrix.insert((distribution.target.clone(), artifact.format));
            }
            output.push(Input {
                distribution: distribution.clone(),
                artifact: artifact.clone(),
                source,
            });
        }
    }
    for (target, formats) in &request.config.required {
        for format in formats {
            if !matrix.contains(&(target.clone(), *format)) {
                return Err(fail(format!(
                    "release gate missing {} {target} {format:?}",
                    request.version
                )));
            }
        }
    }
    for (target, names) in &request.config.custom_required {
        for name in names {
            if !custom_matrix.contains(&(target.clone(), name.clone())) {
                return Err(fail(format!("release custom gate missing {target} {name}")));
            }
        }
    }
    if matrix.is_empty() && custom_matrix.is_empty() {
        return Err(fail("release has no artifacts for selected version"));
    }
    Ok(Inputs {
        native: output,
        custom,
    })
}
fn signer(
    request: &ReleaseRequest,
    provider: &str,
    message: &[u8],
    flag: &AtomicBool,
) -> Result<Vec<u8>, Error> {
    let argv = request.config.signers.get(provider).ok_or_else(|| {
        fail(format!(
            "release.signers.{provider} requires a local signing-profile command"
        ))
    })?;
    let work = tempfile::tempdir()?;
    let input = work.path().join("message");
    let output = work.path().join("signature");
    fs::write(&input, message)?;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(&request.package_root)
        .stdin(fs::File::open(input)?)
        .stdout(fs::File::create(&output)?)
        .stderr(Stdio::inherit());
    let mut worker = crate::Worker::spawn_with_io(&mut command)?;
    if !worker.wait(flag, Duration::from_secs(120))?.success() {
        return Err(fail("local release signing command failed"));
    }
    if fs::metadata(&output)?.len() != 64 {
        return Err(fail(
            "local signer must return exactly 64 raw Ed25519 signature bytes",
        ));
    }
    let signature = fs::read(output)?;
    let key = request
        .public_keys
        .get(provider)
        .ok_or_else(|| fail(format!("release public key missing: {provider}")))?;
    let key = decode_key(key)?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|e| fail(e.to_string()))?;
    let signature_value =
        ed25519_dalek::Signature::from_slice(&signature).map_err(|e| fail(e.to_string()))?;
    key.verify_strict(message, &signature_value)
        .map_err(|_| fail("local signing profile does not match configured public key"))?;
    Ok(signature)
}
fn decode_key(bytes: &[u8]) -> Result<[u8; 32], Error> {
    use base64::Engine as _;
    if bytes.len() == 32 {
        return bytes.try_into().map_err(|_| fail("invalid public key"));
    }
    let value = std::str::from_utf8(bytes)
        .map_err(|_| fail("invalid public key"))?
        .trim();
    let bytes = if value.len() == 64 {
        hex::decode(value).map_err(|_| fail("invalid public key"))?
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|_| fail("invalid public key"))?
    };
    bytes
        .try_into()
        .map_err(|_| fail("Ed25519 public key must contain 32 bytes"))
}
fn public_root(config: &ReleaseConfig) -> Result<String, Error> {
    match config
        .publisher
        .as_ref()
        .ok_or_else(|| fail("release publisher missing"))?
    {
        Publisher::Directory { public_url, .. } | Publisher::Https { public_url, .. } => {
            Ok(public_url.trim_end_matches('/').into())
        }
        Publisher::Github { repository, tag } => Ok(format!(
            "https://github.com/{repository}/releases/download/{tag}"
        )),
    }
}
fn feed_name(request: &ReleaseRequest, target: &str, file: &str) -> String {
    format!("{}/{target}/{file}", request.channel)
}
fn object_name(hash: &str, file: &str) -> Result<String, Error> {
    let path = Path::new(file);
    rake_dist_core::validate_relative_path(path)?;
    if path.components().count() != 1 || file.chars().any(char::is_control) {
        return Err(fail("invalid immutable object filename"));
    }
    let slug = file
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    Ok(format!("{hash}-{slug}"))
}

fn add(prepared: &mut Prepared, name: String, source: &Path, url: String) -> Result<(), Error> {
    let hash = files::digest(source)?;
    let size = fs::metadata(source)?.len();
    if let Some(previous) = prepared.state.files.get(&name) {
        if previous.sha256 != hash {
            return Err(fail(format!("release file collision: {name}")));
        }
        return Ok(());
    }
    prepared.state.files.insert(
        name.clone(),
        PublishedFile {
            url,
            sha256: hash,
            size,
        },
    );
    prepared.objects.insert(name, source.into());
    Ok(())
}
fn generated(prepared: &mut Prepared, name: String, bytes: &[u8], root: &str) -> Result<(), Error> {
    let path = prepared.directory.path().join("generated").join(&name);
    fs::create_dir_all(path.parent().ok_or_else(|| fail("feed parent"))?)?;
    fs::write(&path, bytes)?;
    let hash = files::digest(&path)?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| fail("feed filename"))?;
    add(
        prepared,
        name,
        &path,
        format!("{root}/objects/{}", object_name(&hash, filename)?),
    )
}
fn prepare(request: &ReleaseRequest, flag: &AtomicBool) -> Result<Prepared, Error> {
    let mut selected = inputs(request)?;
    let root = public_root(&request.config)?;
    let directory = tempfile::tempdir()?;
    for (index, input) in selected.native.iter_mut().enumerate() {
        let snapshot = directory
            .path()
            .join("inputs")
            .join(index.to_string())
            .join(
                input
                    .source
                    .file_name()
                    .ok_or_else(|| fail("artifact filename"))?,
            );
        files::copy(&input.source, &snapshot)?;
        if files::digest(&snapshot)? != input.artifact.sha256 {
            return Err(fail("artifact changed during release preparation"));
        }
        input.source = snapshot;
    }
    for (index, input) in selected.custom.iter_mut().enumerate() {
        let snapshot = directory
            .path()
            .join("custom-inputs")
            .join(index.to_string())
            .join(
                input
                    .source
                    .file_name()
                    .ok_or_else(|| fail("custom filename"))?,
            );
        files::copy(&input.source, &snapshot)?;
        if files::digest(&snapshot)? != input.artifact.sha256 {
            return Err(fail("custom artifact changed during release preparation"));
        }
        input.source = snapshot;
    }
    let inputs = selected.native;

    let mut prepared = Prepared {
        directory,
        state: PublishedRelease {
            schema_version: 1,
            identifier: request.identifier.clone(),
            version: request.version.clone(),
            channel: request.channel.clone(),
            previous: None,
            files: BTreeMap::new(),
        },
        objects: BTreeMap::new(),
    };
    let expiry = request
        .published_at
        .checked_add(request.config.feed_validity_seconds.unwrap_or(7 * 86400))
        .ok_or_else(|| fail("feed expiry overflow"))?;
    let mut images: BTreeMap<String, Vec<UpdateRelease>> = BTreeMap::new();
    let mut sparkle: BTreeMap<String, Vec<&Input>> = BTreeMap::new();
    let mut velopack: BTreeMap<String, Vec<&Input>> = BTreeMap::new();
    for input in &inputs {
        cancelled(flag)?;
        let artifact = &input.artifact;
        let target = &input.distribution.target;
        let filename = input
            .source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| fail("artifact filename"))?;
        let name = object_name(&artifact.sha256, filename)?;
        let url = match &request.config.publisher {
            Some(Publisher::Github { .. }) => format!("{root}/{name}"),
            _ => format!("{root}/objects/{name}"),
        };
        add(
            &mut prepared,
            format!("objects/{name}"),
            &input.source,
            url.clone(),
        )?;
        if !request.updates_enabled {
            continue;
        }
        match artifact.format {
            Format::Appimage => images
                .entry(target.clone())
                .or_default()
                .push(UpdateRelease {
                    version: input.distribution.version.clone(),
                    package_version: artifact.package_version.clone(),
                    url,
                    sha256: artifact.sha256.clone(),
                    size: fs::metadata(&input.source)?.len(),
                }),
            Format::Zip | Format::Dmg if target.contains("apple") => {
                sparkle.entry(target.clone()).or_default().push(input);
            }
            Format::Velopack => {
                velopack.entry(target.clone()).or_default().push(input);
            }
            Format::Msix if input.distribution.version == request.version => {
                let name = feed_name(request, target, "application.appinstaller");
                let feed_url = channel_url(
                    &request.config,
                    &request.channel,
                    target,
                    "application.appinstaller",
                )?;
                let xml = appinstaller(&input.distribution, artifact, &feed_url, &url)?;
                generated(&mut prepared, name, xml.as_bytes(), &root)?;
            }
            _ => {}
        }
    }
    for input in selected.custom {
        let artifact = input.artifact;
        let source = input.source;
        let name = object_name(
            &artifact.sha256,
            source
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| fail("custom filename"))?,
        )?;
        let url = if matches!(request.config.publisher, Some(Publisher::Github { .. })) {
            format!("{root}/{name}")
        } else {
            format!("{root}/objects/{name}")
        };
        add(&mut prepared, format!("objects/{name}"), &source, url)?;
    }
    for (target, releases) in images {
        let feed = UpdateFeed {
            schema_version: 1,
            identifier: request.identifier.clone(),
            target: target.clone(),
            channel: request.channel.clone(),
            published_at: request.published_at,
            expires_at: expiry,
            releases,
        };
        let payload = serde_json::to_string(&feed)?;
        let signature = hex::encode(signer(request, "appimage", payload.as_bytes(), flag)?);
        generated(
            &mut prepared,
            feed_name(request, &target, "appimage.json"),
            &serde_json::to_vec(&SignedUpdateFeed { payload, signature })?,
            &root,
        )?;
    }
    for (target, items) in velopack {
        prepare_velopack(request, &mut prepared, &target, &items, &root, expiry, flag)?;
    }
    for (target, items) in sparkle {
        prepare_sparkle(request, &mut prepared, &target, &items, &root, flag)?;
    }
    for (target, formats) in &request.config.repositories {
        for format in formats {
            cancelled(flag)?;
            let packages = inputs
                .iter()
                .filter(|input| {
                    input.distribution.target == *target
                        && input.artifact.format == *format
                        && package_payload(*format, &input.source)
                })
                .map(|input| input.source.clone())
                .collect::<Vec<_>>();
            let kind = serde_json::to_value(format)?
                .as_str()
                .ok_or_else(|| fail("repository format"))?
                .to_owned();
            let name = format!("rake-{kind}");
            let output = prepared
                .directory
                .path()
                .join(format!("repository-{target}-{kind}"));
            let public_url = format!("{root}/{}/{target}/{kind}", request.channel);
            let paths = rake_dist_linux::build_repository(&rake_dist_linux::Repository {
                format: *format,
                identifier: &request.identifier,
                name: &name,
                packages: &packages,
                output: &output,
                gpg_key: request
                    .config
                    .gpg_key
                    .as_deref()
                    .ok_or_else(|| fail("repository GPG key missing"))?,
                public_url: &public_url,
                branch: "master",
            })
            .map_err(|error| fail(error.to_string()))?;
            for path in paths {
                let relative = path
                    .strip_prefix(&output)
                    .map_err(|_| fail("repository output escapes staging"))?;
                let name = format!(
                    "{}/{target}/{kind}/{}",
                    request.channel,
                    relative.to_string_lossy().replace('\\', "/")
                );
                let hash = files::digest(&path)?;
                let filename = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| fail("repository output filename"))?;
                add(
                    &mut prepared,
                    name,
                    &path,
                    format!("{root}/objects/{}", object_name(&hash, filename)?),
                )?;
            }
        }
    }
    Ok(prepared)
}
fn channel_url(
    config: &ReleaseConfig,
    channel: &str,
    target: &str,
    file: &str,
) -> Result<String, Error> {
    let root = match config
        .publisher
        .as_ref()
        .ok_or_else(|| fail("publisher missing"))?
    {
        Publisher::Github { repository, .. } => {
            format!("https://github.com/{repository}/releases/latest/download")
        }
        _ => public_root(config)?,
    };
    Ok(rake_dist_core::feed_url(&root, channel, target, file)?)
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn appinstaller(
    distribution: &Distribution,
    artifact: &Artifact,
    feed: &str,
    url: &str,
) -> Result<String, Error> {
    let name = artifact
        .native_identity
        .get("msix_name")
        .ok_or_else(|| fail("MSIX release identity missing; rebuild package"))?;
    let publisher = artifact
        .native_identity
        .get("msix_publisher")
        .ok_or_else(|| fail("MSIX publisher missing; rebuild package"))?;
    let arch = match distribution.target.split('-').next() {
        Some("x86_64") => "x64",
        Some("aarch64") => "arm64",
        _ => return Err(fail("MSIX update architecture unsupported")),
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><AppInstaller xmlns=\"http://schemas.microsoft.com/appx/appinstaller/2018\" Version=\"{}\" Uri=\"{}\"><MainPackage Name=\"{}\" Publisher=\"{}\" Version=\"{}\" ProcessorArchitecture=\"{arch}\" Uri=\"{}\"/><UpdateSettings><OnLaunch HoursBetweenUpdateChecks=\"0\" ShowPrompt=\"true\" UpdateBlocksActivation=\"false\"/><AutomaticBackgroundTask/></UpdateSettings></AppInstaller>",
        xml(&artifact.package_version),
        xml(feed),
        xml(name),
        xml(publisher),
        xml(&artifact.package_version),
        xml(url)
    ))
}
fn prepare_velopack(
    request: &ReleaseRequest,
    prepared: &mut Prepared,
    target: &str,
    items: &[&Input],
    root: &str,
    expiry: u64,
    flag: &AtomicBool,
) -> Result<(), Error> {
    let mut assets = BTreeMap::new();
    for input in items {
        if input.source.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&input.source)?)?;
        let Some(entries) = value.get("Assets").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for asset in entries {
            if asset
                .get("Type")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|value| !value.eq_ignore_ascii_case("full"))
            {
                continue;
            }
            let filename = asset
                .get("FileName")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| fail("Velopack native asset filename missing"))?;
            segment(filename)?;
            let source = items
                .iter()
                .find(|item| item.source.file_name().is_some_and(|name| name == filename))
                .ok_or_else(|| fail("Velopack release family missing full package"))?;
            if asset["PackageId"] != request.identifier
                || asset["Version"] != source.distribution.version
                || asset["Size"].as_u64() != Some(fs::metadata(&source.source)?.len())
                || asset["SHA256"]
                    .as_str()
                    .is_none_or(|hash| !hash.eq_ignore_ascii_case(&source.artifact.sha256))
            {
                return Err(fail("Velopack native feed disagrees with package records"));
            }
            let url = prepared.state.files[&format!(
                "objects/{}",
                object_name(&source.artifact.sha256, filename)?
            )]
                .url
                .clone();
            add(
                prepared,
                feed_name(request, target, filename),
                &source.source,
                url,
            )?;
            if let Some(previous) = assets.insert(filename.to_owned(), asset.clone())
                && previous != *asset
            {
                return Err(fail("conflicting Velopack native asset"));
            }
        }
    }
    if assets.is_empty() {
        return Err(fail("Velopack native release feed missing"));
    }
    let feed = serde_json::to_vec(
        &serde_json::json!({"Assets":assets.into_values().collect::<Vec<_>>()}),
    )?;
    let filename = format!("releases.{}.json", request.channel);
    let mut signature = NativeFeedSignature {
        schema_version: 1,
        published_at: request.published_at,
        expires_at: expiry,
        signature: String::new(),
    };
    signature.signature = hex::encode(signer(
        request,
        "velopack",
        &signature.message(
            "velopack",
            &request.identifier,
            target,
            &request.channel,
            &feed,
        ),
        flag,
    )?);
    generated(prepared, feed_name(request, target, &filename), &feed, root)?;
    generated(
        prepared,
        feed_name(request, target, &format!("{filename}.sig")),
        &serde_json::to_vec(&signature)?,
        root,
    )
}
fn prepare_sparkle(
    request: &ReleaseRequest,
    prepared: &mut Prepared,
    target: &str,
    items: &[&Input],
    root: &str,
    flag: &AtomicBool,
) -> Result<(), Error> {
    let tool = request.config.sparkle_tool.as_ref().ok_or_else(|| {
        fail("release.sparkle_tool must name the pinned SDK generate_appcast executable")
    })?;
    if !cfg!(target_os = "macos") {
        return Err(fail("Sparkle publication requires a macOS worker"));
    }
    let archives = prepared.directory.path().join(format!("sparkle-{target}"));
    fs::create_dir(&archives)?;
    let mut versions = BTreeSet::new();
    for input in items {
        let version = input
            .artifact
            .build_number
            .as_deref()
            .unwrap_or(&input.artifact.package_version);
        // One archive per build counter; ZIP is preferred by input order when both exist.
        if !versions.insert(version) {
            continue;
        }
        let filename = input
            .source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| fail("Sparkle archive filename"))?;
        let filename = object_name(&input.artifact.sha256, filename)?;
        fs::copy(&input.source, archives.join(filename))?;
    }
    let prefix = if matches!(request.config.publisher, Some(Publisher::Github { .. })) {
        format!("{root}/")
    } else {
        format!("{root}/objects/")
    };
    let mut command = Command::new(request.package_root.join(tool));
    command.args([
        "--maximum-deltas",
        "0",
        "--maximum-versions",
        "0",
        "--download-url-prefix",
        &prefix,
    ]);
    if let Some(account) = &request.config.sparkle_account {
        command.args(["--account", account]);
    }
    command.arg(&archives);
    run_tool(&mut command, flag)?;
    let appcast = archives.join("appcast.xml");
    let content = fs::read_to_string(&appcast)?;
    verify_sparkle_feed(
        content.as_bytes(),
        request
            .public_keys
            .get("sparkle")
            .ok_or_else(|| fail("Sparkle verification public key missing"))?,
    )?;
    // generate_appcast signs the native feed using the key embedded in each archive.
    generated(
        prepared,
        feed_name(request, target, "appcast.xml"),
        content.as_bytes(),
        root,
    )
}
fn run_tool(command: &mut Command, flag: &AtomicBool) -> Result<(), Error> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut worker = crate::Worker::spawn_with_io(command)?;
    if !worker.wait(flag, Duration::from_secs(600))?.success() {
        return Err(fail("release native tool failed"));
    }
    Ok(())
}
/// Verifies artifacts, creates signed native metadata, uploads immutable objects,
/// verifies their bytes and only then promotes the complete channel.
///
/// Directory publication uses one exclusive local lease and atomic channel-tree
/// replacement. GitHub creates a draft against an existing tag and exposes it
/// only after all downloaded asset hashes match. Run remote releases in the
/// supplied serialized CI workflow; it is the cross-host promotion lease.
/// # Errors
/// Returns missing gates/credentials, signer failures, conflicts, cancellation,
/// upload/readback failures and native metadata errors. Partial immutable uploads
/// and GitHub drafts are retained for diagnosis; the previous channel stays live.
pub fn publish(request: &ReleaseRequest, flag: &AtomicBool) -> Result<PublishedRelease, Error> {
    if matches!(&request.config.publisher, Some(Publisher::Github { .. }))
        && !request.config.repositories.is_empty()
    {
        return Err(fail(
            "Linux repositories need a directory/static HTTPS publisher",
        ));
    }
    let mut prepared = prepare(request, flag)?;
    match request
        .config
        .publisher
        .as_ref()
        .ok_or_else(|| fail("publisher missing"))?
    {
        Publisher::Directory { path, .. } => publish_directory(
            request,
            &mut prepared,
            &request.package_root.join(path),
            flag,
        )?,
        Publisher::Github { repository, tag } => {
            publish_github(request, &mut prepared, repository, tag, flag)?
        }
        Publisher::Https { url, token_env, .. } => {
            publish_https(request, &mut prepared, url, token_env, flag)?
        }
    }
    Ok(prepared.state)
}
fn publish_directory(
    request: &ReleaseRequest,
    prepared: &mut Prepared,
    root: &Path,
    flag: &AtomicBool,
) -> Result<(), Error> {
    fs::create_dir_all(root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".rake-release.lock"))?;
    lock.try_lock_exclusive()?;
    let marker = root.join(".rake-release");
    if marker.exists() {
        if fs::read_to_string(&marker)? != request.identifier {
            return Err(fail("release directory belongs to another application"));
        }
    } else if fs::read_dir(root)?
        .any(|entry| entry.is_ok_and(|entry| entry.file_name() != ".rake-release.lock"))
    {
        return Err(fail("refusing nonempty unowned release directory"));
    } else {
        fs::write(marker, &request.identifier)?;
    }
    let current = root.join(&request.channel);
    if current.exists() {
        let bytes = fs::read(current.join("release.json"))?;
        let previous: PublishedRelease = serde_json::from_slice(&bytes)?;
        if previous.identifier != request.identifier || previous.channel != request.channel {
            return Err(fail("channel identity conflict"));
        }
        if semver::Version::parse(&previous.version).map_err(|e| fail(e.to_string()))?
            >= semver::Version::parse(&request.version).map_err(|e| fail(e.to_string()))?
        {
            return Err(fail("channel promotion must advance version"));
        }
        prepared.state.previous = Some(files::digest(&current.join("release.json"))?);
    }
    let objects = root.join("objects");
    if objects.is_symlink() {
        return Err(fail("publication object directory must not be a symlink"));
    }
    fs::create_dir_all(&objects)?;
    let tree = tempfile::Builder::new()
        .prefix(".rake-promotion-")
        .tempdir_in(root)?;
    let staged = tree.path().join("channel");
    fs::create_dir(&staged)?;
    for (name, source) in &prepared.objects {
        cancelled(flag)?;
        let record = &prepared.state.files[name];
        let filename = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| fail("publication filename"))?;
        let object = objects.join(object_name(&record.sha256, filename)?);
        if object.is_symlink() {
            return Err(fail("immutable publication object must not be a symlink"));
        }
        if object.exists() {
            if files::digest(&object)? != record.sha256 {
                return Err(fail("immutable publication object conflict"));
            }
        } else {
            let mut temporary = tempfile::NamedTempFile::new_in(&objects)?;
            std::io::copy(&mut fs::File::open(source)?, &mut temporary)?;
            temporary.as_file().sync_all()?;
            temporary.persist_noclobber(&object).map_err(|e| e.error)?;
        }
        if files::digest(&object)? != record.sha256 {
            return Err(fail("published object readback hash mismatch"));
        }
        if let Some(relative) = name.strip_prefix(&format!("{}/", request.channel)) {
            let destination = staged.join(relative);
            fs::create_dir_all(destination.parent().ok_or_else(|| fail("channel path"))?)?;
            fs::hard_link(&object, &destination)?;
        }
    }
    files::write_json(&staged.join("release.json"), &prepared.state)?;
    cancelled(flag)?;
    promote_directory(&staged, &current, root, &request.identifier)?;
    Ok(())
}
fn gh(command: &mut Command, flag: &AtomicBool) -> Result<(), Error> {
    run_tool(command, flag)
}
fn publish_github(
    request: &ReleaseRequest,
    prepared: &mut Prepared,
    repository: &str,
    tag: &str,
    flag: &AtomicBool,
) -> Result<(), Error> {
    let lease = request.package_root.join(".rake-release-github.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lease)?;
    lock.try_lock_exclusive()?;
    // No tag creation and no replacement of existing published releases/assets.
    gh(
        Command::new("gh").args([
            "release",
            "create",
            tag,
            "--repo",
            repository,
            "--verify-tag",
            "--draft",
            "--title",
            &request.version,
            "--notes",
            "Native distribution artifacts",
        ]),
        flag,
    )?;
    let assets = prepared.directory.path().join("github-assets");
    fs::create_dir(&assets)?;
    let mut names = BTreeMap::new();
    for (name, source) in &prepared.objects {
        let record = &prepared.state.files[name];
        let filename = if let Some(value) = name.strip_prefix("objects/") {
            value.to_owned()
        } else {
            name.replace('/', "-")
        };
        segment(&filename)?;
        if let Some(hash) = names.insert(filename.clone(), record.sha256.clone()) {
            if hash != record.sha256 {
                return Err(fail("GitHub release asset name collision"));
            }
            continue;
        }
        let destination = assets.join(&filename);
        fs::copy(source, &destination)?;
        gh(
            Command::new("gh")
                .args(["release", "upload", tag, "--repo", repository])
                .arg(destination),
            flag,
        )?;
    }
    files::write_json(&assets.join("release.json"), &prepared.state)?;
    gh(
        Command::new("gh")
            .args(["release", "upload", tag, "--repo", repository])
            .arg(assets.join("release.json")),
        flag,
    )?;
    let verification = prepared.directory.path().join("github-readback");
    fs::create_dir(&verification)?;
    gh(
        Command::new("gh")
            .args(["release", "download", tag, "--repo", repository, "--dir"])
            .arg(&verification),
        flag,
    )?;
    for (name, hash) in names {
        if files::digest(&verification.join(name))? != hash {
            return Err(fail("GitHub downloaded asset verification failed"));
        }
    }
    cancelled(flag)?;
    gh(
        Command::new("gh").args([
            "release",
            "edit",
            tag,
            "--repo",
            repository,
            "--draft=false",
            "--latest",
        ]),
        flag,
    )
}
fn publish_https(
    request: &ReleaseRequest,
    prepared: &mut Prepared,
    root: &str,
    token_env: &str,
    flag: &AtomicBool,
) -> Result<(), Error> {
    let token = std::env::var(token_env).map_err(|_| {
        fail(format!(
            "publisher credential environment variable missing: {token_env}"
        ))
    })?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .http_status_as_error(false)
        .build()
        .into();
    let channel = format!(
        "{}/channels/{}",
        root.trim_end_matches('/'),
        request.channel
    );
    let mut previous = agent
        .get(&channel)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|_| fail("HTTPS channel lookup failed"))?;
    let etag = if previous.status() == 200 {
        let etag = previous
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| fail("HTTPS publisher must return channel ETag"))?
            .to_owned();
        let bytes = previous
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_vec()
            .map_err(|_| fail("HTTPS channel read failed"))?;
        let state: PublishedRelease = serde_json::from_slice(&bytes)?;
        if state.identifier != request.identifier
            || state.channel != request.channel
            || semver::Version::parse(&state.version).map_err(|e| fail(e.to_string()))?
                >= semver::Version::parse(&request.version).map_err(|e| fail(e.to_string()))?
        {
            return Err(fail("HTTPS channel identity/version conflict"));
        }
        prepared.state.previous = Some(etag.clone());
        Some(etag)
    } else if previous.status() == 404 {
        None
    } else {
        return Err(fail("HTTPS channel lookup rejected"));
    };
    for (name, path) in &prepared.objects {
        cancelled(flag)?;
        let record = &prepared.state.files[name];
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| fail("publication filename"))?;
        let url = format!(
            "{}/objects/{}",
            root.trim_end_matches('/'),
            object_name(&record.sha256, filename)?
        );
        let response = agent
            .put(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("If-None-Match", "*")
            .send(fs::File::open(path)?)
            .map_err(|_| fail("HTTPS immutable upload failed"))?;
        if !response.status().is_success() && response.status() != 412 {
            return Err(fail("HTTPS immutable upload rejected"));
        }
        let mut response = agent
            .get(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|_| fail("HTTPS readback failed"))?;
        if response.status() != 200 {
            return Err(fail("HTTPS readback rejected"));
        }
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        let mut body = response.body_mut().as_reader().take(record.size + 1);
        let count = std::io::copy(&mut body, &mut hash)?;
        if count != record.size || format!("{:x}", hash.finalize()) != record.sha256 {
            return Err(fail("HTTPS readback verification failed"));
        }
    }
    cancelled(flag)?;
    let bytes = serde_json::to_vec(&prepared.state)?;
    let response = agent
        .put(&channel)
        .header("Authorization", &format!("Bearer {token}"))
        .header(
            if etag.is_some() {
                "If-Match"
            } else {
                "If-None-Match"
            },
            etag.as_deref().unwrap_or("*"),
        )
        .header("Content-Type", "application/json")
        .send(bytes.as_slice())
        .map_err(|_| fail("HTTPS transactional promotion failed"))?;
    if !response.status().is_success() {
        return Err(fail(
            "HTTPS promotion conflict/rejection; previous channel retained",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, version: &str) -> ReleaseRequest {
        let input = root.join(version);
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("app.tar.gz"), format!("payload {version}")).unwrap();
        let distribution = Distribution {
            schema_version: 1,
            identifier: "dev.rake.release-test".into(),
            version: version.into(),
            target: "x86_64-unknown-linux-gnu".into(),
            engine: "test".into(),
            custom_artifacts: vec![],
            artifacts: vec![Artifact {
                path: "app.tar.gz".into(),
                format: Format::Tar,
                sha256: files::digest(&input.join("app.tar.gz")).unwrap(),
                package_version: version.into(),
                components: vec![],
                trust: Default::default(),
                linux: None,
                build_number: None,
                native_identity: BTreeMap::new(),
                update_policy: None,
            }],
        };
        files::write_json(&input.join("distribution.json"), &distribution).unwrap();
        ReleaseRequest {
            identifier: distribution.identifier,
            version: version.into(),
            channel: "stable".into(),
            distributions: vec![input],
            config: ReleaseConfig {
                publisher: Some(Publisher::Directory {
                    path: root.join("published"),
                    public_url: "https://downloads.example.test".into(),
                }),
                required: BTreeMap::from([("x86_64-unknown-linux-gnu".into(), vec![Format::Tar])]),
                ..Default::default()
            },
            package_root: root.into(),
            public_keys: BTreeMap::new(),
            published_at: 1_800_000_000,
            updates_enabled: false,
            custom_verifiers: BTreeMap::new(),
        }
    }
    #[test]
    fn absent_target_and_corrupted_payload_cannot_advance_channel() {
        let root = tempfile::tempdir().unwrap();
        let flag = AtomicBool::new(false);
        let initial = fixture(root.path(), "1.0.0");
        let first = publish(&initial, &flag).unwrap();
        assert!(first.previous.is_none());
        let old = fs::read(root.path().join("published/stable/release.json")).unwrap();
        let mut next = fixture(root.path(), "1.0.1");
        next.config
            .required
            .insert("aarch64-unknown-linux-gnu".into(), vec![Format::Tar]);
        assert!(
            publish(&next, &flag)
                .unwrap_err()
                .to_string()
                .contains("release gate missing")
        );
        assert_eq!(
            fs::read(root.path().join("published/stable/release.json")).unwrap(),
            old
        );
        next.config.required.remove("aarch64-unknown-linux-gnu");
        fs::write(next.distributions[0].join("app.tar.gz"), "corrupted").unwrap();
        assert!(
            publish(&next, &flag)
                .unwrap_err()
                .to_string()
                .contains("hash mismatch")
        );
        assert_eq!(
            fs::read(root.path().join("published/stable/release.json")).unwrap(),
            old
        );
    }
    #[test]
    fn promotion_is_serialized_retains_history_and_rejects_replay() {
        let root = tempfile::tempdir().unwrap();
        let flag = AtomicBool::new(false);
        let initial = fixture(root.path(), "1.0.0");
        let first = publish(&initial, &flag).unwrap();
        let original_url = first.files.values().next().unwrap().url.clone();
        let next = fixture(root.path(), "1.0.1");
        let state = publish(&next, &flag).unwrap();
        assert!(state.previous.is_some());
        let original_object = original_url
            .strip_prefix("https://downloads.example.test/")
            .unwrap();
        assert!(
            root.path()
                .join("published")
                .join(original_object)
                .is_file()
        );
        assert!(
            publish(&initial, &flag)
                .unwrap_err()
                .to_string()
                .contains("must advance")
        );
        let lock = fs::OpenOptions::new()
            .write(true)
            .open(root.path().join("published/.rake-release.lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        let future = fixture(root.path(), "1.0.2");
        assert!(publish(&future, &flag).is_err());
    }
    #[test]
    fn cancellation_and_unowned_output_preserve_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let request = fixture(root.path(), "1.0.0");
        let published = root.path().join("published");
        fs::create_dir(&published).unwrap();
        fs::write(published.join("valuable"), "keep").unwrap();
        assert!(
            publish(&request, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("unowned")
        );
        assert_eq!(
            fs::read_to_string(published.join("valuable")).unwrap(),
            "keep"
        );
        assert!(publish(&request, &AtomicBool::new(true)).is_err());
        assert!(!published.join("stable").exists());
    }
    #[test]
    fn metadata_files_do_not_satisfy_package_gates_and_compiled_policy_is_binding() {
        let root = tempfile::tempdir().unwrap();
        let mut request = fixture(root.path(), "1.0.0");
        let manifest = request.distributions[0].join("distribution.json");
        let mut distribution: Distribution =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        distribution.artifacts[0].format = Format::Flatpak;
        request
            .config
            .required
            .get_mut("x86_64-unknown-linux-gnu")
            .unwrap()[0] = Format::Flatpak;
        files::write_json(&manifest, &distribution).unwrap();
        assert!(publish(&request, &AtomicBool::new(false)).is_err());
        assert!(!root.path().join("published").exists());
        distribution.artifacts[0].format = Format::Tar;
        request
            .config
            .required
            .get_mut("x86_64-unknown-linux-gnu")
            .unwrap()[0] = Format::Tar;
        request.updates_enabled = true;
        distribution.artifacts[0].update_policy = Some(rake_dist_core::UpdateBinding {
            schema_version: 1,
            identifier: request.identifier.clone(),
            version: request.version.clone(),
            package_version: request.version.clone(),
            target: distribution.target.clone(),
            format: Format::Tar,
            owner: rake_dist_core::UpdateOwner::External,
            channel: "beta".into(),
            base_url: Some("https://other.example.test".into()),
            public_keys: BTreeMap::new(),
        });
        files::write_json(&manifest, &distribution).unwrap();
        assert!(
            publish(&request, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("compiled artifact update policy")
        );
        assert!(!root.path().join("published").exists());
    }
    #[test]
    fn release_uses_a_verified_snapshot_even_if_build_outputs_change() {
        let root = tempfile::tempdir().unwrap();
        let request = fixture(root.path(), "1.0.0");
        let input = request.distributions[0].join("app.tar.gz");
        let expected = fs::read(&input).unwrap();
        let mut prepared = prepare(&request, &AtomicBool::new(false)).unwrap();
        fs::write(input, b"a later build replaced this source").unwrap();
        let destination = root.path().join("published");
        publish_directory(
            &request,
            &mut prepared,
            &destination,
            &AtomicBool::new(false),
        )
        .unwrap();
        let url = &prepared.state.files.values().next().unwrap().url;
        let relative = url.strip_prefix("https://downloads.example.test/").unwrap();
        assert_eq!(fs::read(destination.join(relative)).unwrap(), expected);
    }
    #[test]
    fn custom_releases_require_independent_verification_and_safe_object_names() {
        let root = tempfile::tempdir().unwrap();
        let mut request = fixture(root.path(), "1.0.0");
        let manifest = request.distributions[0].join("distribution.json");
        let file = request.distributions[0].join("Company Installer.pkg");
        fs::write(&file, b"custom format payload").unwrap();
        let hash = files::digest(&file).unwrap();
        let command = vec!["tools/verify-company-installer".into()];
        let mut distribution: Distribution =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        distribution.custom_artifacts.push(crate::CustomArtifact {
            format: "company-installer".into(),
            path: "Company Installer.pkg".into(),
            sha256: hash.clone(),
            verification: None,
            verifier: Some(command.clone()),
        });
        files::write_json(&manifest, &distribution).unwrap();
        assert!(publish(&request, &AtomicBool::new(false)).is_err());
        assert!(!root.path().join("published").exists());
        request
            .custom_verifiers
            .insert("company-installer".into(), command);
        request.config.custom_required.insert(
            distribution.target.clone(),
            vec!["company-installer".into()],
        );
        distribution.custom_artifacts[0].verification = Some(rake_dist_core::Verification {
            schema_version: 1,
            sha256: hash,
            policy: "fixture-format-contract".into(),
        });
        files::write_json(&manifest, &distribution).unwrap();
        let state = publish(&request, &AtomicBool::new(false)).unwrap();
        assert!(
            state
                .files
                .values()
                .any(|file| file.url.ends_with("Company_Installer.pkg"))
        );
        assert!(state.files.values().all(|file| !file.url.contains(' ')));
    }
    #[test]
    fn appinstaller_preserves_actual_native_identity_and_escapes_xml() {
        let root = tempfile::tempdir().unwrap();
        let request = fixture(root.path(), "1.0.0");
        let mut distribution: Distribution = serde_json::from_slice(
            &fs::read(request.distributions[0].join("distribution.json")).unwrap(),
        )
        .unwrap();
        distribution.target = "x86_64-pc-windows-msvc".into();
        let mut artifact = distribution.artifacts[0].clone();
        artifact.package_version = "1.2.3.4".into();
        artifact.native_identity = BTreeMap::from([
            ("msix_name".into(), "dev.rake.test".into()),
            ("msix_publisher".into(), "CN=Rake & Co".into()),
        ]);
        let xml = appinstaller(
            &distribution,
            &artifact,
            "https://example.test/feed",
            "https://example.test/app.msix",
        )
        .unwrap();
        assert!(xml.contains("CN=Rake &amp; Co"));
        assert!(xml.contains("Version=\"1.2.3.4\""));
        assert!(xml.contains("ProcessorArchitecture=\"x64\""));
    }
}

fn verify_sparkle_feed(bytes: &[u8], key: &[u8]) -> Result<(), Error> {
    use base64::Engine as _;
    let text = std::str::from_utf8(bytes).map_err(|_| fail("Sparkle feed is not UTF-8"))?;
    let marker = text
        .rfind("<!-- sparkle-signatures:")
        .ok_or_else(|| fail("Sparkle native feed signature missing"))?;
    let trailer = &text[marker..];
    if !trailer.trim_end().ends_with("-->") {
        return Err(fail("Sparkle signature trailer malformed"));
    }
    let signature = trailer
        .lines()
        .find_map(|line| line.strip_prefix("edSignature: "))
        .ok_or_else(|| fail("Sparkle native Ed25519 signature missing"))?;
    let length = trailer
        .lines()
        .find_map(|line| line.strip_prefix("length: "))
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| fail("Sparkle native signed length missing"))?;
    if length != marker {
        return Err(fail("Sparkle signed feed length mismatch"));
    }
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|_| fail("Sparkle feed signature malformed"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&decode_key(key)?)
        .map_err(|e| fail(e.to_string()))?;
    let signature =
        ed25519_dalek::Signature::from_slice(&signature).map_err(|e| fail(e.to_string()))?;
    key.verify_strict(&bytes[..length], &signature)
        .map_err(|_| fail("Sparkle signing profile does not match the configured public key"))?;
    Ok(())
}
#[cfg(unix)]
fn promote_directory(
    staged: &Path,
    current: &Path,
    root: &Path,
    identifier: &str,
) -> Result<(), Error> {
    let generations = root.join(".rake-channels");
    if generations.is_symlink() {
        return Err(fail("channel generation directory must not be a symlink"));
    }
    fs::create_dir_all(&generations)?;
    if current.exists() {
        let actual = fs::canonicalize(current)?;
        if !current.is_symlink()
            || !actual.starts_with(fs::canonicalize(&generations)?)
            || fs::read_to_string(actual.join(".rake-distribution"))? != identifier
        {
            return Err(fail("refusing unowned release channel"));
        }
    }
    fs::write(staged.join(".rake-distribution"), identifier)?;
    let name = files::digest(&staged.join("release.json"))?;
    let generation = generations.join(&name);
    if generation.exists() {
        return Err(fail("channel generation already exists"));
    }
    fs::rename(staged, &generation)?;
    let link = root.join(format!(".rake-promote-{name}"));
    std::os::unix::fs::symlink(Path::new(".rake-channels").join(name), &link)?;
    if let Err(error) = fs::rename(&link, current) {
        let _ = fs::remove_file(link);
        return Err(error.into());
    }
    fs::File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn promote_directory(
    staged: &Path,
    current: &Path,
    _root: &Path,
    identifier: &str,
) -> Result<(), Error> {
    files::commit(staged, current, identifier)
}
#[cfg(test)]
mod signature_tests {
    use super::*;
    #[test]
    fn real_sparkle_2101_signed_feed_is_verified_and_tampering_fails() {
        let feed = include_bytes!("../tests/fixtures/sparkle/appcast.xml");
        let key = include_bytes!("../tests/fixtures/sparkle/public-key.txt");
        verify_sparkle_feed(feed, key).unwrap();
        let mut changed = feed.to_vec();
        changed[100] ^= 1;
        assert!(verify_sparkle_feed(&changed, key).is_err());
        assert!(verify_sparkle_feed(feed, &[7; 32]).is_err());
    }
}

pub(crate) fn package_payload(format: Format, path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match format {
        Format::App => false,
        Format::Dmg => name.ends_with(".dmg"),
        Format::Zip => name.ends_with(".zip"),
        Format::Tar => name.ends_with(".tar.gz") || name.ends_with(".tar"),
        Format::Nsis | Format::Velopack => name.ends_with(".exe"),
        Format::Msi => name.ends_with(".msi"),
        Format::Msix => name.ends_with(".msix"),
        Format::Deb => name.ends_with(".deb"),
        Format::Rpm => name.ends_with(".rpm"),
        Format::Arch => name.contains(".pkg.tar."),
        Format::Appimage => name.ends_with(".appimage"),
        Format::Flatpak => name.ends_with(".flatpak"),
        _ => false,
    }
}
