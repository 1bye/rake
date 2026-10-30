//! Replaces one staged Win32 manifest, preserving GPUI's required declarations.
mod packages;
pub use packages::{
    Msix, certificate_subject, msix_manifest, package_msix, package_velopack, preflight, sign,
    verify_velopack_tool,
};

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
};

/// Native Windows metadata errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input or native tool failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Required compatibility or DPI declarations were removed.
    #[error("custom manifest must retain GPUI declarations: {0}")]
    Manifest(&'static str),
    /// Windows SDK rejected the manifest replacement.
    #[error("Windows distribution tool failed: {0}")]
    Tool(String),
}

/// Validates the required GPUI manifest settings without adding a second resource.
/// # Errors
/// Returns a missing-declaration error. mt.exe performs structural XML validation.
pub fn validate_manifest(xml: &str) -> Result<(), Error> {
    let document = roxmltree::Document::parse(xml).map_err(|_| Error::Manifest("valid XML"))?;
    let checks = [
        ("requestedExecutionLevel", "level", "asInvoker"),
        ("requestedExecutionLevel", "uiAccess", "false"),
        (
            "supportedOS",
            "Id",
            "{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}",
        ),
        (
            "assemblyIdentity",
            "name",
            "Microsoft.Windows.Common-Controls",
        ),
    ];
    for (element, attribute, value) in checks {
        if !document
            .descendants()
            .any(|node| node.has_tag_name(element) && node.attribute(attribute) == Some(value))
        {
            return Err(Error::Manifest(element));
        }
    }
    for (element, value) in [
        ("dpiAwareness", "PerMonitorV2"),
        ("heapType", "SegmentHeap"),
    ] {
        if !document.descendants().any(|node| {
            node.has_tag_name(element) && node.text().is_some_and(|text| text.trim() == value)
        }) {
            return Err(Error::Manifest(element));
        }
    }
    Ok(())
}

/// Replaces manifest resource #1 in an unsigned, staged PE binary and verifies it.
///
/// The application links GPUI's manifest once; this post-link replacement avoids
/// duplicate resource IDs without patching or forking the pinned GPUI dependency.
/// # Errors
/// Requires Windows SDK mt.exe and a valid manifest retaining GPUI declarations.
pub fn apply_manifest(binary: &Path, manifest: &Path) -> Result<(), Error> {
    let xml = fs::read_to_string(manifest)?;
    validate_manifest(&xml)?;
    let xml = native_manifest(&xml, pe_architecture(binary)?)?;
    let directory = tempfile::tempdir()?;
    let manifest = directory.path().join("native.manifest");
    fs::write(&manifest, xml)?;
    let binary = dunce::simplified(binary);
    let manifest = dunce::simplified(&manifest);
    mt(Command::new("mt.exe")
        .arg("-nologo")
        .arg("-manifest")
        .arg(manifest)
        .arg(format!("-outputresource:{};#1", binary.display())))?;
    mt(Command::new("mt.exe")
        .args(["-nologo", "-validate_manifest", "-manifest"])
        .arg(manifest))?;
    let extracted = directory.path().join("embedded.manifest");
    mt(Command::new("mt.exe")
        .arg("-nologo")
        .arg(format!("-inputresource:{};#1", binary.display()))
        .arg(format!("-out:{}", extracted.display())))?;
    validate_manifest(&fs::read_to_string(extracted)?)?;
    Ok(())
}

fn pe_architecture(binary: &Path) -> Result<&'static str, Error> {
    let mut file = fs::File::open(binary)?;
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err(Error::Tool("manifest target is not a PE executable".into()));
    }
    let offset = u32::from_le_bytes(
        dos[60..64]
            .try_into()
            .map_err(|_| Error::Tool("DOS header".into()))?,
    );
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut pe = [0_u8; 6];
    file.read_exact(&mut pe)?;
    if &pe[..4] != b"PE\0\0" {
        return Err(Error::Tool("invalid PE signature".into()));
    }
    match u16::from_le_bytes([pe[4], pe[5]]) {
        0x8664 => Ok("amd64"),
        0xaa64 => Ok("arm64"),
        0x014c => Ok("x86"),
        _ => Err(Error::Tool("unsupported PE processor architecture".into())),
    }
}

fn native_manifest(xml: &str, architecture: &str) -> Result<String, Error> {
    let document = roxmltree::Document::parse(xml).map_err(|_| Error::Manifest("valid XML"))?;
    let identity = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name("assemblyIdentity"))
        .ok_or(Error::Manifest("root assemblyIdentity"))?;
    let mut text = xml.to_owned();
    if let Some(attribute) = identity.attribute_node("processorArchitecture") {
        if attribute.value() == "*" {
            text.replace_range(
                attribute.range(),
                &format!("processorArchitecture=\"{architecture}\""),
            );
        } else if attribute.value() != architecture {
            return Err(Error::Manifest(
                "root processorArchitecture must match executable",
            ));
        }
    }
    Ok(text)
}

fn mt(command: &mut Command) -> Result<(), Error> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(Error::Tool(format!(
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(())
}
/// Writes one generated manifest into the staged executable, preserving icon/version resources.
/// # Errors
/// Requires a valid numeric native version and the Windows SDK.
pub fn apply_default_manifest(binary: &Path, identifier: &str, version: &str) -> Result<(), Error> {
    let fields = version.split(['-', '+']).next().unwrap_or("");
    let mut parts = fields
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| Error::Tool(error.to_string()))?;
    if !(3..=4).contains(&parts.len()) {
        return Err(Error::Tool(
            "native executable version requires 3–4 numeric fields".into(),
        ));
    }
    if parts.len() == 3 {
        parts.push(0);
    }
    let version = parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(".");
    let xml = include_str!("../templates/default.manifest.xml")
        .replace("dev.rake.application", identifier)
        .replace("version=\"1.0.0.0\"", &format!("version=\"{version}\""));
    let directory = tempfile::tempdir()?;
    let manifest = directory.path().join("application.manifest");
    fs::write(&manifest, xml)?;
    apply_manifest(binary, &manifest)
}

#[cfg(test)]
mod tests {
    use super::validate_manifest;
    #[test]
    fn root_architecture_is_native_while_dependency_wildcards_are_preserved() {
        let xml = include_str!("../templates/default.manifest.xml");
        let native = super::native_manifest(xml, "amd64").unwrap();
        let parsed = roxmltree::Document::parse(&native).unwrap();
        let root = parsed
            .root_element()
            .children()
            .find(|node| node.has_tag_name("assemblyIdentity"))
            .unwrap();
        assert_eq!(root.attribute("processorArchitecture"), Some("amd64"));
        let dependency = parsed
            .descendants()
            .find(|node| node.has_tag_name("dependentAssembly"))
            .unwrap()
            .children()
            .find(|node| node.has_tag_name("assemblyIdentity"))
            .unwrap();
        assert_eq!(dependency.attribute("processorArchitecture"), Some("*"));
        assert!(super::native_manifest(&native, "arm64").is_err());
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("fixture.exe");
        let mut header = [0_u8; 70];
        header[..2].copy_from_slice(b"MZ");
        header[60..64].copy_from_slice(&64_u32.to_le_bytes());
        header[64..68].copy_from_slice(b"PE\0\0");
        header[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
        std::fs::write(&binary, header).unwrap();
        assert_eq!(super::pe_architecture(&binary).unwrap(), "amd64");
    }
    #[test]
    fn custom_manifest_retains_gpui_settings_and_rejects_comment_spoofing() {
        let xml = include_str!("../templates/default.manifest.xml");
        validate_manifest(xml).unwrap();
        assert!(validate_manifest(&format!("<assembly><!-- {xml} --></assembly>")).is_err());
        assert!(validate_manifest(&xml.replace("PerMonitorV2", "System")).is_err());
    }
}
