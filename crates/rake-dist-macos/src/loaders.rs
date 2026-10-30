//! Normalizes every non-system load command against the declared staged inventory.
use crate::Error;
use object::read::macho::{FatArch, MachHeader};
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
                && is_loadable_macho(&fs::read(&path)?)
            {
                output.push(path);
            }
        }
    }
    Ok(())
}
fn is_loadable_macho(bytes: &[u8]) -> bool {
    let loadable = |kind| {
        matches!(
            kind,
            object::macho::MH_EXECUTE | object::macho::MH_DYLIB | object::macho::MH_BUNDLE
        )
    };
    let thin = |bytes: &[u8]| match object::File::parse(bytes) {
        Ok(object::File::MachO32(file)) => loadable(file.macho_header().filetype(file.endian())),
        Ok(object::File::MachO64(file)) => loadable(file.macho_header().filetype(file.endian())),
        _ => false,
    };
    if thin(bytes) {
        return true;
    }
    if let Ok(fat) = object::read::macho::MachOFatFile32::parse(bytes) {
        return !fat.arches().is_empty()
            && fat
                .arches()
                .iter()
                .all(|arch| arch.data(bytes).is_ok_and(thin));
    }
    if let Ok(fat) = object::read::macho::MachOFatFile64::parse(bytes) {
        return !fat.arches().is_empty()
            && fat
                .arches()
                .iter()
                .all(|arch| arch.data(bytes).is_ok_and(thin));
    }
    false
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
    let contents = fs::canonicalize(root)?.join("Contents");
    let descriptor = contents.join("Resources/rake-installation.json");
    let groups = if descriptor.is_file() {
        let installation: rake_dist_core::Installation =
            serde_json::from_slice(&fs::read(descriptor)?)
                .map_err(|error| Error::Native(error.to_string()))?;
        let mut groups = Vec::new();
        for members in installation.native_groups.into_values() {
            let mut paths = Vec::new();
            for member in members {
                rake_dist_core::validate_relative_path(&member)
                    .map_err(|error| Error::Native(error.to_string()))?;
                let member = fs::canonicalize(contents.join(member))?;
                if !member.starts_with(&contents) {
                    return Err(Error::Native("native group escapes bundle".into()));
                }
                paths.push(member);
            }
            groups.push(paths);
        }
        groups
    } else {
        Vec::new()
    };
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
        if file
            .extension()
            .is_some_and(|extension| extension == "dylib")
        {
            run(Command::new("install_name_tool")
                .arg("-id")
                .arg(format!(
                    "@rpath/{}",
                    file.file_name().unwrap_or_default().to_string_lossy()
                ))
                .arg(file))?;
        }
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
            let group = groups
                .iter()
                .find(|members| members.iter().any(|member| file.starts_with(member)));
            let local: Vec<_> = candidates
                .iter()
                .filter(|candidate| {
                    group.is_some_and(|members| {
                        members.iter().any(|member| candidate.starts_with(member))
                    })
                })
                .collect();
            let selected = if local.len() == 1 {
                local[0]
            } else if candidates.len() == 1 {
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
        let mut rpaths = BTreeSet::new();
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
                rpaths.insert(path);
            }
        }
        for path in rpaths {
            run(Command::new("install_name_tool")
                .args(["-delete_rpath", path])
                .arg(file))?;
        }
    }
    Ok(inventory)
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    fn macho(kind: u32) -> Vec<u8> {
        [0xfeedfacf_u32, 0x0100000c, 0, kind, 0, 0, 0, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect()
    }
    #[test]
    fn only_loadable_macho_images_enter_signing_and_loader_tools() {
        let root = tempfile::tempdir().unwrap();
        for (name, kind) in [
            ("app", object::macho::MH_EXECUTE),
            ("library", object::macho::MH_DYLIB),
            ("plugin", object::macho::MH_BUNDLE),
            ("object.o", object::macho::MH_OBJECT),
        ] {
            fs::write(root.path().join(name), macho(kind)).unwrap();
        }
        let mut fat: Vec<u8> = [0xcafebabe_u32, 1, 0x0100000c, 0, 4096, 32, 12]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect();
        fat.resize(4096, 0);
        fat.extend(macho(object::macho::MH_DYLIB));
        fs::write(root.path().join("universal"), fat).unwrap();
        fs::write(
            root.path().join("java.class"),
            [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 65, 0, 1, 7, 0],
        )
        .unwrap();
        let names: std::collections::BTreeSet<_> = signable_inventory(root.path())
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            ["app", "library", "plugin", "universal"]
                .into_iter()
                .map(std::ffi::OsString::from)
                .collect()
        );
    }
}
