//! The optional native SDK is an exact, locked component prepared before compilation.
use crate::{Error, components};
use rake_dist_core::{Archive, Component, ComponentSource, PackagePlan, UpdateOwner};
use std::path::{Path, PathBuf};
pub(crate) fn prepare(
    item: &PackagePlan,
    target: &str,
    package: &Path,
    stage: &Path,
    cache: &Path,
    offline: bool,
    prepared: &mut components::Prepared,
) -> Result<Option<PathBuf>, Error> {
    if item.update_owner() != UpdateOwner::Sparkle {
        return Ok(None);
    }
    if prepared.roots.contains_key("rake-sparkle-sdk")
        || item
            .config()
            .runtimes
            .iter()
            .any(|runtime| runtime.name == "sparkle")
    {
        return Err(Error::Execution(
            "sparkle/rake-sparkle-sdk are reserved for the pinned updater SDK".into(),
        ));
    }
    let component=Component{name:"rake-sparkle-sdk".into(),target:target.into(),source:ComponentSource::Https{
        url:"https://github.com/sparkle-project/Sparkle/releases/download/2.10.1/Sparkle-2.10.1.tar.xz".into(),
        sha256:"1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27".into(),archive:Archive::TarXz,max_download_bytes:32*1024*1024
    },licenses:vec![PathBuf::from("LICENSE")]};
    let sdk = components::resolve(
        &[component],
        target,
        package,
        &stage.join("automatic-components"),
        cache,
        offline,
    )?;
    let root = sdk
        .roots
        .get("rake-sparkle-sdk")
        .ok_or_else(|| Error::Execution("prepared Sparkle SDK missing".into()))?;
    let framework = root.join("Sparkle.framework");
    components::architecture(&framework.join("Sparkle"), target)?;
    prepared.roots.extend(sdk.roots);
    prepared.locks.extend(sdk.locks);
    Ok(Some(framework))
}

pub(crate) struct BuildContext<'a> {
    pub distribution_target: &'a str,
    pub sparkle_framework: Option<&'a Path>,
}
