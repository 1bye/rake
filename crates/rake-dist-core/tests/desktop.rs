//! External configuration consumers for common native desktop metadata.
use rake_dist_core::{Config, DesktopOverrides, Format, LocalizedMetadata, Overrides, Platform};
const CONFIG: &str = r#"
schema_version=1
identifier='dev.rake.labels'
[desktop]
category='developer_tool'
url_schemes=['rake-notes']
file_associations=[{extensions=['rnote'],name='Rake Notes',mime_type='application/x-rake-note'}]
[desktop.localizations.fr]
name='Notes françaises'
description='Documents & notes'
[macos.privacy]
NSCameraUsageDescription='Capture a document image'
[windows]
formats=['msix']
[windows.msix]
publisher='CN=Rake SDK Test'
"#;
#[test]
fn common_metadata_plans_natively_and_partial_overrides_preserve_unspecified_lists() {
    let mut config: Config = toml::from_str(CONFIG).unwrap();
    for platform in [Platform::Macos, Platform::Windows, Platform::Linux] {
        config.plan("labels", "1.0.0", platform).unwrap();
    }
    config.target_overrides.insert(
        "x86_64-unknown-linux-gnu".into(),
        Overrides {
            desktop: Some(DesktopOverrides {
                localizations: [(
                    "de".into(),
                    LocalizedMetadata {
                        name: Some("Notizen".into()),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let plan = config
        .plan_for(
            "labels",
            "1.0.0",
            Platform::Linux,
            Some("x86_64-unknown-linux-gnu"),
            Some(&[Format::Deb]),
        )
        .unwrap();
    assert_eq!(plan.config().desktop.url_schemes, ["rake-notes"]);
    assert_eq!(plan.config().desktop.localizations.len(), 2);
    assert!(
        config
            .plan_for(
                "labels",
                "1.0.0",
                Platform::Windows,
                Some("x86_64-pc-windows-msvc"),
                Some(&[Format::Nsis])
            )
            .unwrap_err()
            .to_string()
            .contains("localized OS display labels")
    );
}
#[test]
fn metadata_rejects_path_and_desktop_injection_before_native_tools() {
    for replacement in [
        CONFIG.replace("rake-notes", "bad://uri"),
        CONFIG.replace("rnote", "../escape"),
        CONFIG.replace("localizations.fr", "localizations.'../escape'"),
        CONFIG.replace("NSCameraUsageDescription", "CFBundleIdentifier"),
        CONFIG.replace("application/x-rake-note", "bad mime/type"),
    ] {
        let config: Config = toml::from_str(&replacement).unwrap();
        assert!(
            config.plan("labels", "1.0.0", Platform::Macos).is_err(),
            "invalid input {replacement}"
        );
    }
}

#[test]
fn velopack_accepts_common_registrations_without_custom_callbacks() {
    let config: Config = toml::from_str(&CONFIG.replace("formats=['msix']", "formats=['velopack']").replace("[desktop.localizations.fr]\nname='Notes françaises'\ndescription='Documents & notes'\n", "")).unwrap();
    let plan = config
        .plan_for(
            "labels",
            "1.0.0",
            Platform::Windows,
            Some("x86_64-pc-windows-msvc"),
            None,
        )
        .unwrap();
    assert_eq!(plan.packages()[0].format(), Format::Velopack);
    assert_eq!(
        plan.packages()[0].config().desktop.url_schemes,
        ["rake-notes"]
    );
    assert_eq!(
        plan.packages()[0].config().desktop.file_associations[0].extensions,
        ["rnote"]
    );
}
