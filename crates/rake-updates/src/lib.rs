//! Native update providers with embedded authority and explicit restart handoff.
#[cfg(feature = "appimage")]
mod appimage;
#[cfg(feature = "appimage")]
pub use appimage::{ImageUpdate, StagedImage};
mod service;
pub use rake_dist_core::UpdateBinding;
pub use service::{Capability, DownloadProgress, RestartHandoff, UpdateService};
#[cfg(any(feature = "appimage", feature = "velopack"))]
mod signatures;
#[cfg(any(feature = "appimage", feature = "velopack"))]
mod transport;
#[cfg(feature = "velopack")]
mod velopack;
#[cfg(feature = "velopack")]
pub use velopack::{StagedVelopack, VelopackUpdate};

/// Native update failures; a failed check or declined restart never implies installation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Invalid embedded/installed authority or provider metadata.
    #[error("update policy: {0}")]
    Policy(String),
    /// Malformed serialized policy/feed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Native filesystem/process failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Provider download, verification or installation failure.
    #[error("update provider: {0}")]
    Provider(String),
    /// The application's quit guards declined this restart.
    #[error("update restart was declined")]
    Declined,
    /// The user cancelled an update operation.
    #[error("update operation cancelled")]
    Cancelled,
}

/// Runs installer callbacks before app construction, only for a compiled Velopack package.
/// # Errors
/// Rejects invalid embedded policy. An absent policy means an unbundled application.
pub fn early_startup_for(embedded: Option<&str>) -> Result<(), Error> {
    if let Some(embedded) = embedded {
        let binding: UpdateBinding = serde_json::from_str(embedded)?;
        binding
            .validate()
            .map_err(|error| Error::Policy(error.to_string()))?;
        #[cfg(all(windows, feature = "velopack"))]
        if binding.format == rake_dist_core::Format::Velopack {
            ::velopack::VelopackApp::build()
                .set_auto_apply_on_startup(false)
                .run();
        }
    }
    Ok(())
}

/// Captures build-script policy in the application's crate before SDK startup.
#[macro_export]
macro_rules! early_startup {
    () => {
        $crate::early_startup_for(option_env!("RAKE_UPDATE_POLICY"))
    };
}

/// Captures policy and resource layout from the consuming application crate.
#[macro_export]
macro_rules! updates {
    () => {{
        (|| -> Result<$crate::UpdateService, $crate::Error> {
            let policy = option_env!("RAKE_UPDATE_POLICY").ok_or_else(|| {
                $crate::Error::Policy("call rake_build::build() in build.rs".into())
            })?;
            let resources = $crate::__resources::resources!()
                .map_err(|error| $crate::Error::Policy(error.to_string()))?;
            $crate::UpdateService::new(policy, &resources)
        })()
    }};
}
#[doc(hidden)]
pub use rake_resources as __resources;
