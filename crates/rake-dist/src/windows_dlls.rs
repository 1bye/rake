//! Each executable gets a private loader directory scoped to declared runtimes.
use crate::{Error, files};
use rake_dist_core::{Config, Installation};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
pub(crate) fn prepare(
    config: &Config,
    descriptor: &Installation,
    resource_stage: &Path,
    executable: &Path,
    resources: &mut HashMap<String, String>,
) -> Result<(), Error> {
    let roots = |component: Option<&str>, main: bool| -> Result<Vec<PathBuf>, Error> {
        config
            .runtimes
            .iter()
            .filter(|runtime| {
                !runtime.system
                    && (runtime.link_from_app
                        || (!main && runtime.component.as_deref() == component))
            })
            .map(|runtime| {
                let relative = descriptor
                    .runtimes
                    .get(&runtime.name)
                    .ok_or_else(|| Error::Execution("Windows runtime descriptor missing".into()))?;
                files::confined(resource_stage, relative)
            })
            .collect()
    };
    deploy(
        executable,
        &roots(None, true)?,
        resource_stage,
        resource_stage,
        &config.windows.system_dlls,
        resources,
    )?;
    for helper in &config.sidecars {
        let relative = descriptor
            .helpers
            .get(&helper.name)
            .ok_or_else(|| Error::Execution("private helper descriptor missing".into()))?;
        let executable = files::confined(resource_stage, relative)?;
        deploy(
            &executable,
            &roots(helper.component.as_deref(), false)?,
            executable
                .parent()
                .ok_or_else(|| Error::Execution("helper parent missing".into()))?,
            resource_stage,
            &config.windows.system_dlls,
            resources,
        )?;
    }
    // Scanners and other runtime-owned subprocesses need their own loader scope.
    for runtime in config.runtimes.iter().filter(|runtime| !runtime.system) {
        let root = files::confined(
            resource_stage,
            descriptor
                .runtimes
                .get(&runtime.name)
                .ok_or_else(|| Error::Execution("runtime missing".into()))?,
        )?;
        for executable in executables(&root)? {
            deploy(
                &executable,
                &roots(runtime.component.as_deref(), false)?,
                executable
                    .parent()
                    .ok_or_else(|| Error::Execution("runtime executable parent missing".into()))?,
                resource_stage,
                &config.windows.system_dlls,
                resources,
            )?;
        }
    }
    Ok(())
}
fn deploy(
    executable: &Path,
    roots: &[PathBuf],
    destination: &Path,
    resource_stage: &Path,
    system_dlls: &[String],
    resources: &mut HashMap<String, String>,
) -> Result<(), Error> {
    let paths = rake_dist_windows::deploy_dlls(&rake_dist_windows::DllDeployment {
        executable,
        roots,
        destination,
        system_dlls,
    })
    .map_err(|error| Error::Execution(error.to_string()))?;
    for path in paths {
        let relative = path
            .strip_prefix(resource_stage)
            .map_err(|_| Error::Execution("DLL deployment escaped stage".into()))?;
        let relative = rake_dist_core::portable_relative_path(relative)?;
        let covered = resources.iter().any(|(source, dest)| {
            Path::new(source).is_dir() && relative.starts_with(Path::new(&dest.replace('\\', "/")))
        });
        if !covered {
            resources.insert(
                path.to_string_lossy().into_owned(),
                relative.to_string_lossy().into_owned(),
            );
        }
    }
    Ok(())
}
fn executables(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut output = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            output.extend(executables(&path)?);
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
        {
            output.push(path);
        }
    }
    Ok(output)
}
