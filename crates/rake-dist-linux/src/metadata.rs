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

/// Generates common URL/MIME registrations and localized freedesktop labels.
/// # Errors
/// Rejects malformed common metadata and unsafe executable/icon identifiers.
pub fn desktop_entry_with_metadata(
    name: &str,
    binary: &str,
    icon: &str,
    metadata: &rake_dist_core::DesktopMetadata,
) -> Result<String, Error> {
    metadata
        .validate()
        .map_err(|error| Error::Metadata(error.to_string()))?;
    let mut output = desktop_entry(name, binary, icon)?;
    if let Some(category) = metadata.category {
        output = output.replace(
            "Categories=Utility;",
            &format!("Categories={}", category.linux_categories()),
        );
    }
    let mut mime = std::collections::BTreeSet::new();
    for scheme in &metadata.url_schemes {
        mime.insert(format!("x-scheme-handler/{}", scheme.to_ascii_lowercase()));
    }
    for association in &metadata.file_associations {
        mime.insert(association_mime(icon, association));
    }
    if !mime.is_empty() {
        output.push_str(&format!(
            "MimeType={};\n",
            mime.into_iter().collect::<Vec<_>>().join(";")
        ));
    }
    for (locale, labels) in &metadata.localizations {
        let locale = locale.replace('-', "_");
        if let Some(name) = &labels.name {
            output.push_str(&format!("Name[{locale}]={}\n", name.replace('\\', "\\\\")));
        }
        if let Some(description) = &labels.description {
            output.push_str(&format!(
                "Comment[{locale}]={}\n",
                description.replace('\\', "\\\\")
            ));
        }
    }
    Ok(output)
}
fn association_mime(identifier: &str, association: &rake_dist_core::FileAssociation) -> String {
    association
        .mime_type
        .clone()
        .unwrap_or_else(|| format!("application/x-{}-{}", identifier, association.extensions[0]))
}
/// Generates shared-mime-info globs for the configured file suffixes.
/// # Errors
/// Rejects malformed common metadata before escaping it into XML.
pub fn mime_types(
    identifier: &str,
    metadata: &rake_dist_core::DesktopMetadata,
) -> Result<String, Error> {
    metadata
        .validate()
        .map_err(|error| Error::Metadata(error.to_string()))?;
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">",
    );
    for association in &metadata.file_associations {
        output.push_str(&format!(
            "<mime-type type=\"{}\"><comment>{}</comment>",
            xml(&association_mime(identifier, association)),
            xml(association
                .description
                .as_deref()
                .or(association.name.as_deref())
                .unwrap_or(&association.extensions[0]))
        ));
        for extension in &association.extensions {
            output.push_str(&format!("<glob pattern=\"*.{}\"/>", xml(extension)));
        }
        output.push_str("</mime-type>");
    }
    output.push_str("</mime-info>");
    Ok(output)
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

/// Generates AppStream with localized labels, categories and MIME registrations.
/// # Errors
/// Rejects malformed common metadata.
pub fn appstream_with_metadata(
    identifier: &str,
    name: &str,
    binary: &str,
    version: &str,
    metadata: &rake_dist_core::DesktopMetadata,
) -> Result<String, Error> {
    metadata
        .validate()
        .map_err(|error| Error::Metadata(error.to_string()))?;
    let mut localized = String::new();
    for (locale, labels) in &metadata.localizations {
        if let Some(name) = &labels.name {
            localized.push_str(&format!(
                "<name xml:lang=\"{}\">{}</name>",
                xml(locale),
                xml(name)
            ));
        }
        if let Some(description) = &labels.description {
            localized.push_str(&format!(
                "<summary xml:lang=\"{}\">{}</summary>",
                xml(locale),
                xml(description)
            ));
        }
    }
    if let Some(category) = metadata.category {
        localized.push_str("<categories>");
        for category in category
            .linux_categories()
            .split(';')
            .filter(|value| !value.is_empty())
        {
            localized.push_str(&format!("<category>{category}</category>"));
        }
        localized.push_str("</categories>");
    }
    if !metadata.file_associations.is_empty() {
        localized.push_str("<provides>");
        for association in &metadata.file_associations {
            localized.push_str(&format!(
                "<mediatype>{}</mediatype>",
                xml(&association_mime(binary, association))
            ));
        }
        localized.push_str("</provides>");
    }
    Ok(appstream(identifier, name, binary, version)
        .replace("</component>", &format!("{localized}</component>")))
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

#[cfg(test)]
mod registration_tests {
    use super::*;
    #[test]
    fn generated_desktop_appstream_and_mime_files_agree_and_escape_localized_labels() {
        let metadata = rake_dist_core::DesktopMetadata {
            category: Some(rake_dist_core::ApplicationCategory::DeveloperTool),
            url_schemes: vec!["rake-note".into()],
            file_associations: vec![rake_dist_core::FileAssociation {
                extensions: vec!["rnote".into()],
                name: Some("Notes & projects".into()),
                description: None,
                mime_type: None,
            }],
            localizations: [(
                "pt-PT".into(),
                rake_dist_core::LocalizedMetadata {
                    name: Some("Notas".into()),
                    description: Some("Notas & projetos".into()),
                },
            )]
            .into(),
        };
        let desktop = desktop_entry_with_metadata("Notes", "notes", "notes", &metadata).unwrap();
        assert!(desktop.contains("Categories=Development;"));
        assert!(desktop.contains("Name[pt_PT]=Notas"));
        assert!(desktop.contains("application/x-notes-rnote;x-scheme-handler/rake-note;"));
        let mime = mime_types("notes", &metadata).unwrap();
        let document = roxmltree::Document::parse(&mime).unwrap();
        assert!(
            document
                .descendants()
                .any(|node| node.attribute("pattern") == Some("*.rnote"))
        );
        assert!(mime.contains("Notes &amp; projects"));
        let appstream =
            appstream_with_metadata("dev.rake.notes", "Notes", "notes", "1.0.0", &metadata)
                .unwrap();
        validate_appstream(&appstream, "dev.rake.notes", "notes").unwrap();
        assert!(appstream.contains("xml:lang=\"pt-PT\""));
        assert!(appstream.contains("application/x-notes-rnote"));
    }
}
