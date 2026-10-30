//! Repository production delegates native signing and metadata to established tools.
use crate::{Error, run};
use rake_dist_core::Format;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

/// An isolated repository build; existing published repositories are never mutated.
#[derive(Debug)]
pub struct Repository<'a> {
    /// Debian, RPM, Arch or Flatpak format.
    pub format: Format,
    /// Application identity for Flatpak refs.
    pub identifier: &'a str,
    /// Repository name; ASCII letters/digits/dashes only.
    pub name: &'a str,
    /// Exact packages, including explicitly retained versions.
    pub packages: &'a [PathBuf],
    /// New empty staging directory.
    pub output: &'a Path,
    /// Existing local GPG identity/fingerprint, never private key material.
    pub gpg_key: &'a str,
    /// Public HTTPS URL of this repository.
    pub public_url: &'a str,
    /// Flatpak branch; defaults are selected by the caller.
    pub branch: &'a str,
}
/// Creates signed native repository metadata and an enrollment public key.
///
/// Debian uses a flat APT repository (`deb [signed-by=...] URL ./`). RPM uses
/// createrepo_c and a signed repomd.xml. Arch signs every package and its database.
/// Flatpak imports bundles and signs OSTree commits and summaries with native tools.
/// # Errors
/// Rejects unsupported formats, missing tools/keys, nonempty output and native failures.
/// Run within the distribution worker's process group for bounded cancellation.
pub fn build_repository(input: &Repository<'_>) -> Result<Vec<PathBuf>, Error> {
    if input.name.is_empty()
        || !input
            .name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || !(16..=64).contains(&input.gpg_key.len())
        || !input.gpg_key.bytes().all(|byte| byte.is_ascii_hexdigit())
        || input.packages.is_empty()
    {
        return Err(Error::Metadata(
            "repository name/key/packages invalid".into(),
        ));
    }
    if !input.public_url.starts_with("https://")
        || input.public_url.contains(['\n', '\r'])
        || input.branch.contains(['\n', '\r'])
    {
        return Err(Error::Metadata(
            "repository public URL/branch invalid".into(),
        ));
    }
    fs::create_dir_all(input.output)?;
    if fs::read_dir(input.output)?.next().is_some() {
        return Err(Error::Metadata("repository output must be empty".into()));
    }
    let public = run(Command::new("gpg").args(["--batch", "--armor", "--export", input.gpg_key]))?;
    if !public.contains("BEGIN PGP PUBLIC KEY BLOCK") {
        return Err(Error::Metadata(
            "repository signing public key unavailable".into(),
        ));
    }
    fs::write(input.output.join("public-key.asc"), &public)?;
    let packages = input.output.join("packages");
    fs::create_dir(&packages)?;
    let mut staged = Vec::new();
    for source in input.packages {
        if source.is_symlink() || !source.is_file() {
            return Err(Error::Metadata(
                "repository input must be a regular package file".into(),
            ));
        }
        let destination = packages.join(
            source
                .file_name()
                .ok_or_else(|| Error::Metadata("package filename".into()))?,
        );
        if destination.exists() {
            return Err(Error::Metadata(
                "repository package filename collision".into(),
            ));
        }
        fs::copy(source, &destination)?;
        staged.push(destination);
    }
    match input.format {
        Format::Deb => {
            let index = run(Command::new("dpkg-scanpackages")
                .current_dir(input.output)
                .args(["--multiversion", "packages"]))?;
            fs::write(input.output.join("Packages"), index.as_bytes())?;
            let mut gzip = flate2::write::GzEncoder::new(
                fs::File::create(input.output.join("Packages.gz"))?,
                flate2::Compression::default(),
            );
            gzip.write_all(index.as_bytes())?;
            gzip.finish()?;
            let release = run(Command::new("apt-ftparchive")
                .current_dir(input.output)
                .args(["release", "."]))?;
            fs::write(input.output.join("Release"), release)?;
            sign(
                input,
                &input.output.join("Release"),
                &input.output.join("InRelease"),
                true,
            )?;
            sign(
                input,
                &input.output.join("Release"),
                &input.output.join("Release.gpg"),
                false,
            )?;
        }
        Format::Rpm => {
            let keyring = input.output.join(".rpm-keyring");
            fs::create_dir(&keyring)?;
            run(Command::new("rpmkeys")
                .arg("--dbpath")
                .arg(&keyring)
                .arg("--import")
                .arg(input.output.join("public-key.asc")))?;
            for package in &staged {
                run(Command::new("rpmsign")
                    .args([
                        "--define",
                        &format!("_gpg_name {}", input.gpg_key),
                        "--addsign",
                    ])
                    .arg(package))?;
                run(Command::new("rpmkeys")
                    .arg("--dbpath")
                    .arg(&keyring)
                    .arg("--checksig")
                    .arg(package))?;
            }
            fs::remove_dir_all(&keyring)?;
            run(Command::new("createrepo_c").arg(input.output))?;
            let metadata = input.output.join("repodata/repomd.xml");
            sign(input, &metadata, &metadata.with_extension("xml.asc"), false)?;
        }
        Format::Arch => {
            for package in &staged {
                sign(
                    input,
                    package,
                    &PathBuf::from(format!("{}.sig", package.display())),
                    false,
                )?;
            }
            run(Command::new("repo-add")
                .args(["--sign", "--key", input.gpg_key, "--prevent-downgrade"])
                .arg(input.output.join(format!("{}.db.tar.gz", input.name)))
                .args(&staged))?;
        }
        Format::Flatpak => {
            let repo = input.output.join("repo");
            for package in &staged {
                run(Command::new("flatpak")
                    .arg("build-import-bundle")
                    .arg(format!("--gpg-sign={}", input.gpg_key))
                    .arg(&repo)
                    .arg(package))?;
            }
            run(Command::new("flatpak")
                .args(["build-update-repo", "--generate-static-deltas"])
                .arg(format!("--gpg-sign={}", input.gpg_key))
                .arg(&repo))?;
            use base64::Engine as _;
            // Export binary without UTF-8 decoding; GPG key packets are arbitrary bytes.
            let binary = Command::new("gpg")
                .args(["--batch", "--export", input.gpg_key])
                .output()?;
            if !binary.status.success() {
                return Err(Error::Metadata("GPG key export failed".into()));
            }
            let key = base64::engine::general_purpose::STANDARD.encode(binary.stdout);
            fs::write(
                input.output.join(format!("{}.flatpakrepo", input.name)),
                format!(
                    "[Flatpak Repo]\nTitle={}\nUrl={}/repo\nGPGKey={key}\n",
                    input.name,
                    input.public_url.trim_end_matches('/')
                ),
            )?;
            fs::write(
                input.output.join(format!("{}.flatpakref", input.name)),
                format!(
                    "[Flatpak Ref]\nName={}\nBranch={}\nTitle={}\nIsRuntime=false\nUrl={}/repo\nGPGKey={key}\n",
                    input.identifier,
                    input.branch,
                    input.name,
                    input.public_url.trim_end_matches('/')
                ),
            )?;
        }
        _ => {
            return Err(Error::Metadata(
                "format does not have a native repository".into(),
            ));
        }
    }
    repository_files(input.output, input.output)
}
fn sign(input: &Repository<'_>, source: &Path, output: &Path, clear: bool) -> Result<(), Error> {
    let mut command = Command::new("gpg");
    command
        .args([
            "--batch",
            "--yes",
            "--local-user",
            input.gpg_key,
            "--digest-algo",
            "SHA256",
            "--output",
        ])
        .arg(output);
    command.arg(if clear { "--clearsign" } else { "--armor" });
    if !clear {
        command.arg("--detach-sign");
    }
    command.arg(source);
    run(&mut command)?;
    let mut verify = Command::new("gpg");
    verify.args(["--batch", "--verify"]).arg(output);
    if !clear {
        verify.arg(source);
    }
    run(&mut verify)?;
    Ok(())
}

fn repository_files(root: &Path, path: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            let actual = fs::canonicalize(&path)?;
            if !actual.starts_with(fs::canonicalize(root)?) || !actual.is_file() {
                return Err(Error::Metadata("repository symlink escapes output".into()));
            }
            let bytes = fs::read(&actual)?;
            fs::remove_file(&path)?;
            fs::write(&path, bytes)?;
            files.push(path);
        } else if entry.file_type()?.is_dir() {
            files.extend(repository_files(root, &path)?);
        } else if entry.file_type()?.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
