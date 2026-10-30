//! Desktop and AppStream metadata share the validated application identity.
use crate::Error;
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Generates an executable desktop entry with freedesktop placeholders.
/// # Errors
/// Rejects control characters and unsafe executable/icon identifiers.
pub fn desktop_entry(name: &str, binary: &str, icon: &str) -> Result<String, Error> {
    if name.chars().any(char::is_control)
        || [binary, icon].iter().any(|value| {
            value.is_empty()
                || !value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                })
        })
    {
        return Err(Error::Metadata("desktop name/executable/icon".into()));
    }
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={binary} %U\nIcon={icon}\nTerminal=false\nCategories=Utility;\nStartupNotify=true\n",
        name.replace('\\', "\\\\")
    ))
}

/// Generates minimal AppStream metadata; application licensing remains explicit.
pub fn appstream(identifier: &str, name: &str, binary: &str, version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<component type=\"desktop-application\"><id>{}</id><metadata_license>CC0-1.0</metadata_license><name>{}</name><summary>{} desktop application</summary><description><p>{} desktop application.</p></description><launchable type=\"desktop-id\">{}.desktop</launchable><releases><release version=\"{}\"/></releases></component>\n",
        xml(identifier),
        xml(name),
        xml(name),
        xml(name),
        xml(binary),
        xml(version)
    )
}

/// Ensures a custom AppStream document retains the generated application identity.
/// # Errors
/// Rejects malformed XML, conflicting IDs and missing desktop launchers.
pub fn validate_appstream(text: &str, identifier: &str, binary: &str) -> Result<(), Error> {
    let document =
        roxmltree::Document::parse(text).map_err(|error| Error::Metadata(error.to_string()))?;
    if !document
        .descendants()
        .any(|node| node.has_tag_name("id") && node.text() == Some(identifier))
    {
        return Err(Error::Metadata(
            "AppStream ID conflicts with application".into(),
        ));
    }
    if !document.descendants().any(|node| {
        node.has_tag_name("launchable") && node.text() == Some(&format!("{binary}.desktop"))
    }) {
        return Err(Error::Metadata(
            "AppStream launcher conflicts with executable".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integration_metadata_keeps_identity_and_escapes_text() {
        let text = appstream("dev.rake.app", "Rake & Media", "rake-media", "1.2.3");
        validate_appstream(&text, "dev.rake.app", "rake-media").unwrap();
        assert!(validate_appstream(&text, "dev.rake.other", "rake-media").is_err());
        assert!(desktop_entry("bad\nExec=other", "rake-media", "rake-media").is_err());
    }
}
