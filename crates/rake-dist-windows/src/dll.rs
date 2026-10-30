//! Private DLL deployment uses declared roots, never a developer's PATH.
use crate::Error;
use object::Object;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// One executable's explicitly selected native dependency scope.
#[derive(Debug)]
pub struct DllDeployment<'a> {
    /// Main executable or private helper whose loader directory is being prepared.
    pub executable: &'a Path,
    /// Declared runtime directories visible to this process. Dynamically loaded
    /// libraries/plugins in these roots also seed transitive dependency discovery.
    pub roots: &'a [PathBuf],
    /// Toolchain release CRT directories, resolved only for imports absent from
    /// declared roots. Their unused DLLs do not seed plugin discovery.
    pub fallback_roots: &'a [PathBuf],
    /// Owned staging directory that becomes this executable's installed directory.
    pub destination: &'a Path,
    /// Additional DLLs explicitly supplied by an enrolled system prerequisite.
    pub system_dlls: &'a [String],
}
/// Returns imported DLL basenames from a real PE image, without invoking native tools.
/// # Errors
/// Rejects malformed/non-PE images, import tables and nonportable DLL names.
pub fn pe_imports(path: &Path) -> Result<Vec<String>, Error> {
    let bytes = fs::read(path)?;
    let file =
        object::File::parse(bytes.as_slice()).map_err(|error| Error::Tool(error.to_string()))?;
    if file.format() != object::BinaryFormat::Pe {
        return Err(Error::Tool("DLL deployment requires a PE image".into()));
    }
    let mut imports = BTreeSet::new();
    for import in file
        .imports()
        .map_err(|error| Error::Tool(error.to_string()))?
    {
        let name = std::str::from_utf8(import.library())
            .map_err(|error| Error::Tool(error.to_string()))?
            .to_ascii_lowercase();
        if !dll_name(&name) && !system(&name) {
            return Err(Error::Tool("invalid imported DLL basename".into()));
        }
        imports.insert(name);
    }
    Ok(imports.into_iter().collect())
}
/// Copies a complete private dependency closure beside its executable before signing.
///
/// Runtime DLLs remain at their declared resource paths too, so callers can still
/// resolve plugin/runtime directories. Separate helpers get separate loader scopes.
/// Conflicting DLL versions in one process fail instead of relying on PATH order.
/// # Errors
/// Rejects undeclared imports, conflicting basenames, escaped links and invalid PEs.
/// Call only with an owned staging destination; source/project files are not mutated.
pub fn deploy_dlls(input: &DllDeployment<'_>) -> Result<Vec<PathBuf>, Error> {
    let mut catalog: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for root in input.roots {
        let root = fs::canonicalize(root)?;
        for path in dll_files(&root, &root)? {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| Error::Tool("DLL filename must be UTF-8".into()))?
                .to_ascii_lowercase();
            catalog.entry(name).or_default().push(path);
        }
    }
    let seeds: Vec<_> = catalog.keys().cloned().collect();
    for root in input.fallback_roots {
        let root = fs::canonicalize(root)?;
        for path in dll_files(&root, &root)? {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| Error::Tool("CRT DLL filename".into()))?
                .to_ascii_lowercase();
            catalog.entry(name).or_insert_with(|| vec![path]);
        }
    }
    let machine = pe_architecture(input.executable)?;
    let mut queue = vec![input.executable.to_owned()];
    let mut selected = BTreeMap::new();
    for name in &seeds {
        let path = select(&catalog, name)?;
        queue.push(path.clone());
        selected.insert(name.clone(), path);
    }
    let mut inspected = BTreeSet::new();
    while let Some(path) = queue.pop() {
        if !inspected.insert(fs::canonicalize(&path)?) {
            continue;
        }
        if pe_architecture(&path)? != machine {
            return Err(Error::Tool(format!(
                "private DLL architecture conflicts with {}",
                input.executable.display()
            )));
        }
        for name in pe_imports(&path)? {
            if system(&name)
                || input
                    .system_dlls
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            let dependency=select(&catalog,&name).map_err(|_|Error::Tool(format!("undeclared or conflicting DLL {name} required by {}; add its runtime/component or declare an installed system prerequisite",path.display())))?;
            if selected.insert(name, dependency.clone()).is_none() {
                queue.push(dependency);
            }
        }
    }
    fs::create_dir_all(input.destination)?;
    let mut deployed = Vec::new();
    for (name, path) in selected {
        let output = input.destination.join(name);
        if output.exists() {
            if digest(&output)? != digest(&path)? {
                return Err(Error::Tool("private DLL destination collision".into()));
            }
        } else {
            fs::copy(path, &output)?;
        }
        deployed.push(output);
    }
    Ok(deployed)
}
fn dll_files(root: &Path, directory: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let actual = fs::canonicalize(&path)?;
        if !actual.starts_with(root) {
            return Err(Error::Tool(
                "native DLL symlink escapes declared runtime".into(),
            ));
        }
        if entry.file_type()?.is_dir() {
            files.extend(dll_files(root, &path)?);
        } else if actual.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("dll"))
        {
            files.push(path);
        }
    }
    Ok(files)
}
fn digest(path: &Path) -> Result<Vec<u8>, Error> {
    let mut hash = Sha256::new();
    std::io::copy(&mut fs::File::open(path)?, &mut hash)?;
    Ok(hash.finalize().to_vec())
}
fn select(catalog: &BTreeMap<String, Vec<PathBuf>>, name: &str) -> Result<PathBuf, Error> {
    let values = catalog
        .get(name)
        .ok_or_else(|| Error::Tool(format!("undeclared DLL {name}")))?;
    let first = values
        .first()
        .ok_or_else(|| Error::Tool("empty DLL catalog entry".into()))?;
    let hash = digest(first)?;
    for value in &values[1..] {
        if digest(value)? != hash {
            return Err(Error::Tool(format!(
                "conflicting versions of {name} in one process"
            )));
        }
    }
    Ok(first.clone())
}
fn dll_name(name: &str) -> bool {
    name.ends_with(".dll")
        && name
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b"._-".contains(&value))
        && !name.starts_with('.')
}
fn system(name: &str) -> bool {
    name.starts_with("api-ms-win-")
        || name.starts_with("ext-ms-win-")
        || [
            "comctl32.dll",
            "shcore.dll",
            "pathcch.dll",
            "oleacc.dll",
            "uiautomationcore.dll",
            "windowscodecs.dll",
            "msctf.dll",
            "urlmon.dll",
            "win32u.dll",
            "msvcp_win.dll",
            "ntmarta.dll",
            "netapi32.dll",
            "netutils.dll",
            "wtsapi32.dll",
            "dxcore.dll",
            "dxva2.dll",
            "mfplat.dll",
            "mf.dll",
            "mfreadwrite.dll",
            "evr.dll",
            "kernel32.dll",
            "kernelbase.dll",
            "ntdll.dll",
            "user32.dll",
            "gdi32.dll",
            "gdi32full.dll",
            "advapi32.dll",
            "shell32.dll",
            "shlwapi.dll",
            "ole32.dll",
            "oleaut32.dll",
            "combase.dll",
            "rpcrt4.dll",
            "ws2_32.dll",
            "bcrypt.dll",
            "bcryptprimitives.dll",
            "crypt32.dll",
            "secur32.dll",
            "sspicli.dll",
            "ncrypt.dll",
            "ucrtbase.dll",
            "msvcrt.dll",
            "userenv.dll",
            "version.dll",
            "imm32.dll",
            "winmm.dll",
            "winhttp.dll",
            "wininet.dll",
            "normaliz.dll",
            "dbghelp.dll",
            "iphlpapi.dll",
            "dhcpcsvc.dll",
            "dnsapi.dll",
            "dwmapi.dll",
            "uxtheme.dll",
            "dwrite.dll",
            "d2d1.dll",
            "d3d11.dll",
            "d3d12.dll",
            "dxgi.dll",
            "dcomp.dll",
            "d3dcompiler_47.dll",
            "opengl32.dll",
            "glu32.dll",
            "setupapi.dll",
            "cfgmgr32.dll",
            "powrprof.dll",
            "avrt.dll",
            "winspool.drv",
            "propsys.dll",
            "wintypes.dll",
            "msimg32.dll",
            "wintrust.dll",
            "psapi.dll",
            "cabinet.dll",
            "winusb.dll",
            "hid.dll",
        ]
        .contains(&name)
}
fn pe_architecture(path: &Path) -> Result<object::Architecture, Error> {
    let bytes = fs::read(path)?;
    let file =
        object::File::parse(bytes.as_slice()).map_err(|error| Error::Tool(error.to_string()))?;
    if file.format() != object::BinaryFormat::Pe {
        return Err(Error::Tool("native dependency is not a PE image".into()));
    }
    Ok(file.architecture())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_versions_are_only_allowed_when_bytes_match() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.dll");
        let second = root.path().join("second.dll");
        fs::write(&first, b"same").unwrap();
        fs::write(&second, b"same").unwrap();
        let catalog = BTreeMap::from([("codec.dll".into(), vec![first, second.clone()])]);
        assert!(select(&catalog, "codec.dll").is_ok());
        fs::write(second, b"different").unwrap();
        assert!(select(&catalog, "codec.dll").is_err());
        assert!(select(&catalog, "missing.dll").is_err());
    }
    #[test]
    fn redistributable_crt_is_not_silently_treated_as_an_os_library() {
        assert!(system("api-ms-win-crt-runtime-l1-1-0.dll"));
        assert!(system("ucrtbase.dll"));
        assert!(!system("vcruntime140.dll"));
        assert!(!system("gstcoreelements.dll"));
        assert!(!dll_name("../codec.dll"));
    }
}
