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
        json!({"schema_version":1,"updates":{"enabled":true,"base_url":"https://example.com/","public_keys":{"velopack":"keys/updates.pub"}}}),
    );
    assert!(matches!(
        settings.plan("notes", "1.2.3", Platform::Macos),
        Err(Error::Field(_))
    ));
}

#[test]
fn windows_channels_and_numeric_version_limits_are_explicit() {
    let settings = config(
        json!({"schema_version":1,"windows":{"formats":["msix","velopack","msi"],"msix":{"publisher":"CN=Example"}},"updates":{"enabled":true,"base_url":"https://example.com/","public_keys":{"velopack":"keys/updates.pub"}}}),
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

#[test]
fn pinned_components_reject_bad_hashes_and_missing_target_sources() {
    let source = json!({"name":"media","target":"x86_64-unknown-linux-gnu","source":{"type":"https","url":"https://example.com/media.zip","sha256":"bad","archive":"zip"}});
    assert!(
        config(json!({"components":[source]}))
            .plan("notes", "1.0.0", Platform::Linux)
            .is_err()
    );
    let settings =
        config(json!({"sidecars":[{"name":"ffmpeg","path":"bin/ffmpeg","component":"media"}]}));
    assert!(
        settings
            .plan_for(
                "notes",
                "1.0.0",
                Platform::Linux,
                Some("x86_64-unknown-linux-gnu"),
                None
            )
            .is_err()
    );
}

#[test]
fn universal_macos_accepts_only_explicit_universal_component_inputs() {
    let settings = config(
        json!({"schema_version":1,"macos":{"universal":true},"sidecars":[{"name":"media","path":"bin/media","component":"media"}],"components":[{"name":"media","target":"universal-apple-darwin","source":{"type":"local","path":"vendor/media"}}]}),
    );
    settings
        .plan_for(
            "notes",
            "1.0.0",
            Platform::Macos,
            Some("aarch64-apple-darwin"),
            None,
        )
        .unwrap();
}

#[test]
fn native_relative_separators_are_canonical_and_cannot_hide_traversal() {
    let settings =
        config(json!({"resources":[{"from":"assets\\message.txt","to":"data\\message.txt"}]}));
    let plan = settings.plan("notes", "1.0.0", Platform::Windows).unwrap();
    assert_eq!(
        plan.config().resources[0].to.to_str().unwrap(),
        "data/message.txt"
    );
    for path in ["one\\..\\secret", "\\\\server\\share", "C:\\secret"] {
        assert!(
            config(json!({"assets":path}))
                .plan("notes", "1.0.0", Platform::Linux)
                .is_err()
        );
    }
    let duplicate =
        config(json!({"resources":[{"from":"a","to":"one\\two"},{"from":"b","to":"one/two"}]}));
    assert!(duplicate.plan("notes", "1.0.0", Platform::Linux).is_err());
}

#[test]
fn system_runtime_policy_is_owned_by_each_linux_format() {
    let raw = json!({"schema_version":1,"runtimes":[{"name":"media","recipe":"gstreamer","system":true,"system_dependencies":{"deb":["libgstreamer1.0-0 (>= 1.22)"],"rpm":["gstreamer1 >= 1.22"]}}],"linux":{"formats":["deb","rpm"],"baseline":{"name":"ubuntu-24.04","glibc":"2.39","kernel":"6.8"}}});
    let plan = config(raw.clone())
        .plan("media_app", "1.0.0", Platform::Linux)
        .unwrap();
    assert_eq!(
        plan.config().linux.package_name.as_deref(),
        Some("media-app")
    );
    assert!(
        config(raw.clone())
            .plan_for(
                "media_app",
                "1.0.0",
                Platform::Linux,
                None,
                Some(&[Format::Appimage])
            )
            .is_err()
    );
    assert!(
        config(raw)
            .plan_for(
                "media_app",
                "1.0.0",
                Platform::Linux,
                None,
                Some(&[Format::Arch])
            )
            .is_err()
    );
    assert!(
        config(json!({"schema_version":1,"linux":{"package_name":"Invalid Name"}}))
            .plan("app", "1.0.0", Platform::Linux)
            .is_err()
    );
}

#[test]
fn sparkle_sdk_baseline_and_universal_policy_are_explicit() {
    let settings = config(
        json!({"schema_version":1,"macos":{"universal":true},"updates":{"enabled":true,"base_url":"https://example.org","public_keys":{"sparkle":"key.pub"}}}),
    );
    let plan = settings
        .plan_for(
            "universal-app",
            "1.0.0",
            Platform::Macos,
            Some("universal-apple-darwin"),
            None,
        )
        .unwrap();
    assert_eq!(
        plan.packages()[0]
            .config()
            .minimum_system_version
            .as_deref(),
        Some("12.0")
    );
    let mut too_old = settings;
    too_old.minimum_system_version = Some("11.0".into());
    assert!(
        too_old
            .plan("universal-app", "1.0.0", Platform::Macos)
            .is_err()
    );
}

#[test]
fn publication_destinations_and_native_feed_paths_are_explicit() {
    use rake_dist_core::{ReleaseConfig, feed_url};
    assert_eq!(
        feed_url(
            "https://github.com/team/app/releases/latest/download",
            "stable",
            "x86_64-pc-windows-msvc",
            "releases.stable.json"
        )
        .unwrap(),
        "https://github.com/team/app/releases/latest/download/stable-x86_64-pc-windows-msvc-releases.stable.json"
    );
    assert!(
        feed_url(
            "https://updates.example.test",
            "../beta",
            "target",
            "appcast.xml"
        )
        .is_err()
    );
    let invalid:ReleaseConfig=serde_json::from_value(json!({"publisher":{"kind":"https","url":"https://user:secret@example.test","public_url":"https://example.test","token_env":"UPLOAD_TOKEN"}})).unwrap();
    assert!(invalid.validate().is_err());
    let wrong: ReleaseConfig =
        serde_json::from_value(json!({"required":{"x86_64-pc-windows-msvc":["dmg"]}})).unwrap();
    assert!(wrong.validate().is_err());
}

#[test]
fn custom_formats_require_explicit_commands_and_selected_inputs() {
    let mut config = config(
        json!({"schema_version":1,"macos":{"formats":["app"]},"custom_formats":[{"name":"company-installer","platform":"macos","input_format":"app","command":["tools/company-packager"],"verify_command":["tools/verify-company-installer"]}],"component_recipes":{"codec":["tools/prepare-codec"]},"runtimes":[{"name":"codec","recipe":"codec","source":"vendor/codec","options":{"profile":"minimal"}}]}),
    );
    assert!(config.plan("example", "1.0.0", Platform::Macos).is_ok());
    config.macos.formats = vec![Format::Dmg];
    assert!(config.plan("example", "1.0.0", Platform::Macos).is_err());
    config.macos.formats = vec![Format::App];
    config.custom_formats[0].command.clear();
    assert!(config.plan("example", "1.0.0", Platform::Macos).is_err());
}
