//! ELF inspection and relocation use native readelf/patchelf, never ldd execution.
use crate::{Error, files, run};
use rake_dist_core::LinuxBaseline;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

/// Actual ELF requirements found after relocation.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ElfReport {
    /// Unbundled SONAME dependencies requiring the supported runtime baseline.
    pub system_libraries: Vec<String>,
    /// Highest required GLIBC symbol version.
    pub glibc: Option<String>,
    /// Number of inspected executables/libraries.
    pub binaries: usize,
}

/// Rewrites private library references to `$ORIGIN` paths within the staged tree.
///
/// `groups` maps component identities to their installed roots; same-named libraries
/// resolve within the referring component first. Ambiguous dependencies fail closed.
/// # Errors
/// Returns native tool failures, unresolved/ambiguous bundled libraries, absolute
/// non-system dependencies, or GLIBC requirements exceeding the declared baseline.
pub fn normalize_elf(
    root: &Path,
    groups: &BTreeMap<String, Vec<PathBuf>>,
    baseline: Option<&LinuxBaseline>,
) -> Result<ElfReport, Error> {
    let root = fs::canonicalize(root)?;
    let mut binaries = Vec::new();
    let mut names: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in files(&root)? {
        let mut magic = [0; 4];
        if fs::File::open(&path)?.read(&mut magic)? != 4 || magic != *b"\x7fELF" {
            continue;
        }
        let header = run(Command::new("readelf").args(["-hW"]).arg(&path))?;
        if !header
            .lines()
            .any(|line| line.contains("Type:") && (line.contains("DYN") || line.contains("EXEC")))
        {
            continue;
        }
        let dynamic = run(Command::new("readelf").args(["-dW"]).arg(&path))?;
        let soname = entries(&dynamic, "(SONAME)")
            .next()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        names.entry(soname).or_default().push(path.clone());
        binaries.push((path, dynamic));
    }
    // SONAME symlinks are valid library aliases, but only inside this installation.
    for path in links(&root)? {
        let resolved = fs::canonicalize(&path)?;
        if !resolved.starts_with(&root) {
            return Err(Error::Metadata(
                "ELF library symlink escapes installation".into(),
            ));
        }
        if binaries.iter().any(|(path, _)| *path == resolved) {
            let values = names
                .entry(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                )
                .or_default();
            if !values.contains(&resolved) {
                values.push(resolved);
            }
        }
    }
    let mut system = BTreeSet::new();
    let mut glibc: Option<Vec<u32>> = None;
    for (path, dynamic) in &binaries {
        let owner = groups.values().find(|members| {
            members
                .iter()
                .any(|member| path.starts_with(root.join(member)))
        });
        let mut rpaths = BTreeSet::new();
        for dependency in entries(dynamic, "(NEEDED)") {
            let name = Path::new(dependency)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            if let Some(candidates) = names.get(name.as_ref()) {
                let local = candidates
                    .iter()
                    .filter(|candidate| {
                        owner.is_some_and(|members| {
                            members
                                .iter()
                                .any(|member| candidate.starts_with(root.join(member)))
                        })
                    })
                    .collect::<Vec<_>>();
                let candidates = if local.is_empty() {
                    candidates.iter().collect::<Vec<_>>()
                } else {
                    local
                };
                if candidates.len() != 1 {
                    return Err(Error::Metadata(format!(
                        "ambiguous ELF dependency {dependency} for {}",
                        path.display()
                    )));
                }
                let parent = path
                    .parent()
                    .ok_or_else(|| Error::Metadata("ELF parent".into()))?;
                let library_parent = candidates[0]
                    .parent()
                    .ok_or_else(|| Error::Metadata("ELF library parent".into()))?;
                let relative = relative(parent, library_parent);
                rpaths.insert(if relative.as_os_str().is_empty() {
                    "$ORIGIN".into()
                } else {
                    format!("$ORIGIN/{}", relative.display())
                });
                if dependency.contains('/') {
                    run(Command::new("patchelf")
                        .arg("--replace-needed")
                        .arg(dependency)
                        .arg(name.as_ref())
                        .arg(path))?;
                }
            } else {
                if dependency.contains('/') {
                    return Err(Error::Metadata(format!(
                        "unbundled absolute ELF dependency {dependency}"
                    )));
                }
                system.insert(dependency.to_owned());
            }
        }
        // Remove build-machine RPATH even if every dependency is system-provided.
        if dynamic.contains("(RPATH)") || dynamic.contains("(RUNPATH)") || !rpaths.is_empty() {
            run(Command::new("patchelf")
                .arg("--set-rpath")
                .arg(rpaths.into_iter().collect::<Vec<_>>().join(":"))
                .arg(path))?;
        }
        let versions = run(Command::new("readelf")
            .args(["--version-info", "-W"])
            .arg(path))?;
        for word in versions.split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '.'
        }) {
            if let Some(version) = word.strip_prefix("GLIBC_").and_then(version)
                && glibc.as_ref().is_none_or(|current| version > *current)
            {
                glibc = Some(version);
            }
        }
    }
    if let (Some(required), Some(allowed)) = (
        &glibc,
        baseline
            .and_then(|value| value.glibc.as_deref())
            .and_then(version),
    ) && *required > allowed
    {
        return Err(Error::Metadata(format!(
            "ELF requires GLIBC_{}, newer than declared {}",
            format_version(required),
            format_version(&allowed)
        )));
    }
    Ok(ElfReport {
        system_libraries: system.into_iter().collect(),
        glibc: glibc.as_deref().map(format_version),
        binaries: binaries.len(),
    })
}
fn entries<'a>(text: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> {
    text.lines()
        .filter(move |line| line.contains(tag))
        .filter_map(|line| {
            line.split_once('[')
                .and_then(|(_, tail)| tail.split_once(']').map(|(value, _)| value))
        })
}
fn version(value: &str) -> Option<Vec<u32>> {
    value
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()
}
fn format_version(value: &[u32]) -> String {
    value
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(".")
}
fn relative(from: &Path, to: &Path) -> PathBuf {
    let shared = from
        .components()
        .zip(to.components())
        .take_while(|(left, right)| left == right)
        .count();
    let mut output = PathBuf::new();
    for _ in from.components().skip(shared) {
        output.push("..");
    }
    output.extend(to.components().skip(shared));
    output
}
fn links(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            result.push(entry.path());
        } else if kind.is_dir() {
            result.extend(links(&entry.path())?);
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_native_dependencies_and_compares_versions_numerically() {
        assert_eq!(entries(" 0x1 (NEEDED) Shared library: [liba.so.1]\n 0x2 (SONAME) Library soname: [libb.so]","(NEEDED)").collect::<Vec<_>>(),vec!["liba.so.1"]);
        assert!(version("2.9").unwrap() < version("2.39").unwrap());
        assert_eq!(
            relative(Path::new("/usr/bin"), Path::new("/usr/lib/app")),
            PathBuf::from("../lib/app")
        );
    }
}
