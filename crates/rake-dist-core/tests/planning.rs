//! Covers: portable defaults, format ownership and invalid boundary data.
//! Real: serde configuration and plan validation.
//! Doubles: none.
//! Requires: no native tools, network or credentials.
use rake_dist_core::{Config, Error, Format, Platform, UpdateOwner};
use serde_json::json;

fn config(value: serde_json::Value) -> Config {
    serde_json::from_value(value).unwrap()
}

#[test]
fn versioned_defaults_preserve_legacy_layouts() {
    assert_eq!(
        Config::default()
            .plan("notes", "1.2.3", Platform::Linux)
            .unwrap()
            .packages()[0]
            .format(),
        Format::Tar
    );
    let new = config(json!({"schema_version":1}));
    assert_eq!(
        new.plan("notes", "1.2.3", Platform::Linux)
            .unwrap()
            .packages()[0]
            .format(),
        Format::Appimage
    );
    assert_eq!(
        new.plan("notes", "1.2.3", Platform::Macos)
            .unwrap()
            .packages()[0]
            .format(),
        Format::Dmg
    );
}

#[test]
fn package_managers_own_updates_and_never_self_replace() {
    let new = config(
        json!({"schema_version":1,"linux":{"formats":["deb","rpm","arch","flatpak"]},"updates":{"enabled":true,"base_url":"https://example.com/updates/"}}),
    );
    let plan = new.plan("notes", "1.2.3", Platform::Linux).unwrap();
    let owners: Vec<_> = plan.packages().iter().map(|p| p.update_owner()).collect();
    assert_eq!(
        owners,
        [
            UpdateOwner::Apt,
            UpdateOwner::RpmRepository,
            UpdateOwner::Pacman,
            UpdateOwner::Flatpak
        ]
    );
}

#[test]
fn rejects_traversal_duplicates_bad_schema_and_wrong_platform() {
    for value in [
        json!({"assets":"../secrets"}),
        json!({"assets":"C:\\secret"}),
        json!({"schema_version":7}),
        json!({"linux":{"formats":["deb","deb"]}}),
        json!({"linux":{"formats":["msix"]}}),
    ] {
        assert!(
            config(value.clone())
                .plan("notes", "1.2.3", Platform::Linux)
                .is_err(),
            "accepted {value}"
        );
    }
    assert!(serde_json::from_value::<Config>(json!({"unrecognized":true})).is_err());
}

#[test]
fn self_managed_updates_require_keys_and_a_valid_https_origin() {
    for base in [
        "http://example.com/",
        "https://",
        "https://password@example.com/",
    ] {
        let settings = config(
            json!({"schema_version":1,"updates":{"enabled":true,"base_url":base,"public_keys":{"sparkle":"keys/sparkle.pub"}}}),
        );
        assert!(
            settings.plan("notes", "1.2.3", Platform::Macos).is_err(),
            "accepted {base}"
        );
    }
    let settings = config(
        json!({"schema_version":1,"updates":{"enabled":true,"base_url":"https://example.com/"}}),
    );
    assert!(matches!(
        settings.plan("notes", "1.2.3", Platform::Macos),
        Err(Error::Field(_))
    ));
}

#[test]
fn windows_channels_and_numeric_version_limits_are_explicit() {
    let settings = config(
        json!({"schema_version":1,"windows":{"formats":["msix","velopack","msi"],"msix":{"publisher":"CN=Example"}},"updates":{"enabled":true,"base_url":"https://example.com/"}}),
    );
    let plan = settings.plan("notes", "1.2.3", Platform::Windows).unwrap();
    let owners: Vec<_> = plan.packages().iter().map(|p| p.update_owner()).collect();
    assert_eq!(
        owners,
        [
            UpdateOwner::AppInstaller,
            UpdateOwner::Velopack,
            UpdateOwner::External
        ]
    );
    assert!(
        settings
            .plan("notes", "256.1.0", Platform::Windows)
            .is_err()
    );
}

#[test]
fn documented_configuration_examples_are_consumable() {
    let text = include_str!("../../../.agents/plans/distribution.md");
    for block in text.split("~~~toml\n").skip(1) {
        let block = block.split("~~~").next().unwrap();
        let parsed: toml::Value = toml::from_str(block).unwrap();
        let value = serde_json::to_value(parsed).unwrap();
        let settings: Config =
            serde_json::from_value(value["package"]["metadata"]["rake"]["bundle"].clone()).unwrap();
        for platform in [Platform::Macos, Platform::Windows, Platform::Linux] {
            settings.plan("media-studio", "1.2.3", platform).unwrap();
        }
    }
}
