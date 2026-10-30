//! Normalizes every non-system load command against the declared staged inventory.
use crate::Error;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

/// Returns regular Mach-O files, ordered inside-out for subsequent signing.
/// # Errors
/// Rejects unreadable files and escaping symlinks.
pub fn signable_inventory(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let root = fs::canonicalize(root)?;
    let mut files = Vec::new();
    walk(&root, &root, &mut files)?;
    files.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(files)
}
fn walk(root: &Path, path: &Path, output: &mut Vec<PathBuf>) -> Result<(), Error> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            if !fs::canonicalize(&path)?.starts_with(root) {
                return Err(Error::Native("bundle symlink escapes payload".into()));
            }
        } else if kind.is_dir() {
            walk(root, &path, output)?;
        } else if kind.is_file() {
            let mut magic = [0; 4];
            if fs::File::open(&path)?.read(&mut magic)? == 4
                && matches!(
                    magic,
                    [0xfe, 0xed, 0xfa, 0xce]
                        | [0xce, 0xfa, 0xed, 0xfe]
                        | [0xfe, 0xed, 0xfa, 0xcf]
                        | [0xcf, 0xfa, 0xed, 0xfe]
                        | [0xca, 0xfe, 0xba, 0xbe]
                        | [0xca, 0xfe, 0xba, 0xbf]
                )
            {
                output.push(path);
            }
        }
    }
    Ok(())
}

fn aliases(path: &Path, names: &mut BTreeMap<String, Vec<PathBuf>>) -> Result<(), Error> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            let canonical = fs::canonicalize(entry.path())?;
            if canonical.is_file() && names.values().any(|paths| paths.contains(&canonical)) {
                names
                    .entry(entry.file_name().to_string_lossy().into_owned())
                    .or_default()
                    .push(canonical);
            }
        } else if entry.file_type()?.is_dir() {
            aliases(&entry.path(), names)?;
        }
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<String, Error> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(Error::Native(format!(
            "native loader tool failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}
fn relative(from: &Path, to: &Path) -> PathBuf {
    let left: Vec<_> = from.components().collect();
    let right: Vec<_> = to.components().collect();
    let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..left.len() {
        path.push("..");
    }
    for item in &right[common..] {
        path.push(item.as_os_str());
    }
    path
}
fn system(path: &str) -> bool {
    path.starts_with("/usr/lib/") || path.starts_with("/System/Library/")
}

/// Rewrites all packaged native dependencies to bundle-relative loader paths.
///
/// Missing or ambiguous dependencies fail; libraries are never silently copied
/// from undeclared developer-machine directories. Call before final signing.
/// # Errors
/// Requires otool/install_name_tool and a complete confined native inventory.
pub fn normalize_loaders(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let inventory = signable_inventory(root)?;
    let mut names: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for file in &inventory {
        names
            .entry(
                file.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
            .or_default()
            .push(file.clone());
    }
    aliases(root, &mut names)?;
    for values in names.values_mut() {
        values.sort();
        values.dedup();
    }
    for file in &inventory {
        let output = run(Command::new("otool").arg("-L").arg(file))?;
        let dependencies: BTreeSet<_> = output
            .lines()
            .filter(|line| line.starts_with('\t'))
            .filter_map(|line| line.trim().split(" (compatibility version").next())
            .collect();
        for dependency in dependencies {
            if system(dependency) {
                continue;
            }
            let name = Path::new(dependency)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let candidates = names.get(name.as_ref()).ok_or_else(|| {
                Error::Native(format!(
                    "undeclared native dependency {dependency} required by {}",
                    file.display()
                ))
            })?;
            let selected = if candidates.len() == 1 {
                &candidates[0]
            } else {
                let relative = dependency.strip_prefix("@rpath/").unwrap_or(dependency);
                let matching: Vec<_> = candidates
                    .iter()
                    .filter(|candidate| candidate.ends_with(relative))
                    .collect();
                if matching.len() != 1 {
                    return Err(Error::Native(format!(
                        "ambiguous native dependency {dependency}"
                    )));
                }
                matching[0]
            };
            if selected == file {
                continue;
            }
            let loader = format!(
                "@loader_path/{}",
                relative(
                    file.parent()
                        .ok_or_else(|| Error::Native("native file parent".into()))?,
                    selected
                )
                .to_string_lossy()
            );
            if dependency != loader {
                run(Command::new("install_name_tool")
                    .args(["-change", dependency, &loader])
                    .arg(file))?;
            }
        }
        let commands = run(Command::new("otool").arg("-l").arg(file))?;
        let mut in_rpath = false;
        for line in commands.lines() {
            let line = line.trim();
            if line.starts_with("cmd ") {
                in_rpath = line == "cmd LC_RPATH";
            }
            if in_rpath
                && let Some(path) = line
                    .strip_prefix("path ")
                    .and_then(|line| line.split(" (offset ").next())
                && path.starts_with('/')
                && !system(path)
            {
                run(Command::new("install_name_tool")
                    .args(["-delete_rpath", path])
                    .arg(file))?;
            }
        }
    }
    Ok(inventory)
}
