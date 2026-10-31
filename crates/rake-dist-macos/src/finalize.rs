//! Native trust finalization occurs after every payload mutation.
use crate::{Error, normalize_loaders};
use rake_dist_core::{Format, TrustOptions, TrustRecord};

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn run(command: &mut Command) -> Result<String, Error> {
    let output = command.output()?;

    if !output.status.success() {
        return Err(Error::Native(format!(
            "native release tool failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into())
}

/// Checks required local signing references before building a release.
/// # Errors
/// Rejects missing identities/profiles and inaccessible native signing tools.
pub fn preflight(trust: &TrustOptions) -> Result<(), Error> {
    if trust.required && (trust.macos_identity.is_none() || trust.notary_profile.is_none()) {
        return Err(Error::Native("trusted macOS releases require a Developer ID identity and notarytool Keychain profile".into()));
    }

    if trust.notary_profile.is_some() && trust.macos_identity.is_none() {
        return Err(Error::Native(
            "notarization requires a Developer ID identity".into(),
        ));
    }

    if let Some(identity) = &trust.macos_identity {
        let identities =
            run(Command::new("security").args(["find-identity", "-v", "-p", "codesigning"]))?;

        if !identities
            .lines()
            .any(|line| line.contains(identity) && line.contains("Developer ID Application:"))
        {
            return Err(Error::Native(
                "requested Developer ID identity is unavailable".into(),
            ));
        }
    }

    if let Some(profile) = &trust.notary_profile {
        run(Command::new("xcrun").args([
            "notarytool",
            "history",
            "--keychain-profile",
            profile,
            "--output-format",
            "json",
        ]))?;
    }

    Ok(())
}

fn sign(path: &Path, trust: &TrustOptions, entitlements: Option<&Path>) -> Result<(), Error> {
    let mut command = Command::new("codesign");

    command
        .args(["--force", "--sign"])
        .arg(trust.macos_identity.as_deref().unwrap_or("-"));

    if trust.macos_identity.is_some() {
        command.args(["--timestamp", "--options", "runtime"]);
    }

    if let Some(path) = entitlements {
        command.arg("--entitlements").arg(path);
    } else {
        // Sparkle's XPC services require their declared sandbox/network entitlements.
        command.arg("--preserve-metadata=entitlements");
    }

    run(command.arg(path))?;

    Ok(())
}

fn wrappers(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), Error> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;

        if entry.file_type()?.is_dir() {
            wrappers(&entry.path(), output)?;

            if entry.path().extension().is_some_and(|extension| {
                matches!(extension.to_str(), Some("framework" | "app" | "xpc"))
            }) {
                output.push(entry.path());
            }
        }
    }

    Ok(())
}

/// Normalizes, signs and strictly verifies an app and every packaged code object.
/// # Errors
/// Returns loader, native signing or verifier failures.
pub fn sign_app(
    app: &Path,
    trust: &TrustOptions,
    entitlements: Option<&Path>,
) -> Result<TrustRecord, Error> {
    preflight(trust)?;
    let inventory = normalize_loaders(app)?;

    for file in inventory {
        sign(&file, trust, None)?;
    }

    let mut nested = Vec::new();

    wrappers(app, &mut nested)?;

    for wrapper in nested {
        sign(&wrapper, trust, None)?;
    }

    sign(app, trust, entitlements)?;
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app))?;

    Ok(TrustRecord {
        signature: Some(
            if trust.macos_identity.is_some() {
                "developer_id"
            } else {
                "ad_hoc"
            }
            .into(),
        ),
        timestamped: false,
        notarized: false,
        stapled: false,
    })
}

/// Submits an app/container, requires acceptance, and validates a stapled ticket.
/// # Errors
/// Requires an existing Keychain profile; failures never degrade to unsigned output.
pub fn notarize(path: &Path, trust: &TrustOptions) -> Result<(), Error> {
    let profile = trust
        .notary_profile
        .as_deref()
        .ok_or_else(|| Error::Native("notary profile missing".into()))?;

    let directory = tempfile::tempdir()?;

    let submission = if path.extension().is_some_and(|extension| extension == "app") {
        let zip = directory.path().join("submission.zip");

        run(Command::new("ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(path)
            .arg(&zip))?;

        zip
    } else {
        path.into()
    };

    let output = run(Command::new("xcrun")
        .args(["notarytool", "submit"])
        .arg(submission)
        .args([
            "--keychain-profile",
            profile,
            "--wait",
            "--output-format",
            "json",
        ]))?;

    let status: serde_json::Value =
        serde_json::from_str(&output).map_err(|e| Error::Native(e.to_string()))?;

    if status["status"] != "Accepted" {
        return Err(Error::Native(format!(
            "Apple notarization rejected payload: {} (submission {})",
            status["status"], status["id"]
        )));
    }

    run(Command::new("xcrun").args(["stapler", "staple"]).arg(path))?;
    run(Command::new("xcrun")
        .args(["stapler", "validate"])
        .arg(path))?;

    Ok(())
}

/// Finalizes App/ZIP/DMG outputs from one signed application payload.
/// # Errors
/// Returns native signing, notarization, archive or artifact verification failures.
pub fn package(
    app: &Path,
    format: Format,
    destination: &Path,
    name: &str,
    trust: &TrustOptions,
    entitlements: Option<&Path>,
) -> Result<(PathBuf, TrustRecord), Error> {
    let mut record = sign_app(app, trust, entitlements)?;

    if trust.notary_profile.is_some() {
        notarize(app, trust)?;
        record.notarized = true;
        record.stapled = true;
    }

    match format {
        Format::App => Ok((app.into(), record)),
        Format::Zip => {
            let output = destination.join(format!("{name}.zip"));

            run(Command::new("ditto")
                .args(["-c", "-k", "--keepParent"])
                .arg(app)
                .arg(&output))?;

            Ok((output, record))
        }
        Format::Dmg => {
            let folder = tempfile::tempdir_in(destination)?;

            run(Command::new("ditto").arg(app).arg(
                folder.path().join(
                    app.file_name()
                        .ok_or_else(|| Error::Native("app filename".into()))?,
                ),
            ))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink("/Applications", folder.path().join("Applications"))?;
            let output = destination.join(format!("{name}.dmg"));

            run(Command::new("hdiutil")
                .args([
                    "create",
                    "-quiet",
                    "-format",
                    "UDZO",
                    "-volname",
                    name,
                    "-srcfolder",
                ])
                .arg(folder.path())
                .arg(&output))?;
            sign(&output, trust, None)?;
            run(Command::new("hdiutil").arg("verify").arg(&output))?;

            if trust.notary_profile.is_some() {
                notarize(&output, trust)?;
            }

            Ok((output, record))
        }
        _ => Err(Error::Native("unsupported macOS format".into())),
    }
}

/// Verifies the artifact after its final copy, including stapled-ticket survival.
/// # Errors
/// Rejects invalid signatures, containers or expected tickets.
pub fn verify_artifact(path: &Path, format: Format, record: &TrustRecord) -> Result<(), Error> {
    match format {
        Format::App | Format::Dmg => {
            run(Command::new("codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(path))?;

            if format == Format::Dmg {
                run(Command::new("hdiutil").arg("verify").arg(path))?;
            }

            if record.stapled {
                run(Command::new("xcrun")
                    .args(["stapler", "validate"])
                    .arg(path))?;
            }
        }
        Format::Zip => {
            let directory = tempfile::tempdir()?;

            run(Command::new("ditto")
                .args(["-x", "-k"])
                .arg(path)
                .arg(directory.path()))?;
            let app = fs::read_dir(directory.path())?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.extension().is_some_and(|extension| extension == "app"))
                .ok_or_else(|| Error::Native("ZIP is missing its signed app".into()))?;

            verify_artifact(&app, Format::App, record)?;
        }
        _ => return Err(Error::Native("unsupported verification format".into())),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_release_cannot_degrade_to_ad_hoc_or_skip_notarization() {
        assert!(
            preflight(&TrustOptions {
                required: true,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            preflight(&TrustOptions {
                notary_profile: Some("missing".into()),
                ..Default::default()
            })
            .is_err()
        );
    }
}
