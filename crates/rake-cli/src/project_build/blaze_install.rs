//! Installs and verifies the exact experimental live compiler; never searches PATH.
use super::Result;
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

const SERVER: &str = "rust-analyzer-proc-macro-srv";
const MANIFEST: &str = "RAKE_BLAZE.json";
const SAVED_ARCHIVE: &str = ".archive.tar.gz";
const MAX_ARCHIVE: u64 = 2 << 30;
const MAX_UNPACKED: u64 = 4 << 30;

#[derive(Deserialize)]
struct Pin {
    repo: String,
    version: String,
    url: String,
    sha256: String,
    target: String,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    target: String,
    files: BTreeMap<String, String>,
}

fn pin() -> Result<Pin> {
    Ok(toml::from_str(include_str!("../../blaze-pin.toml"))?)
}

fn supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").ok_or("HOME is required for the Rake live compiler")?,
    )
    .join(".rake/blaze"))
}

fn hash(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buf = [0; 65536];

    loop {
        let n = f.read(&mut buf)?;

        if n == 0 {
            break;
        }

        digest.update(&buf[..n]);
    }

    Ok(format!("{:x}", digest.finalize()))
}

fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn root_name(pin: &Pin) -> String {
    format!("blaze-v{}-{}", pin.version, pin.target)
}

fn check_archive(pin: &Pin, archive: &Path) -> Result<()> {
    if fs::metadata(archive)?.len() > MAX_ARCHIVE {
        return Err("Blaze archive exceeds 2 GiB".into());
    }

    if hash(archive)? != pin.sha256 {
        return Err("pinned Blaze archive checksum mismatch; installation refused".into());
    }

    Ok(())
}

fn archive_manifest(pin: &Pin, archive: &Path) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(GzDecoder::new(fs::File::open(archive)?));
    let expected = PathBuf::from(root_name(pin)).join(MANIFEST);

    for item in tar.entries()? {
        let mut item = item?;

        if item.path()?.as_ref() == expected {
            if item.size() > 4 << 20 {
                return Err("Blaze manifest exceeds 4 MiB".into());
            }

            let mut bytes = Vec::new();

            item.read_to_end(&mut bytes)?;

            return Ok(bytes);
        }
    }

    Err("Blaze archive has no Rake manifest".into())
}

fn validate(pin: &Pin, dir: &Path) -> Result<()> {
    let archive = dir.join(SAVED_ARCHIVE);

    check_archive(pin, &archive)?;
    let bytes = archive_manifest(pin, &archive)?;

    if fs::read(dir.join(MANIFEST))? != bytes {
        return Err("installed Blaze manifest checksum mismatch".into());
    }

    let manifest: Manifest = serde_json::from_slice(&bytes)?;

    if manifest.version != pin.version || manifest.target != pin.target {
        return Err("Blaze archive version/target differs from the pin".into());
    }

    for required in [
        "blaze",
        "blaze-engine",
        "blaze-rustc",
        SERVER,
        "share/blaze/library/DIGEST",
    ] {
        if !manifest.files.contains_key(required) {
            return Err(format!("Blaze archive is missing {required}").into());
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for name in ["blaze", "blaze-engine", "blaze-rustc", SERVER] {
            if fs::metadata(dir.join(name))?.permissions().mode() & 0o111 == 0 {
                return Err(format!("installed Blaze file is not executable: {name}").into());
            }
        }
    }

    for (name, expected) in manifest.files {
        let path = Path::new(&name);

        if !relative(path)
            || !fs::symlink_metadata(dir.join(path))?.is_file()
            || hash(&dir.join(path))? != expected
        {
            return Err(format!("installed Blaze checksum mismatch: {name}").into());
        }
    }

    Ok(())
}

fn install_archive(pin: &Pin, archive: &Path, parent: &Path) -> Result<PathBuf> {
    check_archive(pin, archive)?;
    fs::create_dir_all(parent)?;
    let destination = parent.join(&pin.version);

    if destination.exists() {
        validate(pin, &destination)?;

        return Ok(destination.join("blaze"));
    }

    let temp = tempfile::Builder::new()
        .prefix(".blaze-install-")
        .tempdir_in(parent)?;

    let root = root_name(pin);
    let mut total = 0u64;
    let mut tar = tar::Archive::new(GzDecoder::new(fs::File::open(archive)?));

    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();

        if !relative(&path)
            || path
                .components()
                .next()
                .is_none_or(|c| c.as_os_str() != root.as_str())
        {
            return Err("unsafe path in Blaze archive".into());
        }

        let kind = entry.header().entry_type();
        let output = temp.path().join(&path);

        if kind.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }

        if !kind.is_file() {
            return Err("links and special files are refused in Blaze archives".into());
        }

        total = total
            .checked_add(entry.size())
            .ok_or("Blaze archive size overflow")?;

        if total > MAX_UNPACKED {
            return Err("Blaze archive exceeds 4 GiB unpacked".into());
        }

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)?;

        std::io::copy(&mut entry, &mut file)?;
        file.flush()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &output,
                fs::Permissions::from_mode(entry.header().mode()? & 0o777),
            )?;
        }
    }

    let staged = temp.path().join(root);

    fs::copy(archive, staged.join(SAVED_ARCHIVE))?;
    validate(pin, &staged)?;

    // A concurrent successful install wins; never replace an existing installation.
    if destination.exists() {
        validate(pin, &destination)?;
    } else {
        fs::rename(staged, &destination)?;
    }

    Ok(destination.join("blaze"))
}

/// Selects an explicit override or the checksum-verified pinned compiler.
pub(super) fn ensure() -> Result<PathBuf> {
    if !supported() {
        return Err(
            "rake dev --live currently requires an Apple Silicon Mac (aarch64-apple-darwin)".into(),
        );
    }

    if let Some(path) = std::env::var_os("RAKE_BLAZE") {
        let path = fs::canonicalize(path)?;

        if !path.is_file()
            || !path.parent().is_some_and(|p| {
                [SERVER, "blaze-engine", "blaze-rustc"]
                    .iter()
                    .all(|name| p.join(name).is_file())
            })
        {
            return Err("RAKE_BLAZE must name a compiler with the patched rust-analyzer-proc-macro-srv beside it".into());
        }

        return Ok(path);
    }

    let pin = pin()?;
    let parent = home()?;
    let dir = parent.join(&pin.version);

    if dir.exists() {
        validate(&pin, &dir)?;

        return Ok(dir.join("blaze"));
    }

    if let Some(archive) = std::env::var_os("BLAZE_ARCHIVE") {
        return install_archive(&pin, Path::new(&archive), &parent);
    }

    eprintln!(
        "rake: installing pinned Blaze {} from {}",
        pin.version, pin.repo
    );
    let download = tempfile::NamedTempFile::new()?;

    let status = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            "600",
            "--max-filesize",
            "2147483648",
            "--retry",
            "2",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(download.path())
        .arg(&pin.url)
        .status()?;

    if !status.success() {
        return Err(format!("cannot download pinned Blaze {} from {} (fork release may not be published yet); use BLAZE_ARCHIVE for the prepared local archive",pin.version,pin.url).into());
    }

    install_archive(&pin, download.path(), &parent)
}

/// Readiness is diagnostic only and never downloads or changes an installation.
pub(super) fn readiness() -> Value {
    let result = (|| -> Result<Value> {
        let pin = pin()?;
        let dir = home()?.join(&pin.version);
        let installed = dir.join("blaze").is_file();
        let checked = validate(&pin, &dir);
        let checksum = checked.is_ok();
        let server = dir.join(SERVER).is_file();

        Ok(
            json!({"supported":supported(),"installed":installed,"version":pin.version,"path":dir,"checksum_matches":checksum,"macro_server_present":server,"ready":supported()&&checksum&&server,"note":checked.err().map(|e|e.to_string())}),
        )
    })();

    result.unwrap_or_else(|e| json!({"ready":false,"supported":supported(),"note":e.to_string()}))
}

#[cfg(test)]
mod tests {
    //! Real gzip/tar/filesystem; fake local binaries; no network or Blaze required.
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    fn fixture(root: &Path) -> (Pin, PathBuf) {
        let mut pin = Pin {
            repo: "fixture/blaze".into(),
            version: "0.20261030.1-rake.3".into(),
            target: "aarch64-apple-darwin".into(),
            url: "https://invalid.test/archive".into(),
            sha256: String::new(),
        };

        let path = root.join("fixture.tar.gz");
        let gzip = GzEncoder::new(fs::File::create(&path).unwrap(), Compression::fast());
        let mut tar = tar::Builder::new(gzip);
        let mut files = BTreeMap::new();

        for name in [
            "blaze",
            "blaze-engine",
            "blaze-rustc",
            SERVER,
            "share/blaze/library/DIGEST",
        ] {
            let data = format!("fixture {name}");

            files.insert(
                name.to_owned(),
                format!("{:x}", Sha256::digest(data.as_bytes())),
            );
            append(
                &mut tar,
                &format!("{}/{name}", root_name(&pin)),
                data.as_bytes(),
            );
        }

        let manifest = json!({"version":pin.version,"target":pin.target,"files":files}).to_string();

        append(
            &mut tar,
            &format!("{}/{}", root_name(&pin), MANIFEST),
            manifest.as_bytes(),
        );
        tar.into_inner().unwrap().finish().unwrap();
        pin.sha256 = hash(&path).unwrap();

        (pin, path)
    }

    fn append(tar: &mut tar::Builder<GzEncoder<fs::File>>, name: &str, data: &[u8]) {
        let mut h = tar::Header::new_gnu();

        h.set_size(data.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        tar.append_data(&mut h, name, data).unwrap();
    }

    #[test]
    fn installs_offline_and_rejects_tampered_installed_files() {
        let temp = tempfile::tempdir().unwrap();
        let (pin, path) = fixture(temp.path());
        let parent = temp.path().join("installed");
        let exe = install_archive(&pin, &path, &parent).unwrap();

        assert!(exe.is_file());
        validate(&pin, exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"tampered").unwrap();
        assert!(
            validate(&pin, exe.parent().unwrap())
                .unwrap_err()
                .to_string()
                .contains("checksum mismatch")
        );
        assert!(
            install_archive(&pin, &path, &parent).is_err(),
            "never accept or overwrite a damaged installation"
        );
    }

    #[test]
    fn wrong_checksum_cannot_install() {
        let temp = tempfile::tempdir().unwrap();
        let (mut pin, path) = fixture(temp.path());

        pin.sha256 = "0".repeat(64);
        let parent = temp.path().join("installed");

        assert!(
            install_archive(&pin, &path, &parent)
                .unwrap_err()
                .to_string()
                .contains("checksum mismatch")
        );
        assert!(!parent.exists());
    }

    #[test]
    fn authenticated_manifest_cannot_be_rewritten_to_hide_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let (pin, path) = fixture(temp.path());
        let exe = install_archive(&pin, &path, &temp.path().join("installed")).unwrap();

        fs::write(exe.parent().unwrap().join(MANIFEST), b"{}").unwrap();
        assert!(
            validate(&pin, exe.parent().unwrap())
                .unwrap_err()
                .to_string()
                .contains("manifest checksum mismatch")
        );
    }

    #[test]
    fn archive_paths_are_confined() {
        for bad in ["../outside", "/absolute", "root/../../outside", ""] {
            assert!(!relative(Path::new(bad)), "{bad}");
        }

        assert!(relative(Path::new("root/blaze")));
    }
}
