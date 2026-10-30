//! The pinned SDK retains native package installation; signed transport owns feed trust.
use crate::{DownloadProgress, Error, RestartHandoff, UpdateService, transport};
use ::velopack::{
    UpdateCheck, UpdateInfo, UpdateManager, UpdateOptions, VelopackAsset, VelopackAssetFeed,
    locator::{self, LocationContext},
    sources::UpdateSource,
};
use rake_dist_core::{NativeFeedSignature, UpdateBinding, UpdateOwner};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
};

/// A native SDK update selected only from an authenticated release feed.
pub struct VelopackUpdate {
    info: UpdateInfo,
}
impl std::fmt::Debug for VelopackUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VelopackUpdate")
            .field("version", &self.info.TargetFullRelease.Version)
            .finish()
    }
}
impl VelopackUpdate {
    /// Returns the SDK's native package version for update UI.
    pub fn version(&self) -> &str {
        &self.info.TargetFullRelease.Version
    }
}
/// A checksum-verified native SDK package awaiting guarded shutdown.
pub struct StagedVelopack {
    manager: UpdateManager,
    info: UpdateInfo,
    path: PathBuf,
}
impl std::fmt::Debug for StagedVelopack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StagedVelopack")
            .field("path", &self.path)
            .finish()
    }
}
impl StagedVelopack {
    /// Schedules the SDK to wait for host exit, apply and restart after guard approval.
    pub fn restart_handoff(self, args: Vec<OsString>) -> RestartHandoff {
        RestartHandoff::new(move || {
            verify_package(&self.path, &self.info.TargetFullRelease)?;
            self.manager
                .wait_exit_then_apply_updates(&self.info, false, true, args)
                .map_err(provider)
        })
    }
}
impl UpdateService {
    /// Checks the signed native Velopack feed through the pinned SDK.
    /// Blocking I/O must run on an application-owned worker.
    /// # Errors
    /// Rejects other package owners, uninstalled/mismatched apps, unsigned feeds,
    /// invalid package names, downgrades and native SDK/transport failures.
    pub fn check_velopack(
        &self,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<VelopackUpdate>, Error> {
        let manager = self.velopack_manager(cancelled, Arc::new(|_| {}))?;
        match manager.check_for_updates().map_err(provider)? {
            UpdateCheck::UpdateAvailable(info) => {
                if info.IsDowngrade {
                    return Err(Error::Policy("Velopack downgrade rejected".into()));
                }
                Ok(Some(VelopackUpdate { info: *info }))
            }
            UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty => Ok(None),
        }
    }
    /// Downloads the full SDK package with bounded transfer, progress and cancellation.
    /// # Errors
    /// Returns transport/SDK failures and rejects corrupted cached packages too.
    pub fn stage_velopack(
        &self,
        update: VelopackUpdate,
        cancelled: Arc<AtomicBool>,
        progress: impl Fn(DownloadProgress) + Send + Sync + 'static,
    ) -> Result<StagedVelopack, Error> {
        validate_asset(&self.binding, &update.info.TargetFullRelease)?;
        let manager = self.velopack_manager(Arc::clone(&cancelled), Arc::new(progress))?;
        manager
            .download_updates(&update.info, None)
            .map_err(provider)?;
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let location =
            locator::auto_locate_app_manifest(LocationContext::FromCurrentExe).map_err(provider)?;
        let path = location
            .get_packages_dir()
            .join(&update.info.TargetFullRelease.FileName);
        verify_package(&path, &update.info.TargetFullRelease)?;
        Ok(StagedVelopack {
            manager,
            info: update.info,
            path,
        })
    }
    fn velopack_manager(
        &self,
        cancelled: Arc<AtomicBool>,
        progress: Arc<dyn Fn(DownloadProgress) + Send + Sync>,
    ) -> Result<UpdateManager, Error> {
        if self.binding.owner != UpdateOwner::Velopack || !cfg!(windows) {
            return Err(Error::Policy(
                "this distribution does not own Windows Velopack installation".into(),
            ));
        }
        let options = UpdateOptions {
            AllowVersionDowngrade: false,
            ExplicitChannel: Some(self.binding.channel.clone()),
            MaximumDeltasBeforeFallback: -1,
        };
        let manager = UpdateManager::new(
            SignedSource {
                agent: self.http_agent(),
                binding: self.binding.clone(),
                cancelled,
                progress,
            },
            Some(options),
            None,
        )
        .map_err(provider)?;
        if manager.get_app_id() != self.binding.identifier
            || manager.get_current_version_as_string() != self.binding.package_version
            || manager.get_is_portable()
        {
            return Err(Error::Policy(
                "Velopack installation conflicts with embedded identity/version".into(),
            ));
        }
        Ok(manager)
    }
}
struct SignedSource {
    agent: ureq::Agent,
    binding: UpdateBinding,
    cancelled: Arc<AtomicBool>,
    progress: Arc<dyn Fn(DownloadProgress) + Send + Sync>,
}
impl SignedSource {
    fn url(&self, file: &str) -> Result<String, Error> {
        rake_dist_core::feed_url(
            self.binding
                .base_url
                .as_deref()
                .ok_or_else(|| Error::Policy("Velopack feed URL missing".into()))?,
            &self.binding.channel,
            &self.binding.target,
            file,
        )
        .map_err(|error| Error::Policy(error.to_string()))
    }
}
impl UpdateSource for SignedSource {
    fn get_release_feed(
        &self,
        channel: &str,
        _app: &::velopack::bundle::Manifest,
        _staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, ::velopack::Error> {
        let operation = || -> Result<VelopackAssetFeed, Error> {
            if channel != self.binding.channel {
                return Err(Error::Policy(
                    "SDK channel conflicts with compiled policy".into(),
                ));
            }
            let url = self.url(&format!("releases.{channel}.json"))?;
            let bytes = transport::get(&self.agent, &url, 1024 * 1024, &self.cancelled)?;
            let signature =
                transport::get(&self.agent, &format!("{url}.sig"), 1024, &self.cancelled)?;
            let key = self
                .binding
                .public_keys
                .get("velopack")
                .ok_or_else(|| Error::Policy("embedded Velopack public key missing".into()))?;
            let signature: NativeFeedSignature = serde_json::from_slice(&signature)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| Error::Policy(error.to_string()))?
                .as_secs();
            if signature.schema_version != 1
                || signature.expires_at <= now
                || signature.published_at > now.saturating_add(300)
                || signature.expires_at <= signature.published_at
            {
                return Err(Error::Provider(
                    "native feed signature expired or invalid".into(),
                ));
            }
            let message = signature.message(
                "velopack",
                &self.binding.identifier,
                &self.binding.target,
                &self.binding.channel,
                &bytes,
            );
            crate::signatures::verify(key, &message, &signature.signature)?;
            let mut feed: VelopackAssetFeed = serde_json::from_slice(&bytes)?;
            feed.Assets
                .retain(|asset| asset.Type.eq_ignore_ascii_case("full"));
            for asset in &feed.Assets {
                validate_asset(&self.binding, asset)?;
            }
            Ok(feed)
        };
        operation().map_err(|error| ::velopack::Error::Other(error.to_string()))
    }
    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        local_file: &Path,
        _progress: Option<Sender<i16>>,
    ) -> Result<(), ::velopack::Error> {
        let operation = || -> Result<(), Error> {
            validate_asset(&self.binding, asset)?;
            let url = self.url(&asset.FileName)?;
            let mut file = fs::File::create(local_file)?;
            transport::download(
                &self.agent,
                &url,
                asset.Size,
                &mut file,
                &self.cancelled,
                &mut |progress| (self.progress)(progress),
            )?;
            file.sync_all()?;
            Ok(())
        };
        operation().map_err(|error| ::velopack::Error::Other(error.to_string()))
    }
}
fn validate_asset(binding: &UpdateBinding, asset: &VelopackAsset) -> Result<(), Error> {
    if asset.PackageId != binding.identifier
        || !asset.Type.eq_ignore_ascii_case("full")
        || asset.Size == 0
        || asset.Size > 4 * 1024 * 1024 * 1024
        || asset.SHA256.len() != 64
        || !asset.SHA256.bytes().all(|value| value.is_ascii_hexdigit())
        || asset.FileName.is_empty()
        || !asset
            .FileName
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'-' | b'_'))
        || !asset.FileName.ends_with(".nupkg")
        || asset.FileName.starts_with('.')
    {
        return Err(Error::Policy(
            "invalid native Velopack asset identity/path/hash".into(),
        ));
    }
    let _version =
        semver::Version::parse(&asset.Version).map_err(|error| Error::Policy(error.to_string()))?;
    Ok(())
}
fn verify_package(path: &Path, asset: &VelopackAsset) -> Result<(), Error> {
    if path.is_symlink()
        || !path.is_file()
        || fs::metadata(path)?.len() != asset.Size
        || !transport::digest(path)?.eq_ignore_ascii_case(&asset.SHA256)
    {
        return Err(Error::Provider(
            "Velopack package size/SHA-256 verification failed".into(),
        ));
    }
    Ok(())
}
fn provider(error: ::velopack::Error) -> Error {
    Error::Provider(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_package_validation_covers_cached_corruption_and_escaping_names() {
        let binding = UpdateBinding {
            schema_version: 1,
            identifier: "dev.rake.test".into(),
            version: "1.0.0".into(),
            package_version: "1.0.0".into(),
            target: "x86_64-pc-windows-msvc".into(),
            format: rake_dist_core::Format::Velopack,
            owner: UpdateOwner::Velopack,
            channel: "stable".into(),
            base_url: Some("https://example.org".into()),
            public_keys: Default::default(),
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("release.nupkg");
        fs::write(&path, b"package").unwrap();
        let mut asset = VelopackAsset {
            PackageId: binding.identifier.clone(),
            Version: "1.0.1".into(),
            Type: "Full".into(),
            FileName: "release.nupkg".into(),
            SHA256: transport::digest(&path).unwrap(),
            Size: 7,
            ..Default::default()
        };
        validate_asset(&binding, &asset).unwrap();
        verify_package(&path, &asset).unwrap();
        fs::write(&path, b"changed").unwrap();
        assert!(verify_package(&path, &asset).is_err());
        asset.FileName = "../other.nupkg".into();
        assert!(validate_asset(&binding, &asset).is_err());
        asset.FileName = "release.nupkg".into();
        asset.PackageId = "dev.other.app".into();
        assert!(validate_asset(&binding, &asset).is_err());
    }
}
