//! Built-in update signatures; metadata and logs contain references, never private seeds.
use crate::{Error, UpdateSigningKey};
use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
};
use zeroize::Zeroizing;

fn fail(message: &str) -> Error {
    Error::Execution(message.into())
}
fn decode(bytes: &[u8]) -> Result<SigningKey, Error> {
    if bytes.len() > 1024 {
        return Err(fail("private signing seed exceeds size limit"));
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    if bytes.len() == 32 {
        seed.copy_from_slice(bytes);
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| fail("invalid Ed25519 private seed encoding"))?
            .trim();
        let decoded = Zeroizing::new(if text.len() == 64 {
            hex::decode(text).map_err(|_| fail("invalid Ed25519 private seed encoding"))?
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .map_err(|_| fail("invalid Ed25519 private seed encoding"))?
        });
        if decoded.len() != 32 {
            return Err(fail("Ed25519 private seed must contain 32 bytes"));
        }
        seed.copy_from_slice(&decoded);
    }
    Ok(SigningKey::from_bytes(&seed))
}
fn load(reference: &UpdateSigningKey, root: &Path) -> Result<SigningKey, Error> {
    reference.validate()?;
    let bytes = Zeroizing::new(match reference {
        UpdateSigningKey::File { path } => {
            let path = root.join(path);
            let file = fs::File::open(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if file.metadata()?.permissions().mode() & 0o077 != 0 {
                    return Err(fail(
                        "private signing key is accessible to other users; use mode 0600",
                    ));
                }
            }
            let mut bytes = Zeroizing::new(Vec::new());
            file.take(1025).read_to_end(&mut bytes)?;
            return decode(&bytes);
        }
        UpdateSigningKey::Environment { variable } => std::env::var(variable)
            .map_err(|_| fail("update signing environment variable is missing or not UTF-8"))?
            .into_bytes(),
        UpdateSigningKey::Keychain { service, account } => keychain_load(service, account)?,
    });
    if bytes.len() > 1024 {
        return Err(fail("private signing seed exceeds size limit"));
    }
    decode(&bytes)
}
#[cfg(target_os = "macos")]
fn keychain_load(service: &str, account: &str) -> Result<Vec<u8>, Error> {
    use security_framework::passwords::{PasswordOptions, generic_password};
    generic_password(PasswordOptions::new_generic_password(service, account)).map_err(|error| {
        Error::Execution(format!("update signing Keychain lookup failed: {error}"))
    })
}
#[cfg(not(target_os = "macos"))]
fn keychain_load(_: &str, _: &str) -> Result<Vec<u8>, Error> {
    Err(fail("Keychain update signing requires macOS"))
}

pub(crate) fn sign(
    reference: &UpdateSigningKey,
    root: &Path,
    message: &[u8],
) -> Result<Vec<u8>, Error> {
    Ok(load(reference, root)?.sign(message).to_bytes().to_vec())
}

fn temporary(path: &Path) -> Result<tempfile::NamedTempFile, Error> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(tempfile::NamedTempFile::new_in(parent)?)
}
fn write_public(path: &Path, key: &SigningKey) -> Result<(), Error> {
    let mut file = temporary(path)?;
    writeln!(file, "{}", hex::encode(key.verifying_key().as_bytes()))?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}

/// Generates a new update key without overwriting an existing file or Keychain item.
///
/// File seeds are base64 and owner-only on Unix; Windows removes inherited ACLs
/// before writing the seed. Keychain destinations use the default macOS Keychain.
/// The public output is hex, accepted by native update clients and safe to commit.
/// Environment destinations are read-only. Parent directories must exist. If writing the public output fails
/// after storing the seed, recover it with [`export_update_public_key`].
/// # Errors
/// Returns invalid references, existing outputs, OS randomness/storage or I/O errors.
pub fn generate_update_key(
    reference: &UpdateSigningKey,
    public_output: &Path,
) -> Result<(), Error> {
    reference.validate()?;
    if public_output.try_exists()? {
        return Err(fail("public key output already exists"));
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(|_| fail("OS update key generation failed"))?;
    let key = SigningKey::from_bytes(&seed);
    let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(seed.as_slice()));
    match reference {
        UpdateSigningKey::File { path } => {
            let mut file = temporary(path)?;
            #[cfg(windows)]
            restrict_windows_file(file.path())?;
            writeln!(file, "{}", encoded.as_str())?;
            file.as_file().sync_all()?;
            file.persist_noclobber(path)
                .map_err(|error| Error::Io(error.error))?;
        }
        UpdateSigningKey::Keychain { service, account } => {
            keychain_create(service, account, encoded.as_bytes())?
        }
        UpdateSigningKey::Environment { .. } => {
            return Err(fail(
                "environment signing keys are read-only; generate a file and enroll it in your secret manager",
            ));
        }
    }
    write_public(public_output, &key)
}

/// Exports a public key from a private reference without exposing or replacing the seed.
/// # Errors
/// Returns missing/invalid private keys, storage failures or existing public output errors.
pub fn export_update_public_key(
    reference: &UpdateSigningKey,
    root: &Path,
    public_output: &Path,
) -> Result<(), Error> {
    write_public(public_output, &load(reference, root)?)
}

#[cfg(target_os = "macos")]
fn keychain_create(service: &str, account: &str, bytes: &[u8]) -> Result<(), Error> {
    use security_framework::os::macos::keychain::SecKeychain;
    SecKeychain::default()
        .and_then(|keychain| keychain.add_generic_password(service, account, bytes))
        .map_err(|error| {
            Error::Execution(format!(
                "update signing Keychain enrollment failed: {error}"
            ))
        })
}
#[cfg(not(target_os = "macos"))]
fn keychain_create(_: &str, _: &str, _: &[u8]) -> Result<(), Error> {
    Err(fail("Keychain update signing requires macOS"))
}
#[cfg(windows)]
fn restrict_windows_file(path: &Path) -> Result<(), Error> {
    use std::process::Command;
    let identity = Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()?;
    if !identity.status.success() {
        return Err(fail("cannot resolve signing key file owner"));
    }
    // SIDs are ASCII even when the account name uses the console's OEM encoding.
    let output = String::from_utf8_lossy(&identity.stdout);
    let sid = output
        .split('"')
        .find(|value| {
            value.starts_with("S-1-")
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'S' | b'-'))
        })
        .ok_or_else(|| fail("invalid signing key file owner SID"))?;
    let result = Command::new("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r", &format!("*{sid}:F")])
        .output()?;
    if !result.status.success() {
        return Err(fail("cannot restrict private signing key ACL"));
    }
    Ok(())
}
