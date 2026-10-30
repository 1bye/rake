//! Resolves installed resources and private helpers independently of the working directory.
use rake_dist_core::{Installation, validate_relative_path};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Resource/descriptor errors; absence is normal for an unbundled development app.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No installation descriptor accompanies this executable.
    #[error("application is not installed with a Rake descriptor")]
    Unbundled,
    /// A descriptor or resource could not be read.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Installed metadata is malformed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A path or descriptor version violates the installation contract.
    #[error("invalid installed resource: {0}")]
    Invalid(String),
}

/// Owns a validated installation descriptor and canonical resource root.
pub struct Resources {
    root: PathBuf,
    installation: Installation,
}

impl Resources {
    /// Finds resources adjacent to the current executable using the package's resource directory name.
    /// # Errors
    /// Returns [`Error::Unbundled`] or malformed/inaccessible metadata errors.
    pub fn discover(name: &str) -> Result<Self, Error> {
        Self::for_executable(&std::env::current_exe()?, name)
    }

    /// Resolves macOS, Windows and Linux layouts from a supplied executable path.
    /// # Errors
    /// Returns an error for invalid names, missing descriptors or malformed metadata.
    pub fn for_executable(executable: &Path, name: &str) -> Result<Self, Error> {
        validate_relative_path(Path::new(name)).map_err(|e| Error::Invalid(e.to_string()))?;
        if Path::new(name).components().count() != 1 {
            return Err(Error::Invalid("resource directory name".into()));
        }
        let executable = fs::canonicalize(executable)?;
        let parent = executable.parent().ok_or(Error::Unbundled)?;
        let mut roots = vec![parent.to_path_buf()];
        if let Some(prefix) = parent.parent() {
            roots.push(prefix.join("Resources"));
            roots.push(prefix.join("lib").join(name));
        }
        for root in roots {
            if root.join("rake-installation.json").is_file() {
                return Self::load(&root);
            }
        }
        Err(Error::Unbundled)
    }

    /// Loads a descriptor from an explicit resource directory (useful for tests/custom layouts).
    /// # Errors
    /// Rejects unknown revisions, non-confined helper/runtime paths and invalid metadata.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let root = fs::canonicalize(root)?;
        let mut installation: Installation =
            serde_json::from_slice(&fs::read(root.join("rake-installation.json"))?)?;
        installation
            .normalize_paths()
            .map_err(|error| Error::Invalid(error.to_string()))?;
        if installation.schema_version != 1 {
            return Err(Error::Invalid("descriptor schema".into()));
        }
        for path in installation
            .helpers
            .values()
            .chain(installation.runtimes.values())
            .chain(installation.frameworks.values())
        {
            validate_relative_path(path).map_err(|e| Error::Invalid(e.to_string()))?;
        }
        Ok(Self { root, installation })
    }

    /// Returns the installed identity and provider policy.
    pub fn installation(&self) -> &Installation {
        &self.installation
    }

    /// Resolves an existing resource while rejecting traversal and escaping symlinks.
    /// # Errors
    /// Returns errors for invalid, missing or escaping paths.
    pub fn path(&self, relative: impl AsRef<Path>) -> Result<PathBuf, Error> {
        let relative = rake_dist_core::portable_relative_path(relative.as_ref())
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let path = fs::canonicalize(self.root.join(relative))?;
        if !path.starts_with(&self.root) {
            return Err(Error::Invalid("symlink escapes resources".into()));
        }
        Ok(path)
    }

    /// Resolves a configured private helper; never searches PATH.
    /// # Errors
    /// Returns errors for unknown or inaccessible helper entries.
    pub fn helper(&self, name: &str) -> Result<PathBuf, Error> {
        self.path(
            self.installation
                .helpers
                .get(name)
                .ok_or_else(|| Error::Invalid(format!("unknown helper {name}")))?,
        )
    }

    /// Resolves a macOS framework under Contents/Frameworks.
    /// # Errors
    /// Returns errors for missing frameworks or paths escaping the bundle.
    pub fn framework(&self, name: &str) -> Result<PathBuf, Error> {
        if self.installation.platform != rake_dist_core::Platform::Macos {
            return Err(Error::Invalid("frameworks require macOS".into()));
        }
        let root = fs::canonicalize(
            self.root
                .parent()
                .ok_or(Error::Unbundled)?
                .join("Frameworks"),
        )?;
        let relative = self
            .installation
            .frameworks
            .get(name)
            .ok_or_else(|| Error::Invalid(format!("unknown framework {name}")))?;
        let path = fs::canonicalize(root.join(relative))?;
        if !path.starts_with(&root) {
            return Err(Error::Invalid("framework escapes bundle".into()));
        }
        Ok(path)
    }

    /// Configures a helper process to discover only a packaged GStreamer runtime.
    /// # Errors
    /// Requires a runtime with lib/gstreamer-1.0 and a packaged scanner in libexec.
    pub fn gstreamer_command(
        &self,
        helper: &str,
        runtime: &str,
    ) -> Result<std::process::Command, Error> {
        let root = self.runtime(runtime)?;
        let plugins = root.join("lib/gstreamer-1.0");
        let scanner = root.join("libexec/gstreamer-1.0").join(if cfg!(windows) {
            "gst-plugin-scanner.exe"
        } else {
            "gst-plugin-scanner"
        });
        if !plugins.is_dir() || !scanner.is_file() {
            return Err(Error::Invalid("GStreamer plugin/scanner layout".into()));
        }
        let mut command = std::process::Command::new(self.helper(helper)?);
        command
            .env("GST_PLUGIN_SYSTEM_PATH_1_0", "")
            .env("GST_PLUGIN_PATH_1_0", plugins)
            .env("GST_PLUGIN_SCANNER_1_0", scanner);
        Ok(command)
    }

    /// Resolves a configured native runtime directory.
    /// # Errors
    /// Returns errors for unknown or inaccessible runtime entries.
    pub fn runtime(&self, name: &str) -> Result<PathBuf, Error> {
        self.path(
            self.installation
                .runtimes
                .get(name)
                .ok_or_else(|| Error::Invalid(format!("unknown runtime {name}")))?,
        )
    }
}

/// Resolves resources with directory metadata emitted by rake-build at the call site.
#[macro_export]
macro_rules! resources {
    () => {
        $crate::Resources::discover(
            option_env!("RAKE_RESOURCE_DIRECTORY").unwrap_or(env!("CARGO_PKG_NAME")),
        )
    };
}
