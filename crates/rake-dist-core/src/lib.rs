//! Portable distribution configuration and validated package plans.
//!
//! Resolving a plan performs no filesystem, network or native-tool operations.

mod component;
pub use component::{Archive, Component, ComponentSource};
mod release;
pub use release::{Publisher, ReleaseConfig, feed_url};
mod installation;
mod update_binding;
mod update_feed;
pub use installation::Installation;
pub use update_binding::UpdateBinding;
pub use update_feed::{NativeFeedSignature, SignedUpdateFeed, UpdateFeed, UpdateRelease};
mod overrides;
mod trust;
pub use trust::{TrustOptions, TrustRecord};
mod version;
pub use overrides::{Overrides, UpdateOverrides};
pub use version::PackageVersion;

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Operating system whose packages are being planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Platform {
    /// Apple desktop platform.
    Macos,
    /// Microsoft desktop platform.
    Windows,
    /// Linux desktop platform.
    Linux,
}

impl Platform {
    /// Resolves a supported desktop OS from a Rust target triple.
    /// # Errors
    /// Rejects malformed triples and unsupported operating systems.
    pub fn from_target(target: &str) -> Result<Self, Error> {
        let triple = target
            .parse::<target_lexicon::Triple>()
            .map_err(|_| Error::Field("target triple".into()))?;
        match triple.operating_system {
            target_lexicon::OperatingSystem::Darwin(_)
            | target_lexicon::OperatingSystem::MacOSX(_) => Ok(Self::Macos),
            target_lexicon::OperatingSystem::Windows => Ok(Self::Windows),
            target_lexicon::OperatingSystem::Linux => Ok(Self::Linux),
            _ => Err(Error::Field("unsupported target operating system".into())),
        }
    }
}

/// Installation format, independent of build target and release channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Format {
    /// macOS application bundle.
    App,
    /// macOS disk image.
    Dmg,
    /// Native ZIP archive.
    Zip,
    /// Legacy Linux directory archive.
    Tar,
    /// Windows NSIS installer.
    Nsis,
    /// Windows Installer package.
    Msi,
    /// Windows managed package.
    Msix,
    /// Velopack installer and release family.
    Velopack,
    /// Debian-family package.
    Deb,
    /// RPM-family package.
    Rpm,
    /// Arch package and recipe.
    Arch,
    /// Portable Linux application image.
    Appimage,
    /// Sandboxed Linux package.
    Flatpak,
}

impl Format {
    /// Returns whether the format belongs to the given platform.
    pub fn supports(self, platform: Platform) -> bool {
        match self {
            Self::App | Self::Dmg => platform == Platform::Macos,
            Self::Zip => platform == Platform::Macos || platform == Platform::Windows,
            Self::Nsis | Self::Msi | Self::Msix | Self::Velopack => platform == Platform::Windows,
            Self::Tar | Self::Deb | Self::Rpm | Self::Arch | Self::Appimage | Self::Flatpak => {
                platform == Platform::Linux
            }
        }
    }
}

/// Mechanism responsible for an installed application's updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UpdateOwner {
    /// Checks are disabled by application configuration.
    Disabled,
    /// Sparkle owns installation and relaunch.
    Sparkle,
    /// Windows App Installer owns updates.
    AppInstaller,
    /// Velopack owns its native installation.
    Velopack,
    /// The configured APT repository supplies updates.
    Apt,
    /// An RPM package manager supplies updates.
    RpmRepository,
    /// Pacman supplies updates.
    Pacman,
    /// Flatpak supplies updates.
    Flatpak,
    /// A validated standalone image adapter supplies updates.
    Appimage,
    /// The user or another external tool handles updates.
    External,
}

/// Project-provided metadata before target selection or validation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Missing means the existing legacy bundle contract.
    pub schema_version: Option<u32>,
    /// Display name; defaults to the Cargo package name.
    pub name: Option<String>,
    /// Stable reverse-domain application identity.
    pub identifier: Option<String>,
    /// Source icon relative to the package directory.
    pub icon: Option<PathBuf>,
    /// Legacy resource tree relative to the package directory.
    pub assets: Option<PathBuf>,
    /// Existing numeric bundle build number.
    pub build_number: Option<String>,
    /// Native package version override for restricted version formats.
    pub package_version: Option<String>,
    /// Per-Rust-target partial overrides.
    pub target_overrides: BTreeMap<String, Overrides>,
    /// Per-installation-format partial overrides.
    pub format_overrides: BTreeMap<Format, Overrides>,
    /// Existing macOS deployment declaration.
    pub minimum_system_version: Option<String>,
    /// Existing Web configuration remains owned by the Web pipeline.
    pub web: WebConfig,
    /// macOS overrides.
    pub macos: MacosConfig,
    /// Windows overrides.
    pub windows: WindowsConfig,
    /// Linux overrides.
    pub linux: LinuxConfig,
    /// Pinned native component sources by name and exact target.
    pub components: Vec<Component>,
    /// Mapped resource inputs.
    pub resources: Vec<Resource>,
    /// Architecture-specific helper executables.
    pub sidecars: Vec<Sidecar>,
    /// Native runtime recipes.
    pub runtimes: Vec<Runtime>,
    /// Update settings shared across installation channels.
    pub updates: Updates,
    /// Publication, signing references and required release targets.
    pub release: ReleaseConfig,
}

/// Existing Web bundle options.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    /// Host URL prefix.
    pub base_path: Option<String>,
    /// Existing path or hash routing selection.
    pub routing: Option<String>,
}

/// macOS-specific packaging settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct MacosConfig {
    /// Build and validate Apple Silicon and Intel slices in one application.
    pub universal: bool,
    /// Selected native formats.
    pub formats: Vec<Format>,
    /// Custom plist overlay.
    pub info_plist: Option<PathBuf>,
    /// Signing entitlements.
    pub entitlements: Option<PathBuf>,
    /// Existing local Developer ID signing identity.
    pub signing_identity: Option<String>,
    /// Existing notarytool Keychain profile.
    pub notary_profile: Option<String>,
}

/// Windows-specific packaging settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct WindowsConfig {
    /// Selected native formats.
    pub formats: Vec<Format>,
    /// Custom Win32 application manifest.
    pub manifest: Option<PathBuf>,
    /// Managed package identity.
    pub msix: MsixConfig,
    /// Current-user certificate store thumbprint; never a private-key path.
    pub certificate_thumbprint: Option<String>,
    /// RFC 3161 timestamp service URL used for release signing.
    pub timestamp_url: Option<String>,
}

/// Windows managed package identity overrides.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct MsixConfig {
    /// Certificate-matching publisher identity.
    pub publisher: Option<String>,
    /// Optional complete AppxManifest.xml retaining the generated identity.
    pub manifest: Option<PathBuf>,
}

/// Linux-specific packaging settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LinuxConfig {
    /// Selected native formats.
    pub formats: Vec<Format>,
    /// Desktop-entry template.
    pub desktop_entry: Option<PathBuf>,
    /// AppStream metadata.
    pub appstream: Option<PathBuf>,
    /// Stable native package name; defaults to the Cargo package name.
    pub package_name: Option<String>,
    /// Native dependency expressions for each Linux package manager.
    pub dependencies: BTreeMap<Format, Vec<String>>,
    /// Explicit runtime compatibility profile.
    pub baseline: Option<LinuxBaseline>,
    /// Flatpak build configuration.
    pub flatpak: FlatpakConfig,
}

/// Declared Linux compatibility profile, recorded with the final artifact.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LinuxBaseline {
    /// Distribution/profile label, such as ubuntu-24.04.
    pub name: String,
    /// Minimum glibc version; native ELF verification rejects newer requirements.
    pub glibc: Option<String>,
    /// Minimum kernel version documented for this application.
    pub kernel: Option<String>,
}

/// Flatpak build override.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct FlatpakConfig {
    /// Manifest identifying the pinned runtime and SDK.
    pub manifest: Option<PathBuf>,
    /// Flatpak runtime ID.
    pub runtime: String,
    /// Matching SDK ID.
    pub sdk: String,
    /// Runtime/SDK branch.
    pub runtime_version: String,
    /// Optional exact installed runtime commit.
    pub runtime_commit: Option<String>,
    /// Optional exact installed SDK commit.
    pub sdk_commit: Option<String>,
    /// Explicit runtime permissions; defaults to graphics and display sockets.
    pub finish_args: Vec<String>,
}

impl Default for FlatpakConfig {
    fn default() -> Self {
        Self {
            manifest: None,
            runtime: "org.freedesktop.Platform".into(),
            sdk: "org.freedesktop.Sdk".into(),
            runtime_version: "25.08".into(),
            runtime_commit: None,
            sdk_commit: None,
            finish_args: vec![
                "--socket=wayland".into(),
                "--socket=fallback-x11".into(),
                "--share=ipc".into(),
                "--device=dri".into(),
            ],
        }
    }
}

/// Explicit input-to-installed-resource mapping.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// Package-relative source.
    pub from: PathBuf,
    /// Confined installed destination.
    pub to: PathBuf,
}

/// A helper executable resolved for a selected target.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Sidecar {
    /// Stable application-facing name.
    pub name: String,
    /// Package-relative path with documented target placeholders.
    pub path: PathBuf,
    /// Named component root instead of the package root.
    #[serde(default)]
    pub component: Option<String>,
}

/// Native runtime deployment request.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    /// Stable runtime identity.
    pub name: String,
    /// Registered deployment recipe.
    pub recipe: String,
    /// Package-relative runtime source; omitted for system-managed dependencies.
    #[serde(default)]
    pub source: PathBuf,
    /// Declare packages supplied by the Linux installation channel instead of bundling.
    #[serde(default)]
    pub system: bool,
    /// Package-manager-specific requirements for a system runtime.
    #[serde(default)]
    pub system_dependencies: BTreeMap<Format, Vec<String>>,
    /// Link this runtime into the main process as well as component helpers.
    #[serde(default)]
    pub link_from_app: bool,
    /// Named component root instead of the package root.
    #[serde(default)]
    pub component: Option<String>,
    /// Explicit dynamically loaded plugin selection.
    #[serde(default)]
    pub plugins: Vec<String>,
}

/// Shared public update settings; contains no private signing credentials.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Updates {
    /// Whether update support is requested.
    pub enabled: bool,
    /// Release track; defaults to stable.
    pub channel: Option<String>,
    /// HTTPS publication root.
    pub base_url: Option<String>,
    /// Provider public-key files relative to the package.
    pub public_keys: BTreeMap<String, PathBuf>,
}

/// Validated request for one package; does not assert backend availability.
#[derive(Clone, Debug, Serialize)]
pub struct PackagePlan {
    format: Format,
    update_owner: UpdateOwner,
    version: PackageVersion,
    config: Config,
}

impl PackagePlan {
    /// Returns this artifact's native version mapping.
    pub fn version(&self) -> &PackageVersion {
        &self.version
    }
    /// Returns the effective per-format configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }
    /// Returns the requested installation format.
    pub fn format(&self) -> Format {
        self.format
    }
    /// Returns the update owner determined by the installation channel.
    pub fn update_owner(&self) -> UpdateOwner {
        self.update_owner
    }
}

/// Portable, validated plan that must still pass native capability checks.
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    name: String,
    identifier: String,
    version: String,
    platform: Platform,
    legacy: bool,
    config: Config,
    packages: Vec<PackagePlan>,
}

impl Plan {
    /// Returns the resolved display name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the validated stable application identifier.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
    /// Returns the Cargo semantic version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Returns the selected packaging platform.
    pub fn platform(&self) -> Platform {
        self.platform
    }
    /// Returns the validated configuration snapshot used by later native checks.
    pub fn config(&self) -> &Config {
        &self.config
    }
    /// Returns the installation requests in configured order.
    pub fn packages(&self) -> &[PackagePlan] {
        &self.packages
    }
    /// Returns whether legacy bundle behavior was selected.
    pub fn is_legacy(&self) -> bool {
        self.legacy
    }
}

/// Configuration errors detected before any native tools or network operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Unsupported serialized configuration revision.
    #[error("unsupported distribution schema {0}")]
    Schema(u32),
    /// Invalid identity, name, channel or version value.
    #[error("invalid distribution field: {0}")]
    Field(String),
    /// Input or installed path escapes its declared root.
    #[error("path must be a confined relative path: {0}")]
    Path(PathBuf),
    /// A requested format cannot be produced for the selected OS.
    #[error("format {format:?} does not support {platform:?}")]
    Format {
        /// Requested installation format.
        format: Format,
        /// Selected operating system.
        platform: Platform,
    },
    /// Repeated names or output requests would collide.
    #[error("duplicate distribution entry: {0}")]
    Duplicate(String),
}

impl Config {
    /// Resolves portable defaults and update ownership without performing I/O.
    ///
    /// # Errors
    /// Returns an error for unsupported schemas, invalid metadata/paths, duplicate
    /// names/formats, missing required public settings or incompatible formats.
    pub fn plan(&self, package: &str, version: &str, platform: Platform) -> Result<Plan, Error> {
        self.plan_for(package, version, platform, None, None)
    }

    /// Applies target, format and explicit CLI format selection in that order.
    ///
    /// # Errors
    /// Returns errors for malformed target names, forbidden format-list overrides,
    /// and invalid effective configuration or native version mappings.
    pub fn plan_for(
        &self,
        package: &str,
        version: &str,
        platform: Platform,
        target: Option<&str>,
        formats: Option<&[Format]>,
    ) -> Result<Plan, Error> {
        let mut base = self.clone();
        base.normalize_paths()?;
        if platform == Platform::Linux && base.linux.package_name.is_none() {
            base.linux.package_name = Some(package.to_ascii_lowercase().replace('_', "-"));
        }
        if let Some(target) = target {
            let matches_platform = if target == "universal-apple-darwin"
                && platform == Platform::Macos
                && base.macos.universal
            {
                true
            } else {
                Platform::from_target(target)? == platform
            };
            if !matches_platform {
                return Err(Error::Field(
                    "Rust target triple does not match platform".into(),
                ));
            }
            if let Some(layer) = self.target_overrides.get(target) {
                layer.apply(&mut base, platform);
                base.normalize_paths()?;
            }
        } else if !self.target_overrides.is_empty() {
            return Err(Error::Field(
                "target overrides require an explicit Rust target triple".into(),
            ));
        }
        if let Some(formats) = formats {
            *base.formats_mut(platform) = formats.to_vec();
        }
        let requested = base.selected_formats(platform);
        let mut packages = Vec::new();
        for format in requested {
            if packages.iter().any(|p: &PackagePlan| p.format == format) {
                return Err(Error::Duplicate(format!("{format:?}")));
            }
            let mut effective = base.clone();
            if let Some(layer) = self.format_overrides.get(&format) {
                if layer.formats.is_some() {
                    return Err(Error::Field(
                        "format overrides cannot change format lists".into(),
                    ));
                }
                layer.apply(&mut effective, platform);
                effective.normalize_paths()?;
            }
            if platform == Platform::Macos && effective.updates.enabled {
                if let Some(minimum) = &effective.minimum_system_version {
                    if minimum
                        .split('.')
                        .next()
                        .and_then(|value| value.parse::<u32>().ok())
                        .is_none_or(|major| major < 12)
                    {
                        return Err(Error::Field(
                            "Sparkle 2.10.1 requires minimum_system_version >= 12.0".into(),
                        ));
                    }
                } else {
                    effective.minimum_system_version = Some("12.0".into());
                }
            }
            *effective.formats_mut(platform) = vec![format];
            for name in effective
                .sidecars
                .iter()
                .filter_map(|item| item.component.as_deref())
                .chain(
                    effective
                        .runtimes
                        .iter()
                        .filter_map(|item| item.component.as_deref()),
                )
            {
                if !effective.components.iter().any(|component| {
                    component.name == name
                        && target.is_none_or(|target| {
                            component.target == target
                                || (platform == Platform::Macos
                                    && effective.macos.universal
                                    && component.target == "universal-apple-darwin")
                        })
                }) {
                    return Err(Error::Field(format!(
                        "component {name} has no source for the selected target"
                    )));
                }
            }
            let mut validated = effective.plan_effective(package, version, platform)?;
            packages.append(&mut validated.packages);
        }
        let mut result = base;
        result.target_overrides.clear();
        result.format_overrides.clear();
        let name = result.name.clone().unwrap_or_else(|| package.into());
        let identifier = result
            .identifier
            .clone()
            .unwrap_or_else(|| format!("dev.rake.{}", package.replace('_', "-")));
        Ok(Plan {
            name,
            identifier,
            version: version.into(),
            platform,
            legacy: self.schema_version.is_none(),
            config: result,
            packages,
        })
    }

    pub(crate) fn formats_mut(&mut self, platform: Platform) -> &mut Vec<Format> {
        match platform {
            Platform::Macos => &mut self.macos.formats,
            Platform::Windows => &mut self.windows.formats,
            Platform::Linux => &mut self.linux.formats,
        }
    }

    fn selected_formats(&self, platform: Platform) -> Vec<Format> {
        let configured = match platform {
            Platform::Macos => &self.macos.formats,
            Platform::Windows => &self.windows.formats,
            Platform::Linux => &self.linux.formats,
        };
        if !configured.is_empty() {
            return configured.clone();
        }
        vec![match (platform, self.schema_version.is_none()) {
            (Platform::Macos, true) => Format::Zip,
            (Platform::Macos, false) => Format::Dmg,
            (Platform::Windows, _) => Format::Nsis,
            (Platform::Linux, true) => Format::Tar,
            (Platform::Linux, false) => Format::Appimage,
        }]
    }

    fn plan_effective(
        &self,
        package: &str,
        version: &str,
        platform: Platform,
    ) -> Result<Plan, Error> {
        if let Some(schema) = self.schema_version
            && schema != 1
        {
            return Err(Error::Schema(schema));
        }
        self.release.validate()?;
        let legacy = self.schema_version.is_none();
        let name = self.name.clone().unwrap_or_else(|| package.into());
        if name.trim().is_empty()
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err(Error::Field("name".into()));
        }
        let identifier = self
            .identifier
            .clone()
            .unwrap_or_else(|| format!("dev.rake.{}", package.replace('_', "-")));
        if !identifier.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) || !identifier.contains('.')
        {
            return Err(Error::Field("identifier".into()));
        }
        let parsed_version =
            semver::Version::parse(version).map_err(|_| Error::Field("version".into()))?;
        let paths = [
            self.icon.as_ref(),
            self.assets.as_ref(),
            self.macos.info_plist.as_ref(),
            self.macos.entitlements.as_ref(),
            self.windows.manifest.as_ref(),
            self.windows.msix.manifest.as_ref(),
            self.linux.desktop_entry.as_ref(),
            self.linux.appstream.as_ref(),
            self.linux.flatpak.manifest.as_ref(),
        ];
        for path in paths.into_iter().flatten() {
            validate_relative_path(path)?;
        }
        let mut components = std::collections::BTreeSet::new();
        for component in &self.components {
            component_name(&component.name)?;
            if component.target != "universal-apple-darwin"
                && component.target.parse::<target_lexicon::Triple>().is_err()
            {
                return Err(Error::Field("component target".into()));
            }
            if !components.insert((&component.name, &component.target)) {
                return Err(Error::Duplicate(component.name.clone()));
            }
            for path in &component.licenses {
                validate_relative_path(path)?;
            }
            match &component.source {
                ComponentSource::Local { path } => validate_relative_path(path)?,
                ComponentSource::Https {
                    url,
                    sha256,
                    max_download_bytes,
                    ..
                } => {
                    let url =
                        url::Url::parse(url).map_err(|_| Error::Field("component URL".into()))?;
                    if url.scheme() != "https"
                        || url.host_str().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.fragment().is_some()
                    {
                        return Err(Error::Field("component requires public HTTPS URL".into()));
                    }
                    if sha256.len() != 64
                        || !sha256
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                        || *max_download_bytes == 0
                        || *max_download_bytes > 4 * 1024 * 1024 * 1024
                    {
                        return Err(Error::Field("component hash/download limit".into()));
                    }
                }
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for sidecar in &self.sidecars {
            component_name(&sidecar.name)?;
            if let Some(component) = &sidecar.component {
                component_name(component)?;
            }
            if !names.insert(&sidecar.name) {
                return Err(Error::Duplicate(sidecar.name.clone()));
            }
            validate_relative_path(&sidecar.path)?;
        }
        for runtime in &self.runtimes {
            component_name(&runtime.name)?;
            if let Some(component) = &runtime.component {
                component_name(component)?;
            }
            for plugin in &runtime.plugins {
                component_name(plugin)?;
            }
            if !names.insert(&runtime.name) {
                return Err(Error::Duplicate(runtime.name.clone()));
            }
            if runtime.system {
                if platform != Platform::Linux
                    || !runtime.source.as_os_str().is_empty()
                    || runtime.component.is_some()
                    || !runtime.plugins.is_empty()
                {
                    return Err(Error::Field("system runtimes require Linux dependency declarations without bundled inputs".into()));
                }
            } else {
                validate_relative_path(&runtime.source)?;
            }
        }
        let mut destinations = std::collections::BTreeSet::new();
        for resource in &self.resources {
            validate_relative_path(&resource.from)?;
            validate_relative_path(&resource.to)?;
            if !destinations.insert(&resource.to) {
                return Err(Error::Duplicate(resource.to.display().to_string()));
            }
        }
        for path in self.updates.public_keys.values() {
            validate_relative_path(path)?;
        }
        if self.updates.enabled {
            component_name(self.updates.channel.as_deref().unwrap_or("stable"))?;
            let base = self
                .updates
                .base_url
                .as_deref()
                .and_then(|value| url::Url::parse(value).ok());
            if !base.as_ref().is_some_and(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            }) {
                return Err(Error::Field("updates.base_url requires HTTPS".into()));
            }
        }
        let formats = match platform {
            Platform::Macos => &self.macos.formats,
            Platform::Windows => &self.windows.formats,
            Platform::Linux => &self.linux.formats,
        };
        let formats = if formats.is_empty() {
            vec![match (platform, legacy) {
                (Platform::Macos, true) => Format::Zip,
                (Platform::Macos, false) => Format::Dmg,
                (Platform::Windows, _) => Format::Nsis,
                (Platform::Linux, true) => Format::Tar,
                (Platform::Linux, false) => Format::Appimage,
            }]
        } else {
            formats.clone()
        };
        let mut packages = Vec::new();
        for format in formats {
            if !format.supports(platform) {
                return Err(Error::Format { format, platform });
            }
            if packages.iter().any(|p: &PackagePlan| p.format == format) {
                return Err(Error::Duplicate(format!("{format:?}")));
            }
            let owner = if !self.updates.enabled {
                UpdateOwner::Disabled
            } else {
                format.update_owner(platform)
            };
            let key = match owner {
                UpdateOwner::Sparkle => Some("sparkle"),
                UpdateOwner::Appimage => Some("appimage"),
                UpdateOwner::Velopack => Some("velopack"),
                _ => None,
            };
            if let Some(key) = key
                && !self.updates.public_keys.contains_key(key)
            {
                return Err(Error::Field(format!("updates.public_keys.{key}")));
            }
            if format == Format::Msix
                && self
                    .windows
                    .msix
                    .publisher
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(Error::Field("windows.msix.publisher".into()));
            }
            if platform == Platform::Linux {
                let name = self.linux.package_name.as_deref().unwrap_or(package);
                if name.len() < 2
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+.-".contains(&byte)
                    })
                    || !name.as_bytes()[0].is_ascii_alphanumeric()
                {
                    return Err(Error::Field("linux.package_name".into()));
                }
                for dependencies in self.linux.dependencies.values().chain(
                    self.runtimes
                        .iter()
                        .flat_map(|runtime| runtime.system_dependencies.values()),
                ) {
                    for dependency in dependencies {
                        if dependency.trim().is_empty() || dependency.chars().any(char::is_control)
                        {
                            return Err(Error::Field("Linux dependency expression".into()));
                        }
                    }
                }
                for runtime in self.runtimes.iter().filter(|runtime| runtime.system) {
                    if format == Format::Appimage
                        || runtime
                            .system_dependencies
                            .get(&format)
                            .is_none_or(Vec::is_empty)
                    {
                        return Err(Error::Field(format!(
                            "system runtime {} needs dependencies for {format:?}; AppImage runtimes must be bundled",
                            runtime.name
                        )));
                    }
                }
                if let Some(baseline) = &self.linux.baseline {
                    if baseline.name.trim().is_empty() {
                        return Err(Error::Field("linux.baseline.name".into()));
                    }
                    for version in [baseline.glibc.as_deref(), baseline.kernel.as_deref()]
                        .into_iter()
                        .flatten()
                    {
                        if version.split('.').any(|part| part.parse::<u32>().is_err()) {
                            return Err(Error::Field("Linux baseline version".into()));
                        }
                    }
                }
            }
            packages.push(PackagePlan {
                format,
                update_owner: owner,
                version: version::convert(
                    &parsed_version,
                    if format == Format::Zip && platform == Platform::Macos {
                        Format::App
                    } else {
                        format
                    },
                    self.package_version.as_deref(),
                    self.build_number.as_deref(),
                )?,
                config: self.clone(),
            });
        }
        Ok(Plan {
            name,
            identifier,
            version: version.into(),
            platform,
            legacy,
            config: self.clone(),
            packages,
        })
    }
}

/// Converts a generated/native relative path to a validated portable spelling.
/// # Errors
/// Rejects non-UTF8 paths, drive prefixes, absolute paths and traversal.
pub fn portable_relative_path(path: &std::path::Path) -> Result<PathBuf, Error> {
    let text = path.to_str().ok_or_else(|| Error::Path(path.into()))?;
    let portable = PathBuf::from(text.replace('\\', "/"));
    validate_relative_path(&portable)?;
    Ok(portable)
}

impl Config {
    fn normalize_paths(&mut self) -> Result<(), Error> {
        for path in [
            self.icon.as_mut(),
            self.assets.as_mut(),
            self.macos.info_plist.as_mut(),
            self.macos.entitlements.as_mut(),
            self.windows.manifest.as_mut(),
            self.windows.msix.manifest.as_mut(),
            self.linux.desktop_entry.as_mut(),
            self.linux.appstream.as_mut(),
            self.linux.flatpak.manifest.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            *path = portable_relative_path(path)?;
        }
        for resource in &mut self.resources {
            resource.from = portable_relative_path(&resource.from)?;
            resource.to = portable_relative_path(&resource.to)?;
        }
        for sidecar in &mut self.sidecars {
            sidecar.path = portable_relative_path(&sidecar.path)?;
        }
        for runtime in &mut self.runtimes {
            if !runtime.system {
                runtime.source = portable_relative_path(&runtime.source)?;
            }
        }
        for path in self.updates.public_keys.values_mut() {
            *path = portable_relative_path(path)?;
        }
        for component in &mut self.components {
            if let ComponentSource::Local { path } = &mut component.source {
                *path = portable_relative_path(path)?;
            }
            for path in &mut component.licenses {
                *path = portable_relative_path(path)?;
            }
        }
        Ok(())
    }
}

/// Validates a portable relative path without accessing the filesystem.
/// # Errors
/// Rejects absolute paths, Windows prefixes and parent traversal on every host.
pub fn validate_relative_path(path: &std::path::Path) -> Result<(), Error> {
    let text = path.to_str().ok_or_else(|| Error::Path(path.into()))?;
    if text.is_empty()
        || text.contains(['\\', ':'])
        || text.chars().any(char::is_control)
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::Path(path.into()));
    }
    Ok(())
}

fn component_name(name: &str) -> Result<(), Error> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(Error::Field(format!("component/channel name: {name}")));
    }
    Ok(())
}
