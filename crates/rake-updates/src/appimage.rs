//! Signed full-image replacement uses a verified sibling and one atomic rename.
use crate::transport::{digest, download, get};
use crate::{DownloadProgress, Error, RestartHandoff, UpdateService};

use fs2::FileExt;
use rake_dist_core::{SignedUpdateFeed, UpdateBinding, UpdateFeed, UpdateOwner, UpdateRelease};

use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_FEED: u64 = 1024 * 1024;
const MAX_IMAGE: u64 = 4 * 1024 * 1024 * 1024;

/// Authenticated update selection; only a verified feed can construct this value.
#[derive(Debug)]
pub struct ImageUpdate {
    release: UpdateRelease,
    current: UpdateRelease,
    feed: UpdateFeed,
}
impl ImageUpdate {
    /// Returns authenticated release metadata for the application's update UI.
    pub fn release(&self) -> &UpdateRelease {
        &self.release
    }
}

/// Owns a verified same-filesystem image and an exclusive update lock.
#[derive(Debug)]
pub struct StagedImage {
    staged: tempfile::NamedTempFile,
    current: PathBuf,
    old_hash: String,
    new_hash: String,
    backup: PathBuf,
    _lock: fs::File,
}
impl StagedImage {
    /// Defers replacement and restart until the application's committed quit boundary.
    /// Dropping the returned handoff removes staging and leaves the current image intact.
    pub fn restart_handoff(self, args: Vec<OsString>) -> RestartHandoff {
        RestartHandoff::new(move || self.replace_and_restart(&args))
    }
    fn replace_and_restart(self, args: &[OsString]) -> Result<(), Error> {
        if digest(&self.current)? != self.old_hash || digest(self.staged.path())? != self.new_hash {
            return Err(Error::Provider(
                "application image changed during update preparation".into(),
            ));
        }
        fs::hard_link(&self.current, &self.backup)?;
        let staged = self.staged.into_temp_path();
        fs::rename(&staged, &self.current)?;
        sync_parent(&self.current)?;
        let mut command = std::process::Command::new(&self.current);
        command.args(args).current_dir(std::env::temp_dir());
        for name in [
            "APPIMAGE",
            "APPDIR",
            "ARGV0",
            "LD_LIBRARY_PATH",
            "LD_PRELOAD",
        ] {
            command.env_remove(name);
        }
        if let Err(error) = command.spawn() {
            fs::rename(&self.backup, &self.current)?;
            sync_parent(&self.current)?;
            return Err(error.into());
        }
        Ok(())
    }
}

impl UpdateService {
    /// Checks a bounded signed feed without changing any installed files.
    ///
    /// This is blocking I/O: run on an application-owned background worker.
    /// # Errors
    /// Rejects package-manager ownership, invalid/expired signatures, identity or
    /// channel conflicts, downgrades, unsupported feeds and transfer failures.
    pub fn check_appimage(&self, cancelled: &AtomicBool) -> Result<Option<ImageUpdate>, Error> {
        require_owner(&self.binding)?;
        let base = self
            .binding
            .base_url
            .as_deref()
            .ok_or_else(|| Error::Policy("update URL missing".into()))?;
        let url = rake_dist_core::feed_url(
            base,
            &self.binding.channel,
            &self.binding.target,
            "appimage.json",
        )
        .map_err(|error| Error::Policy(error.to_string()))?;
        let bytes = get(&url, MAX_FEED, cancelled)?;
        select(&self.binding, &bytes, now()?)
    }

    /// Downloads and verifies a complete sibling image under an exclusive update lock.
    ///
    /// The current `APPIMAGE` path is only a candidate: its complete bytes must
    /// match the signed feed's installed-version record before any replacement.
    /// Environment variables cannot grant package-manager apps update authority.
    /// # Errors
    /// Returns cancellation, ownership/signature failures, competing updater,
    /// read-only filesystem, changed installation or bounded transfer errors.
    pub fn stage_appimage(
        &self,
        update: ImageUpdate,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(DownloadProgress),
    ) -> Result<StagedImage, Error> {
        require_owner(&self.binding)?;
        validate_feed(&self.binding, &update.feed, now()?)?;
        let current = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .ok_or_else(|| {
                Error::Policy("AppImage runtime did not provide its image path".into())
            })?;
        let metadata = fs::symlink_metadata(&current)?;
        if !metadata.is_file() || metadata.permissions().readonly() {
            return Err(Error::Policy(
                "AppImage must be a writable regular image".into(),
            ));
        }
        let current = fs::canonicalize(current)?;
        let parent = current
            .parent()
            .ok_or_else(|| Error::Policy("AppImage parent".into()))?;
        let name = current
            .file_name()
            .ok_or_else(|| Error::Policy("AppImage filename".into()))?
            .to_string_lossy();
        let lock_path = parent.join(format!(".{name}.rake-update.lock"));
        if lock_path.is_symlink() {
            return Err(Error::Policy(
                "AppImage update lock cannot be a symlink".into(),
            ));
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::Provider("another process owns this AppImage update".into()))?;
        verify_image(&current, &update.current)?;
        let backup = parent.join(format!(".{name}.rake-previous"));
        recover_backup(&backup, &update.feed, &update.current)?;
        let mut staged = tempfile::Builder::new()
            .prefix(".rake-image-")
            .tempfile_in(parent)?;
        download(
            &update.release.url,
            update.release.size,
            &mut staged,
            cancelled,
            &mut progress,
        )?;
        staged.flush()?;
        staged.as_file().sync_all()?;
        verify_image(staged.path(), &update.release)?;
        fs::set_permissions(staged.path(), metadata.permissions())?;
        staged.as_file().sync_all()?;
        Ok(StagedImage {
            staged,
            current,
            old_hash: update.current.sha256,
            new_hash: update.release.sha256,
            backup,
            _lock: lock,
        })
    }
}
fn require_owner(binding: &UpdateBinding) -> Result<(), Error> {
    if binding.owner != UpdateOwner::Appimage {
        return Err(Error::Policy(
            "this compiled distribution does not own AppImage replacement".into(),
        ));
    }
    #[cfg(not(target_os = "linux"))]
    return Err(Error::Policy("AppImage updates require Linux".into()));
    #[cfg(target_os = "linux")]
    Ok(())
}
fn now() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|error| Error::Policy(error.to_string()))
}
fn select(binding: &UpdateBinding, bytes: &[u8], now: u64) -> Result<Option<ImageUpdate>, Error> {
    if bytes.len() as u64 > MAX_FEED {
        return Err(Error::Provider("feed exceeds size bound".into()));
    }
    let signed: SignedUpdateFeed = serde_json::from_slice(bytes)?;
    let key = binding
        .public_keys
        .get("appimage")
        .ok_or_else(|| Error::Policy("embedded AppImage public key missing".into()))?;
    crate::signatures::verify(key, signed.payload.as_bytes(), &signed.signature)?;
    let feed: UpdateFeed = serde_json::from_str(&signed.payload)?;
    validate_feed(binding, &feed, now)?;
    let installed = semver::Version::parse(&binding.package_version)
        .map_err(|error| Error::Policy(error.to_string()))?;
    let current = feed
        .releases
        .iter()
        .find(|release| {
            release.version == binding.version && release.package_version == binding.package_version
        })
        .ok_or_else(|| {
            Error::Provider(
                "signed feed must retain the installed image for ownership verification".into(),
            )
        })?
        .clone();
    let latest = feed
        .releases
        .iter()
        .filter_map(|release| {
            semver::Version::parse(&release.package_version)
                .ok()
                .map(|version| (version, release))
        })
        .filter(|(version, _)| *version > installed)
        .max_by(|(left, _), (right, _)| left.cmp(right));
    Ok(latest.map(|(_, release)| ImageUpdate {
        release: release.clone(),
        current,
        feed: feed.clone(),
    }))
}
fn validate_feed(binding: &UpdateBinding, feed: &UpdateFeed, now: u64) -> Result<(), Error> {
    if feed.schema_version != 1
        || feed.identifier != binding.identifier
        || feed.target != binding.target
        || feed.channel != binding.channel
        || feed.published_at > now.saturating_add(300)
        || feed.expires_at <= now
        || feed.expires_at <= feed.published_at
        || feed.releases.len() > 1000
    {
        return Err(Error::Provider(
            "feed identity, channel, expiry or revision is invalid".into(),
        ));
    }
    let mut versions = std::collections::BTreeSet::new();
    for release in &feed.releases {
        let version = semver::Version::parse(&release.package_version)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let url =
            url::Url::parse(&release.url).map_err(|error| Error::Provider(error.to_string()))?;
        if !versions.insert(version)
            || release.size < 11
            || release.size > MAX_IMAGE
            || release.sha256.len() != 64
            || !release.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.host_str().is_none()
        {
            return Err(Error::Provider("invalid or duplicate image release".into()));
        }
    }
    Ok(())
}
fn verify_image(path: &Path, release: &UpdateRelease) -> Result<(), Error> {
    let mut file = fs::File::open(path)?;
    let mut header = [0; 11];
    file.read_exact(&mut header)?;
    if header[..4] != *b"\x7fELF"
        || header[8..11] != *b"AI\x02"
        || file.metadata()?.len() != release.size
        || digest(path)? != release.sha256
    {
        return Err(Error::Provider(
            "image type, size or SHA-256 does not match its signed release".into(),
        ));
    }
    Ok(())
}
fn recover_backup(path: &Path, feed: &UpdateFeed, current: &UpdateRelease) -> Result<(), Error> {
    if !path.try_exists()? {
        return Ok(());
    }
    if path.is_symlink() || !path.is_file() {
        return Err(Error::Policy("unexpected AppImage recovery path".into()));
    }
    let hash = digest(path)?;
    let installed = semver::Version::parse(&current.package_version)
        .map_err(|error| Error::Policy(error.to_string()))?;
    if !feed.releases.iter().any(|release| {
        release.sha256 == hash
            && semver::Version::parse(&release.package_version)
                .is_ok_and(|version| version <= installed)
    }) {
        return Err(Error::Provider(
            "recovery image is not a trusted previous release; preserved for manual recovery"
                .into(),
        ));
    }
    fs::remove_file(path)?;
    sync_parent(path)
}
fn sync_parent(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    fs::File::open(
        path.parent()
            .ok_or_else(|| Error::Policy("image parent missing".into()))?,
    )?
    .sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rake_dist_core::Format;
    fn fixture() -> (UpdateBinding, UpdateFeed, SigningKey) {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let binding = UpdateBinding {
            schema_version: 1,
            identifier: "dev.rake.image".into(),
            version: "1.0.0".into(),
            package_version: "1.0.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            format: Format::Appimage,
            owner: UpdateOwner::Appimage,
            channel: "stable".into(),
            base_url: Some("https://example.org".into()),
            public_keys: std::collections::BTreeMap::from([(
                "appimage".into(),
                signing.verifying_key().to_bytes().to_vec(),
            )]),
        };
        let feed = UpdateFeed {
            schema_version: 1,
            identifier: binding.identifier.clone(),
            target: binding.target.clone(),
            channel: binding.channel.clone(),
            published_at: 100,
            expires_at: 200,
            releases: ["1.0.0", "1.0.1"]
                .map(|version| UpdateRelease {
                    version: version.into(),
                    package_version: version.into(),
                    url: format!("https://example.org/app-{version}.AppImage"),
                    sha256: "a".repeat(64),
                    size: 100,
                })
                .to_vec(),
        };
        (binding, feed, signing)
    }
    fn signed(feed: &UpdateFeed, key: &SigningKey) -> Vec<u8> {
        let payload = serde_json::to_string(feed).unwrap();
        let signature = hex::encode(key.sign(payload.as_bytes()).to_bytes());
        serde_json::to_vec(&SignedUpdateFeed { payload, signature }).unwrap()
    }
    #[test]
    fn signature_channel_expiry_and_installed_image_are_required() {
        let (binding, mut feed, key) = fixture();
        let selected = select(&binding, &signed(&feed, &key), 150)
            .unwrap()
            .unwrap();
        assert_eq!(selected.release.version, "1.0.1");
        assert!(
            select(
                &binding,
                &signed(&feed, &SigningKey::from_bytes(&[8; 32])),
                150
            )
            .is_err()
        );
        assert!(select(&binding, &signed(&feed, &key), 200).is_err());
        feed.channel = "beta".into();
        assert!(select(&binding, &signed(&feed, &key), 150).is_err());
        feed.channel = "stable".into();
        feed.releases.remove(0);
        assert!(select(&binding, &signed(&feed, &key), 150).is_err());
    }
    #[test]
    fn older_feed_cannot_downgrade_and_duplicate_versions_fail() {
        let (mut binding, mut feed, key) = fixture();
        binding.version = "1.0.1".into();
        binding.package_version = "1.0.1".into();
        assert!(
            select(&binding, &signed(&feed, &key), 150)
                .unwrap()
                .is_none()
        );
        feed.releases.push(feed.releases[0].clone());
        assert!(select(&binding, &signed(&feed, &key), 150).is_err());
    }
    #[test]
    fn decline_discards_staging_and_preserves_both_image_and_user_data() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("app.AppImage");
        fs::write(&current, b"old image").unwrap();
        let state = root.path().join("state");
        fs::write(&state, b"unsaved content").unwrap();
        let mut staged = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        staged.write_all(b"new image").unwrap();
        let stage_path = staged.path().to_owned();
        let image = StagedImage {
            old_hash: digest(&current).unwrap(),
            new_hash: digest(staged.path()).unwrap(),
            staged,
            current: current.clone(),
            backup: root.path().join("backup"),
            _lock: fs::File::create(root.path().join("lock")).unwrap(),
        };
        assert!(matches!(
            image.restart_handoff(Vec::new()).commit(false),
            Err(Error::Declined)
        ));
        assert_eq!(fs::read(&current).unwrap(), b"old image");
        assert_eq!(fs::read(state).unwrap(), b"unsaved content");
        assert!(!stage_path.exists());
    }
    #[test]
    fn recovery_preserves_unknown_backups_and_accepts_only_signed_history() {
        let (_, mut feed, _) = fixture();
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("previous");
        fs::write(&backup, b"previous image").unwrap();
        let current = feed.releases[1].clone();
        assert!(recover_backup(&backup, &feed, &current).is_err());
        assert!(backup.exists());
        feed.releases[0].sha256 = digest(&backup).unwrap();
        recover_backup(&backup, &feed, &current).unwrap();
        assert!(!backup.exists());
    }
}
