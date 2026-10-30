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
        // RFC 3161 authenticates the timestamp token itself; native SignTool
        // and public TSA services also use HTTP transport.
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Tool(
                "timestamp service requires a public HTTP(S) URL without credentials".into(),
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
    let mut verify = Command::new("signtool.exe");
    verify.args(["verify", "/pa", "/all"]);
    if timestamp.is_some() {
        verify.arg("/tw");
    }
    run(verify.arg(dunce::simplified(path)))?;
    Ok(TrustRecord {
        signature: Some("authenticode".into()),
        timestamped: timestamp.is_some(),
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
    msix_manifest_with_metadata(settings, &Default::default())
}
/// Generates managed protocol/file associations and localized resource references.
/// # Errors
/// Rejects invalid native metadata and combining a complete custom manifest with
/// common metadata. Custom manifests retain ownership of their registrations.
pub fn msix_manifest_with_metadata(
    settings: &Msix<'_>,
    metadata: &rake_dist_core::DesktopMetadata,
) -> Result<String, Error> {
    metadata
        .validate()
        .map_err(|error| Error::Tool(error.to_string()))?;
    if settings.manifest.is_some() && !metadata.is_empty() {
        return Err(Error::Tool(
            "custom MSIX manifest owns desktop metadata; omit common desktop fields".into(),
        ));
    }

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
    let mut extensions = String::new();
    for scheme in &metadata.url_schemes {
        extensions.push_str(&format!("<uap:Extension Category=\"windows.protocol\"><uap:Protocol Name=\"{}\"><uap:DisplayName>{}</uap:DisplayName></uap:Protocol></uap:Extension>",xml(&scheme.to_ascii_lowercase()),xml(settings.name)));
    }
    for (index, association) in metadata.file_associations.iter().enumerate() {
        extensions.push_str(&format!("<uap:Extension Category=\"windows.fileTypeAssociation\"><uap:FileTypeAssociation Name=\"type-{index}\"><uap:DisplayName>{}</uap:DisplayName><uap:SupportedFileTypes>",xml(association.name.as_deref().unwrap_or(&association.extensions[0]))));
        for extension in &association.extensions {
            extensions.push_str(&format!(
                "<uap:FileType{}>.{}</uap:FileType>",
                association
                    .mime_type
                    .as_ref()
                    .map(|mime| format!(" ContentType=\"{}\"", xml(mime)))
                    .unwrap_or_default(),
                xml(&extension.to_ascii_lowercase())
            ));
        }
        extensions.push_str("</uap:SupportedFileTypes></uap:FileTypeAssociation></uap:Extension>");
    }
    if !extensions.is_empty() {
        extensions = format!("<Extensions>{extensions}</Extensions>");
    }
    let display_name = if metadata.localizations.is_empty() {
        settings.name
    } else {
        "ms-resource:AppName"
    };
    let description = if metadata.localizations.is_empty() {
        settings.name
    } else {
        "ms-resource:AppDescription"
    };
    let mut languages = std::collections::BTreeSet::from(["en-US".to_owned()]);
    languages.extend(metadata.localizations.keys().map(|language| {
        if language.eq_ignore_ascii_case("en-US") {
            "en-US".into()
        } else {
            language.clone()
        }
    }));
    let resources = languages
        .into_iter()
        .map(|language| format!("<Resource Language=\"{}\"/>", xml(&language)))
        .collect::<String>();
    Ok(format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10" xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10" xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities" IgnorableNamespaces="uap rescap">
<Identity Name="{}" Publisher="{}" Version="{}" ProcessorArchitecture="{architecture}"/>
<Properties><DisplayName>{}</DisplayName><PublisherDisplayName>{}</PublisherDisplayName><Logo>Assets\StoreLogo.png</Logo></Properties>
<Resources>{resources}</Resources>
<Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0"/></Dependencies>
<Applications><Application Id="App" Executable="{}" EntryPoint="Windows.FullTrustApplication"><uap:VisualElements DisplayName="{}" Description="{}" BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png"/>{extensions}</Application></Applications>
<Capabilities><rescap:Capability Name="runFullTrust"/></Capabilities></Package>"#,
        xml(settings.identifier),
        xml(settings.publisher),
        xml(settings.version),
        xml(display_name),
        xml(settings.name),
        xml(settings.executable),
        xml(display_name),
        xml(description)
    ))
}

/// Builds a managed package using the Windows SDK with semantic validation enabled.
/// # Errors
/// Requires makeappx.exe and the payload's Assets images.
pub fn package_msix(payload: &Path, output: &Path, settings: &Msix<'_>) -> Result<PathBuf, Error> {
    package_msix_with_metadata(payload, output, settings, &Default::default())
}
/// Builds SDK-validated managed packages, including optional localized PRI labels.
/// # Errors
/// Returns metadata/SDK failures; localized labels additionally require makepri.exe.
pub fn package_msix_with_metadata(
    payload: &Path,
    output: &Path,
    settings: &Msix<'_>,
    metadata: &rake_dist_core::DesktopMetadata,
) -> Result<PathBuf, Error> {
    fs::write(
        payload.join("AppxManifest.xml"),
        msix_manifest_with_metadata(settings, metadata)?,
    )?;
    if !metadata.localizations.is_empty() {
        let default = rake_dist_core::LocalizedMetadata {
            name: Some(settings.name.into()),
            description: Some(settings.name.into()),
        };
        let mut languages = std::collections::BTreeMap::from([("en-US".to_owned(), default)]);
        for (language, labels) in &metadata.localizations {
            languages.insert(
                if language.eq_ignore_ascii_case("en-US") {
                    "en-US".into()
                } else {
                    language.clone()
                },
                labels.clone(),
            );
        }
        for (language, labels) in languages {
            let directory = payload.join("Strings").join(language);
            fs::create_dir_all(&directory)?;
            fs::write(
                directory.join("Resources.resw"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><root><data name=\"AppName\" xml:space=\"preserve\"><value>{}</value></data><data name=\"AppDescription\" xml:space=\"preserve\"><value>{}</value></data></root>",
                    xml(labels.name.as_deref().unwrap_or(settings.name)),
                    xml(labels.description.as_deref().unwrap_or(settings.name))
                ),
            )?;
        }
        let work = tempfile::tempdir_in(output)?;
        let config = work.path().join("priconfig.xml");
        run(Command::new("makepri.exe")
            .args(["createconfig", "/cf"])
            .arg(dunce::simplified(&config))
            .args(["/dq", "en-US", "/o"]))?;
        let mut configuration = fs::read_to_string(&config)?;
        let document = roxmltree::Document::parse(&configuration)
            .map_err(|error| Error::Tool(error.to_string()))?;
        if let Some(packaging) = document
            .descendants()
            .find(|node| node.has_tag_name("packaging"))
        {
            let range = packaging.range();
            configuration.replace_range(range, "");
        }
        fs::write(&config, configuration)?;
        run(Command::new("makepri.exe")
            .args(["new", "/pr"])
            .arg(dunce::simplified(payload))
            .arg("/cf")
            .arg(dunce::simplified(&config))
            .arg("/of")
            .arg(dunce::simplified(&payload.join("resources.pri")))
            .args(["/in", settings.identifier, "/o"]))?;
    }

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
    verify_velopack_tool()?;
    run(Command::new("vpk")
        .args([
            "--skip-updates",
            "--yes",
            "--legacyConsole",
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
    fn timestamp_transport_accepts_native_http_and_rejects_other_schemes_and_credentials() {
        for url in ["http://timestamp.digicert.com", "https://tsa.example.test"] {
            preflight(&TrustOptions::default(), Some(url)).unwrap();
        }
        for url in [
            "file:///tmp/token",
            "ftp://tsa.example.test",
            "http://user:secret@tsa.example.test",
            "https://tsa.example.test/#fragment",
        ] {
            assert!(preflight(&TrustOptions::default(), Some(url)).is_err());
        }
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

fn pinned_vpk_help(value: &str) -> bool {
    let mut plain = String::new();
    let mut escape = false;
    for character in value.chars() {
        if character == '\u{1b}' {
            escape = true;
            continue;
        }
        if escape {
            if character.is_ascii_alphabetic() {
                escape = false;
            }
            continue;
        }
        plain.push(character);
    }
    plain.split_whitespace().any(|word| {
        word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.') == "1.2.161"
    })
}
#[cfg(test)]
mod vpk_tests {
    #[test]
    fn pinned_version_accepts_ansi_console_help_but_not_another_release() {
        assert!(super::pinned_vpk_help(
            "Velopack CLI 1.2.161, for distributing applications."
        ));
        assert!(super::pinned_vpk_help(
            "\u{1b}[32mVelopack CLI\u{1b}[0m \u{1b}[1m1.2.161\u{1b}[0m"
        ));
        assert!(!super::pinned_vpk_help("Velopack CLI 11.2.161"));
        assert!(!super::pinned_vpk_help("Velopack CLI 1.2.1610"));
    }
}

/// Checks the pinned native tool before starting an expensive application build.
/// # Errors
/// Returns missing tool, unsuccessful help or an unexpected installed version.
pub fn verify_velopack_tool() -> Result<(), Error> {
    let help = Command::new("vpk").arg("--help").output()?;
    let tool = format!(
        "{} {}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    if !help.status.success() {
        return Err(Error::Tool(tool));
    }
    if !pinned_vpk_help(&tool) {
        return Err(Error::Tool(format!(
            "vpk 1.2.161 is required; observed help: {}",
            tool.chars().take(500).collect::<String>()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod desktop_tests {
    use super::*;
    #[test]
    fn managed_registrations_and_localized_references_are_semantic_xml() {
        let settings = Msix {
            identifier: "dev.rake.notes",
            publisher: "CN=Rake",
            version: "1.0.0.0",
            target: "x86_64-pc-windows-msvc",
            executable: "notes.exe",
            name: "Rake & Notes",
            manifest: None,
        };
        let metadata = rake_dist_core::DesktopMetadata {
            url_schemes: vec!["rake-notes".into()],
            file_associations: vec![rake_dist_core::FileAssociation {
                extensions: vec!["rnote".into()],
                name: Some("Note & document".into()),
                description: None,
                mime_type: Some("application/x-rake-note".into()),
            }],
            localizations: [("fr".into(), Default::default())].into(),
            ..Default::default()
        };
        let xml = msix_manifest_with_metadata(&settings, &metadata).unwrap();
        let document = roxmltree::Document::parse(&xml).unwrap();
        assert!(
            document
                .descendants()
                .any(|node| node.has_tag_name("Protocol")
                    && node.attribute("Name") == Some("rake-notes"))
        );
        assert!(
            document
                .descendants()
                .any(|node| node.has_tag_name("FileType") && node.text() == Some(".rnote"))
        );
        assert!(
            document
                .descendants()
                .any(|node| node.has_tag_name("Resource")
                    && node.attribute("Language") == Some("fr"))
        );
        assert!(
            document
                .descendants()
                .any(|node| node.has_tag_name("VisualElements")
                    && node.attribute("DisplayName") == Some("ms-resource:AppName"))
        );
    }
}
