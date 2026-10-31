//! Application-local release CRT files from the enrolled Visual Studio toolchain.
use crate::Error;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Finds the target's release CRT redistributable directory in installed MSVC tools.
/// Never searches PATH/System32 or uses debug_nonredist files. The developer's
/// Visual Studio redistribution terms apply to these toolchain-provided files.
/// # Errors
/// Returns malformed target or unreadable toolchain enrollment errors.
pub fn vc_runtime_directory(target: &str) -> Result<Option<PathBuf>, Error> {
    let architecture = if target.starts_with("x86_64-") {
        "x64"
    } else if target.starts_with("aarch64-") {
        "arm64"
    } else if target.starts_with("i686-") {
        "x86"
    } else {
        return Err(Error::Tool("unsupported MSVC runtime target".into()));
    };

    if let Some(root) = std::env::var_os("VCToolsRedistDir") {
        return select(&PathBuf::from(root), architecture);
    }

    let Some(program_files) = std::env::var_os("ProgramFiles(x86)") else {
        return Ok(None);
    };

    let tool = PathBuf::from(program_files).join("Microsoft Visual Studio/Installer/vswhere.exe");

    if !tool.is_file() {
        return Ok(None);
    }

    let result = Command::new(tool)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()?;

    if !result.status.success() {
        return Err(Error::Tool(
            "Visual Studio redistributable discovery failed".into(),
        ));
    }

    let installation = String::from_utf8_lossy(&result.stdout);
    let installation = installation.trim();

    if installation.is_empty() {
        return Ok(None);
    }

    let root = Path::new(installation).join("VC/Redist/MSVC");

    if !root.is_dir() {
        return Ok(None);
    }

    let mut versions = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;

        let Some(version) = entry.file_name().to_str().and_then(|value| {
            value
                .split('.')
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()
        }) else {
            continue;
        };

        versions.push((version, entry.path()));
    }

    versions.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, root) in versions {
        if let Some(path) = select(&root, architecture)? {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn select(root: &Path, architecture: &str) -> Result<Option<PathBuf>, Error> {
    let directory = root.join(architecture);

    if !directory.is_dir() {
        return Ok(None);
    }

    let mut candidates = Vec::new();

    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name.starts_with("Microsoft.VC") && name.ends_with(".CRT") && entry.file_type()?.is_dir()
        {
            candidates.push(entry.path());
        }
    }

    candidates.sort();

    if candidates.len() > 1 {
        return Err(Error::Tool(
            "ambiguous release CRT directories; set VCToolsRedistDir to the enrolled toolset"
                .into(),
        ));
    }

    candidates
        .pop()
        .map(fs::canonicalize)
        .transpose()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_crt_selection_keeps_architectures_and_debug_payloads_separate() {
        let root = tempfile::tempdir().unwrap();

        fs::create_dir_all(
            root.path()
                .join("debug_nonredist/x64/Microsoft.VC143.DebugCRT"),
        )
        .unwrap();
        fs::create_dir_all(root.path().join("x86/Microsoft.VC143.CRT")).unwrap();
        assert!(select(root.path(), "x64").unwrap().is_none());
        let selected = root.path().join("x64/Microsoft.VC143.CRT");

        fs::create_dir_all(&selected).unwrap();
        assert_eq!(
            select(root.path(), "x64").unwrap().unwrap(),
            fs::canonicalize(selected).unwrap()
        );
    }
}
