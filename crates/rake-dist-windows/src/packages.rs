//! Windows SDK packaging and current-user certificate-store signing.
use crate::Error;
use rake_dist_core::{TrustOptions, TrustRecord};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn run(command: &mut Command) -> Result<String, Error> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(Error::Tool(format!(
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

/// Validates local certificate references before building a trusted release.
/// # Errors
/// Rejects absent/non-hex thumbprints, missing timestamp URLs and store/tool errors.
pub fn preflight(trust: &TrustOptions, timestamp: Option<&str>) -> Result<(), Error> {
    if trust.required && (trust.windows_thumbprint.is_none() || timestamp.is_none()) {
        return Err(Error::Tool("trusted Windows releases require a current-user signing certificate and RFC3161 timestamp URL".into()));
    }
    if let Some(thumbprint) = &trust.windows_thumbprint {
        if thumbprint.len() != 40 || !thumbprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::Tool(
                "certificate thumbprint must contain 40 hex digits".into(),
            ));
        }
        certificate_subject(thumbprint)?;
    }
    if let Some(url) = timestamp {
        let url = url::Url::parse(url).map_err(|e| Error::Tool(e.to_string()))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Tool(
                "timestamp service requires public HTTPS".into(),
            ));
        }
    }
    Ok(())
}

/// Reads a public certificate subject from the existing current-user My store.
/// # Errors
/// Rejects invalid thumbprints and missing/expired certificates or private-key access.
pub fn certificate_subject(thumbprint: &str) -> Result<String, Error> {
    if thumbprint.len() != 40 || !thumbprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Tool("invalid certificate thumbprint".into()));
    }
    let script = format!(
        "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $s=[Security.Cryptography.X509Certificates.X509Store]::new('My','CurrentUser'); try {{ $s.Open([Security.Cryptography.X509Certificates.OpenFlags]::OpenExistingOnly); $found=$s.Certificates.Find([Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint,'{thumbprint}',$false); if ($found.Count -ne 1) {{throw 'signing certificate is unavailable'}}; $c=$found[0]; if (!$c.HasPrivateKey -or $c.NotAfter -lt (Get-Date)) {{throw 'signing certificate is unavailable or expired'}}; [Console]::WriteLine($c.Subject) }} finally {{$s.Close()}}"
    );
    Ok(run(Command::new("powershell.exe").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]))?
    .trim()
    .into())
}

/// Signs with a local store identity and requires native Authenticode verification.
/// # Errors
/// Returns signing/chain verification failures; unsigned development output is explicit.
pub fn sign(
    path: &Path,
    trust: &TrustOptions,
    timestamp: Option<&str>,
) -> Result<TrustRecord, Error> {
    preflight(trust, timestamp)?;
    let Some(thumbprint) = &trust.windows_thumbprint else {
        return Ok(TrustRecord::default());
    };
    let mut command = Command::new("signtool.exe");
    command.args(["sign", "/s", "My", "/sha1", thumbprint, "/fd", "SHA256"]);
    if let Some(timestamp) = timestamp {
        command.args(["/tr", timestamp, "/td", "SHA256"]);
    }
    run(command.arg(dunce::simplified(path)))?;
    run(Command::new("signtool.exe")
        .args(["verify", "/pa", "/all"])
        .arg(dunce::simplified(path)))?;
    Ok(TrustRecord {
        signature: Some("authenticode".into()),
        ..Default::default()
    })
}

/// Managed Windows package identity and visual metadata.
pub struct Msix<'a> {
    /// Stable application/package identity.
    pub identifier: &'a str,
    /// Certificate-matching publisher distinguished name.
    pub publisher: &'a str,
    /// Four-field numeric MSIX version.
    pub version: &'a str,
    /// Rust target triple.
    pub target: &'a str,
    /// Main executable filename relative to the package root.
    pub executable: &'a str,
    /// Human-readable application name.
    pub name: &'a str,
    /// Optional complete application manifest.
    pub manifest: Option<&'a Path>,
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Generates an Appx manifest or validates a complete custom one against identity.
/// # Errors
/// Rejects invalid versions, architectures and conflicting custom identity/executable fields.
pub fn msix_manifest(settings: &Msix<'_>) -> Result<String, Error> {
    if settings.identifier.len() < 3
        || settings.identifier.len() > 50
        || !settings
            .identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
    {
        return Err(Error::Tool(
            "MSIX identity must contain 3–50 letters/digits/dots/dashes".into(),
        ));
    }
    let parts: Vec<_> = settings.version.split('.').collect();
    if parts.len() != 4 || parts.iter().any(|part| part.parse::<u16>().is_err()) {
        return Err(Error::Tool("MSIX version requires four u16 fields".into()));
    }
    let architecture = if settings.target.starts_with("x86_64-") {
        "x64"
    } else if settings.target.starts_with("aarch64-") {
        "arm64"
    } else if settings.target.starts_with("i686-") {
        "x86"
    } else {
        return Err(Error::Tool("unsupported MSIX architecture".into()));
    };
    if let Some(path) = settings.manifest {
        let text = fs::read_to_string(path)?;
        let document = roxmltree::Document::parse(&text).map_err(|e| Error::Tool(e.to_string()))?;
        let identity = document
            .descendants()
            .find(|node| node.has_tag_name("Identity"))
            .ok_or_else(|| Error::Tool("MSIX Identity missing".into()))?;
        for (key, value) in [
            ("Name", settings.identifier),
            ("Publisher", settings.publisher),
            ("Version", settings.version),
            ("ProcessorArchitecture", architecture),
        ] {
            if identity.attribute(key) != Some(value) {
                return Err(Error::Tool(format!(
                    "custom MSIX manifest conflicts with {key}"
                )));
            }
        }
        if !document.descendants().any(|node| {
            node.has_tag_name("Application")
                && node.attribute("Executable") == Some(settings.executable)
        }) {
            return Err(Error::Tool(
                "custom MSIX executable conflicts with main binary".into(),
            ));
        }
        return Ok(text);
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10" xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10" xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities" IgnorableNamespaces="uap rescap">
<Identity Name="{}" Publisher="{}" Version="{}" ProcessorArchitecture="{architecture}"/>
<Properties><DisplayName>{}</DisplayName><PublisherDisplayName>{}</PublisherDisplayName><Logo>Assets\StoreLogo.png</Logo></Properties>
<Resources><Resource Language="en-us"/></Resources>
<Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0"/></Dependencies>
<Applications><Application Id="App" Executable="{}" EntryPoint="Windows.FullTrustApplication"><uap:VisualElements DisplayName="{}" Description="{}" BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png"/></Application></Applications>
<Capabilities><rescap:Capability Name="runFullTrust"/></Capabilities></Package>"#,
        xml(settings.identifier),
        xml(settings.publisher),
        xml(settings.version),
        xml(settings.name),
        xml(settings.name),
        xml(settings.executable),
        xml(settings.name),
        xml(settings.name)
    ))
}

/// Builds a managed package using the Windows SDK with semantic validation enabled.
/// # Errors
/// Requires makeappx.exe and the payload's Assets images.
pub fn package_msix(payload: &Path, output: &Path, settings: &Msix<'_>) -> Result<PathBuf, Error> {
    fs::write(payload.join("AppxManifest.xml"), msix_manifest(settings)?)?;
    let package = output.join(format!("{}.msix", settings.identifier));
    run(Command::new("makeappx.exe")
        .args(["pack", "/o", "/d"])
        .arg(dunce::simplified(payload))
        .arg("/p")
        .arg(dunce::simplified(&package)))?;
    Ok(package)
}

/// Builds Velopack's complete release family with the reviewed native CLI.
/// # Errors
/// Requires vpk 1.2.161 and an executable implementing the Velopack startup contract.
pub fn package_velopack(
    payload: &Path,
    output: &Path,
    identifier: &str,
    version: &str,
    executable: &str,
    channel: &str,
) -> Result<Vec<PathBuf>, Error> {
    let tool = run(Command::new("vpk").arg("--version"))?;
    if !tool.contains("1.2.161") {
        return Err(Error::Tool(
            "vpk 1.2.161 is required; install the pinned dotnet tool".into(),
        ));
    }
    run(Command::new("vpk")
        .args([
            "pack",
            "--packId",
            identifier,
            "--packVersion",
            version,
            "--packDir",
        ])
        .arg(dunce::simplified(payload))
        .args(["--mainExe", executable, "--channel", channel, "--outputDir"])
        .arg(dunce::simplified(output)))?;
    let files = fs::read_dir(output)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    if !files.iter().any(|path| {
        path.extension()
            .is_some_and(|extension| extension == "nupkg")
    }) {
        return Err(Error::Tool("vpk produced no release package".into()));
    }
    Ok(files.into_iter().filter(|path| path.is_file()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn msix_metadata_escapes_text_and_checks_custom_identity() {
        let mut settings = Msix {
            identifier: "dev.rake.fixture",
            publisher: "CN=Rake & Company",
            version: "1.2.3.0",
            target: "x86_64-pc-windows-msvc",
            executable: "fixture.exe",
            name: "Rake & <Media>",
            manifest: None,
        };
        let text = msix_manifest(&settings).unwrap();
        let document = roxmltree::Document::parse(&text).unwrap();
        assert_eq!(
            document
                .descendants()
                .find(|node| node.has_tag_name("Identity"))
                .unwrap()
                .attribute("Publisher"),
            Some(settings.publisher)
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("AppxManifest.xml");
        fs::write(&path, &text).unwrap();
        settings.manifest = Some(&path);
        msix_manifest(&settings).unwrap();
        settings.version = "1.2.4.0";
        assert!(msix_manifest(&settings).is_err());
    }
    #[test]
    fn trusted_windows_release_requires_existing_credentials() {
        assert!(
            preflight(
                &TrustOptions {
                    required: true,
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(
            preflight(
                &TrustOptions {
                    windows_thumbprint: Some("not a thumbprint".into()),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
    }
}
