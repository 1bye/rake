//! Covers: grouped dependency lowering, target overrides and confinement.
//! Real: public configuration parser and cross-platform plans.
//! Doubles: none. Requires: no native tools or filesystem inputs.
use rake_dist_core::{ComponentSource, Config, Platform};
use serde_json::json;

#[test]
fn one_dependency_declaration_supplies_all_native_targets() {
    let config: Config = serde_json::from_value(json!({
        "schema_version":1,
        "dependencies":{"ffmpeg":{
            "path":"vendor/ffmpeg/{target}",
            "executables":{"ffmpeg":"bin/ffmpeg{exe}","ffprobe":"bin/ffprobe{exe}"},
            "licenses":["LICENSE"],
            "resources":{"presets":"share/presets"},
            "libraries":{"path":"lib"}
        }}
    }))
    .unwrap();
    for (platform, target, executable) in [
        (Platform::Macos, "aarch64-apple-darwin", "bin/ffmpeg"),
        (
            Platform::Windows,
            "x86_64-pc-windows-msvc",
            "bin/ffmpeg.exe",
        ),
        (Platform::Linux, "x86_64-unknown-linux-gnu", "bin/ffmpeg"),
    ] {
        let plan = config
            .plan_for("media", "1.0.0", platform, Some(target), None)
            .unwrap();
        let effective = plan.packages()[0].config();
        assert!(effective.dependencies.is_empty(), "lowered once");
        assert_eq!(effective.sidecars[0].path.to_str(), Some(executable));
        assert_eq!(effective.sidecars[0].component.as_deref(), Some("ffmpeg"));
        assert_eq!(effective.runtimes[0].component.as_deref(), Some("ffmpeg"));
        assert_eq!(
            effective.components[0].licenses[0].to_str(),
            Some("LICENSE")
        );
        assert!(
            matches!(&effective.components[0].source, ComponentSource::Local { path } if path.to_str() == Some(&format!("vendor/ffmpeg/{target}")))
        );
        assert_eq!(
            effective.resources[0].from.to_str(),
            Some(format!("vendor/ffmpeg/{target}/share/presets").as_str())
        );
    }
    assert!(config.plan("media", "1.0.0", Platform::Macos).is_err());
}

#[test]
fn overrides_replace_one_dependency_and_invalid_paths_or_collisions_fail() {
    let mut value = json!({"schema_version":1,
        "dependencies":{"media":{"path":"vendor/media","executables":{"ffmpeg":"bin/ffmpeg{exe}"}}},
        "target_overrides":{"aarch64-apple-darwin":{"dependencies":{"media":{"path":"vendor/mac","libraries":{"path":"Media.framework","recipe":"framework"}}}}}
    });
    let plan = |value: &serde_json::Value| {
        serde_json::from_value::<Config>(value.clone())
            .unwrap()
            .plan_for(
                "media",
                "1.0.0",
                Platform::Macos,
                Some("aarch64-apple-darwin"),
                None,
            )
    };
    let selected = plan(&value).unwrap();
    assert!(selected.packages()[0].config().sidecars.is_empty());
    assert_eq!(
        selected.packages()[0].config().runtimes[0].recipe,
        "framework"
    );
    value["target_overrides"] = json!({});
    for path in ["../outside", "C:\\outside", "vendor/{unknown}"] {
        value["dependencies"]["media"]["path"] = json!(path);
        assert!(plan(&value).is_err(), "reject {path}");
    }
    value["dependencies"]["media"]["path"] = json!("vendor");
    value["sidecars"] = json!([{"name":"ffmpeg","path":"other"}]);
    assert!(
        plan(&value).is_err(),
        "helper names cannot silently collide"
    );
}
