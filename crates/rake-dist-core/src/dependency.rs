//! Grouped local inputs lowered to the established component and runtime contracts.
use crate::{Component, ComponentSource, Config, Error, Platform, Resource, Runtime, Sidecar};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// One prepared dependency tree, with helpers, libraries, resources and license notices.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BundledDependency {
    /// Package-relative root; `{target}` expands to the selected Rust target.
    pub path: PathBuf,
    /// Stable helper names mapped to root-relative executables; `{exe}` is `.exe` on Windows.
    #[serde(default)]
    pub executables: BTreeMap<String, PathBuf>,
    /// Installed resource destinations mapped to root-relative inputs (files or directories).
    #[serde(default)]
    pub resources: BTreeMap<PathBuf, PathBuf>,
    /// Root-relative license/notice paths, installed under `rake/licenses/<dependency>/`.
    #[serde(default)]
    pub licenses: Vec<PathBuf>,
    /// Optional private native library or framework deployment.
    #[serde(default)]
    pub libraries: Option<BundledLibraries>,
}

/// Library deployment using an existing directory, framework or GStreamer recipe.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BundledLibraries {
    /// Root-relative directory or `.framework` path.
    pub path: PathBuf,
    /// Deployment recipe; defaults to `directory`.
    #[serde(default = "directory")]
    pub recipe: String,
    /// Explicit plugin selection, for recipes supporting plugins.
    #[serde(default)]
    pub plugins: Vec<String>,
    /// Also link these libraries into the application process.
    #[serde(default)]
    pub link_from_app: bool,
}
fn directory() -> String {
    "directory".into()
}

impl Config {
    pub(crate) fn expand_dependencies(
        &mut self,
        platform: Platform,
        target: Option<&str>,
    ) -> Result<(), Error> {
        if self.dependencies.is_empty() {
            return Ok(());
        }
        let target = target.ok_or_else(|| {
            Error::Field("bundled dependencies require an explicit Rust target triple".into())
        })?;
        let expand = |path: &std::path::Path| -> Result<PathBuf, Error> {
            let text = path.to_str().ok_or_else(|| Error::Path(path.into()))?;
            let text = text.replace("{target}", target).replace(
                "{exe}",
                if platform == Platform::Windows {
                    ".exe"
                } else {
                    ""
                },
            );
            if text.contains(['{', '}']) {
                return Err(Error::Field(
                    "unknown bundled dependency placeholder".into(),
                ));
            }
            crate::portable_relative_path(std::path::Path::new(&text))
        };
        for (name, dependency) in std::mem::take(&mut self.dependencies) {
            crate::component_name(&name)?;
            if self
                .components
                .iter()
                .any(|component| component.name == name)
            {
                return Err(Error::Duplicate(name));
            }
            let root = expand(&dependency.path)?;
            let licenses = dependency
                .licenses
                .iter()
                .map(|path| expand(path))
                .collect::<Result<_, _>>()?;
            self.components.push(Component {
                name: name.clone(),
                target: target.into(),
                source: ComponentSource::Local { path: root.clone() },
                licenses,
            });
            for (helper, path) in dependency.executables {
                self.sidecars.push(Sidecar {
                    name: helper,
                    path: expand(&path)?,
                    component: Some(name.clone()),
                });
            }
            for (to, from) in dependency.resources {
                self.resources.push(Resource {
                    from: root.join(expand(&from)?),
                    to: expand(&to)?,
                });
            }
            if let Some(libraries) = dependency.libraries {
                self.runtimes.push(Runtime {
                    name: format!("{name}-runtime"),
                    recipe: libraries.recipe,
                    source: expand(&libraries.path)?,
                    system: false,
                    system_dependencies: BTreeMap::new(),
                    link_from_app: libraries.link_from_app,
                    component: Some(name),
                    plugins: libraries.plugins,
                    options: BTreeMap::new(),
                });
            }
        }
        Ok(())
    }
}
