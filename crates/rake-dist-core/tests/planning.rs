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

#[test]
fn target_and_format_layers_replace_arrays_and_preserve_unset_update_fields() {
    let settings = config(
        json!({"schema_version":1,"sidecars":[{"name":"old","path":"old"}],"updates":{"enabled":true,"base_url":"https://example.com/"},"target_overrides":{"x86_64-unknown-linux-gnu":{"sidecars":[],"formats":["deb","rpm"],"updates":{"channel":"beta"}}},"format_overrides":{"rpm":{"name":"RPM edition","package_version":"9.0~rc1"}}}),
    );
    let plan = settings
        .plan_for(
            "notes",
            "1.2.3-beta.1",
            Platform::Linux,
            Some("x86_64-unknown-linux-gnu"),
            None,
        )
        .unwrap();
    assert!(plan.config().sidecars.is_empty());
    assert_eq!(plan.config().updates.channel.as_deref(), Some("beta"));
    assert!(plan.config().updates.enabled);
    assert_eq!(plan.packages()[0].version().version, "1.2.3~beta.1");
    assert_eq!(plan.packages()[1].version().version, "9.0~rc1");
    assert_eq!(
        plan.packages()[1].config().name.as_deref(),
        Some("RPM edition")
    );
    assert_eq!(plan.packages()[0].update_owner(), UpdateOwner::Apt);
    let selected = settings
        .plan_for(
            "notes",
            "1.2.3",
            Platform::Linux,
            Some("x86_64-unknown-linux-gnu"),
            Some(&[Format::Rpm]),
        )
        .unwrap();
    assert_eq!(selected.packages().len(), 1);
}

#[test]
fn restricted_versions_never_silently_drop_prerelease_ordering() {
    let settings = config(json!({"schema_version":1,"windows":{"formats":["msi"]}}));
    assert!(
        settings
            .plan("notes", "1.0.0-beta.1", Platform::Windows)
            .is_err()
    );
    let explicit = config(
        json!({"schema_version":1,"package_version":"1.0.42","windows":{"formats":["msi"]}}),
    );
    assert_eq!(
        explicit
            .plan("notes", "1.0.0-beta.1", Platform::Windows)
            .unwrap()
            .packages()[0]
            .version()
            .version,
        "1.0.42"
    );
    assert!(
        config(json!({"schema_version":1,"linux":{"formats":["arch"]}}))
            .plan("notes", "1.0.0-rc.1", Platform::Linux)
            .is_err()
    );
}

#[test]
fn target_triples_must_parse_and_agree_with_platform() {
    for target in ["nonsense", "aarch64-apple-darwin", "x86_64-pc-windows-msvc"] {
        assert!(
            Config::default()
                .plan_for("notes", "1.2.3", Platform::Linux, Some(target), None)
                .is_err()
        );
    }
}
