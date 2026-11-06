//! Covers: public API round trips and process environment selection.
//! Real: temporary XDG filesystem and current-executable/AppImage selection.
//! Doubles: none.
//! Requires: Linux; the child receives a temporary config home and no sandbox flags.
#![cfg(target_os = "linux")]

use rake_autostart::{LaunchAtLogin, Status};
use std::{fs, process::Command};

#[test]
fn isolated_xdg_child() {
    let Some(config) = std::env::var_os("RAKE_AUTOSTART_TEST_CONFIG") else {
        return;
    };
    assert_eq!(std::env::var_os("XDG_CONFIG_HOME"), Some(config.clone()));
    let login = LaunchAtLogin::new("dev.rake.consumer")
        .unwrap()
        .args(["--background"])
        .unwrap();
    futures::executor::block_on(async {
        assert_eq!(login.status().await.unwrap(), Status::Disabled);
        assert_eq!(login.enable().await.unwrap(), Status::Enabled);
        assert_eq!(login.enable().await.unwrap(), Status::Enabled);
        assert_eq!(login.status().await.unwrap(), Status::Enabled);
        let text = fs::read_to_string(
            std::path::Path::new(&config).join("autostart/dev.rake.consumer.desktop"),
        )
        .unwrap();
        if let Some(appimage) = std::env::var_os("APPIMAGE") {
            assert!(text.contains(appimage.to_str().unwrap()), "{text}");
        } else {
            assert!(
                text.contains(std::env::current_exe().unwrap().to_str().unwrap()),
                "{text}"
            );
        }
        assert!(text.contains("\"--background\""));
        assert_eq!(login.disable().await.unwrap(), Status::Disabled);
        assert_eq!(login.disable().await.unwrap(), Status::Disabled);
    });
}

#[test]
fn api_uses_temporary_xdg_home_and_appimage_override() {
    let config = tempfile::tempdir().unwrap();
    for appimage in [None, Some("/tmp/An AppImage.AppImage")] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "isolated_xdg_child", "--nocapture"])
            .env("RAKE_AUTOSTART_TEST_CONFIG", config.path())
            .env("XDG_CONFIG_HOME", config.path())
            .env_remove("SNAP")
            .env_remove("APPIMAGE");
        if let Some(appimage) = appimage {
            child.env("APPIMAGE", appimage);
        }
        assert!(
            child.status().unwrap().success(),
            "isolated child {appimage:?}"
        );
    }
}
