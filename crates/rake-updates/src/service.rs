//! Provider activation requires both embedded policy and matching installation metadata.
use crate::Error;
use rake_dist_core::{Installation, UpdateBinding, UpdateOwner};
use rake_resources::Resources;

/// Bytes transferred by a bounded update download.
#[derive(Clone, Copy, Debug)]
pub struct DownloadProgress {
    /// Verified upper bound from the signed feed.
    pub total_bytes: u64,
    /// Bytes received so far; completion still requires hash verification.
    pub received_bytes: u64,
}

/// What the installed distribution channel permits the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// Update support was disabled at build time.
    Disabled,
    /// An operating-system/package manager owns installation.
    Managed(UpdateOwner),
    /// The application can check and stage updates; restart still requires approval.
    SelfManaged,
    /// The configured SDK was not enabled in this application build.
    ProviderUnavailable(UpdateOwner),
    /// Installation is external and requires an explicit user action.
    External,
}

/// Owns embedded verification policy and the installation it authorizes.
#[derive(Debug)]
pub struct UpdateService {
    pub(crate) binding: UpdateBinding,
}
impl UpdateService {
    /// Creates a service from policy captured in the application crate and installed resources.
    /// # Errors
    /// Rejects missing/invalid policy and metadata that changes identity, target,
    /// version, packaging format, owner, channel or publication root.
    pub fn new(embedded: &str, resources: &Resources) -> Result<Self, Error> {
        let binding: UpdateBinding = serde_json::from_str(embedded)?;
        binding
            .validate()
            .map_err(|error| Error::Policy(error.to_string()))?;
        matches_installation(&binding, resources.installation())?;
        Ok(Self { binding })
    }
    /// Returns the application identity and immutable verification policy.
    pub fn binding(&self) -> &UpdateBinding {
        &self.binding
    }
    /// Reports capabilities without downloading or mutating the installation.
    pub fn capability(&self) -> Capability {
        match self.binding.owner {
            UpdateOwner::Disabled => Capability::Disabled,
            UpdateOwner::AppInstaller
            | UpdateOwner::Apt
            | UpdateOwner::RpmRepository
            | UpdateOwner::Pacman
            | UpdateOwner::Flatpak => Capability::Managed(self.binding.owner),
            UpdateOwner::External => Capability::External,
            UpdateOwner::Appimage if cfg!(all(target_os = "linux", feature = "appimage")) => {
                Capability::SelfManaged
            }
            UpdateOwner::Velopack if cfg!(all(windows, feature = "velopack")) => {
                Capability::SelfManaged
            }
            other => Capability::ProviderUnavailable(other),
        }
    }
}
fn matches_installation(binding: &UpdateBinding, installed: &Installation) -> Result<(), Error> {
    if binding.identifier != installed.identifier
        || binding.version != installed.version
        || binding.package_version != installed.package_version
        || binding.target != installed.target
        || binding.format != installed.format
        || binding.owner != installed.update_owner
        || binding.channel != installed.updates.channel.as_deref().unwrap_or("stable")
        || binding.base_url != installed.updates.base_url
    {
        return Err(Error::Policy(
            "installed metadata conflicts with the compiled update policy".into(),
        ));
    }
    Ok(())
}

/// A pending handoff owned by the application lifecycle; dropping it never installs.
pub struct RestartHandoff {
    action: Option<Box<dyn FnOnce() -> Result<(), Error>>>,
}
impl std::fmt::Debug for RestartHandoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RestartHandoff")
            .field("pending", &self.action.is_some())
            .finish()
    }
}
impl RestartHandoff {
    /// Owns a provider/application-defined install callback until explicit approval.
    /// Callbacks must not install anything before this handoff is committed.
    pub fn new(action: impl FnOnce() -> Result<(), Error> + 'static) -> Self {
        Self {
            action: Some(Box::new(action)),
        }
    }
    /// Runs the staged provider handoff after the application's guards approve.
    ///
    /// Call from the host's committed-quit boundary, before exiting. `false`
    /// cancels this attempt without installing or replacing files. The application
    /// owns saving documents, waiting/cancelling work and stopping its services.
    /// # Errors
    /// Returns guard decline or the native provider's install/restart failure.
    pub fn commit(mut self, approved: bool) -> Result<(), Error> {
        if !approved {
            return Err(Error::Declined);
        }
        self.action
            .take()
            .ok_or_else(|| Error::Policy("handoff already consumed".into()))?()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    #[test]
    fn decline_and_drop_never_invoke_the_installer() {
        let invoked = Rc::new(Cell::new(false));
        let observed = Rc::clone(&invoked);
        let handoff = RestartHandoff::new(move || {
            observed.set(true);
            Ok(())
        });
        assert!(matches!(handoff.commit(false), Err(Error::Declined)));
        assert!(!invoked.get());
        let observed = Rc::clone(&invoked);
        drop(RestartHandoff::new(move || {
            observed.set(true);
            Ok(())
        }));
        assert!(!invoked.get());
        let observed = Rc::clone(&invoked);
        RestartHandoff::new(move || {
            observed.set(true);
            Ok(())
        })
        .commit(true)
        .unwrap();
        assert!(invoked.get());
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use rake_dist_core::{Format, Platform, Updates};
    #[test]
    fn managed_ownership_cannot_be_changed_by_an_installed_descriptor() {
        let binding = UpdateBinding {
            schema_version: 1,
            identifier: "dev.rake.app".into(),
            version: "1.0.0".into(),
            package_version: "1.0.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            format: Format::Deb,
            owner: UpdateOwner::Apt,
            channel: "stable".into(),
            base_url: Some("https://updates.example.org".into()),
            public_keys: Default::default(),
        };
        binding.validate().unwrap();
        let mut installed = Installation {
            schema_version: 1,
            identifier: binding.identifier.clone(),
            version: binding.version.clone(),
            package_version: "1.0.0".into(),
            target: binding.target.clone(),
            platform: Platform::Linux,
            format: Format::Deb,
            update_owner: UpdateOwner::Apt,
            updates: Updates {
                enabled: true,
                base_url: binding.base_url.clone(),
                ..Default::default()
            },
            helpers: Default::default(),
            runtimes: Default::default(),
            frameworks: Default::default(),
            native_groups: Default::default(),
        };
        matches_installation(&binding, &installed).unwrap();
        installed.format = Format::Appimage;
        installed.update_owner = UpdateOwner::Appimage;
        assert!(matches_installation(&binding, &installed).is_err());
        let mut forged = binding;
        forged.owner = UpdateOwner::Appimage;
        assert!(forged.validate().is_err());
    }
}
