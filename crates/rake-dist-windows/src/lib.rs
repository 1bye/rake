//! Replaces one staged Win32 manifest, preserving GPUI's required declarations.
use std::{fs, path::Path, process::Command};

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
    #[error("mt.exe failed to update or verify resource #1")]
    Tool,
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
    validate_manifest(&fs::read_to_string(manifest)?)?;
    if !Command::new("mt.exe")
        .arg("-nologo")
        .arg("-manifest")
        .arg(manifest)
        .arg(format!("-outputresource:{};#1", binary.display()))
        .status()?
        .success()
    {
        return Err(Error::Tool);
    }
    if !Command::new("mt.exe")
        .args(["-nologo", "-validate_manifest", "-manifest"])
        .arg(manifest)
        .status()?
        .success()
    {
        return Err(Error::Tool);
    }
    let directory = tempfile::tempdir()?;
    let extracted = directory.path().join("embedded.manifest");
    if !Command::new("mt.exe")
        .arg("-nologo")
        .arg(format!("-inputresource:{};#1", binary.display()))
        .arg(format!("-out:{}", extracted.display()))
        .status()?
        .success()
    {
        return Err(Error::Tool);
    }
    validate_manifest(&fs::read_to_string(extracted)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_manifest;
    #[test]
    fn custom_manifest_retains_gpui_settings_and_rejects_comment_spoofing() {
        let xml = include_str!("../../rake-build/templates/windows.manifest.xml");
        validate_manifest(xml).unwrap();
        assert!(validate_manifest(&format!("<assembly><!-- {xml} --></assembly>")).is_err());
        assert!(validate_manifest(&xml.replace("PerMonitorV2", "System")).is_err());
    }
}
