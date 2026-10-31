//! Covers: public platform services, preference replacement and bounded file reads.
//! Real: native filesystem, blocking executor and shared service activation inbox.
//! Doubles: none. Requires: no display, GPU or network.

#![cfg(not(target_family = "wasm"))]

use futures::executor::block_on;
use rake::platform::{Error, Platform, Preferences};

#[test]
fn preferences_replace_remove_and_isolate_keys_without_partial_files() {
    let directory = tempfile::tempdir().unwrap();
    let preferences = Preferences::at(directory.path());
    block_on(async {
        assert_eq!(preferences.get("theme").await.unwrap(), None);
        preferences.set("theme", "light").await.unwrap();
        preferences.set("theme", "dark 漢字").await.unwrap();
        preferences.set("other", "kept").await.unwrap();
        assert_eq!(
            preferences.clone().get("theme").await.unwrap().as_deref(),
            Some("dark 漢字")
        );
        preferences.remove("theme").await.unwrap();
        preferences.remove("theme").await.unwrap();
        assert_eq!(preferences.get("theme").await.unwrap(), None);
        assert_eq!(
            preferences.get("other").await.unwrap().as_deref(),
            Some("kept")
        );
        for key in ["", "..", "../outside", "a/b", "a\\b", "a:b"] {
            assert!(matches!(
                preferences.set(key, "bad").await,
                Err(Error::InvalidInput(_))
            ));
        }
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn preference_write_failure_preserves_the_previous_value() {
    let directory = tempfile::tempdir().unwrap();
    let preferences = Preferences::at(directory.path());
    block_on(async {
        preferences.set("kept", "original").await.unwrap();
        std::fs::create_dir(directory.path().join("blocked")).unwrap();
        assert!(matches!(
            preferences.set("blocked", "new").await,
            Err(Error::Io(_))
        ));
        assert_eq!(
            preferences.get("kept").await.unwrap().as_deref(),
            Some("original")
        );
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn service_snapshots_share_the_activation_inbox() {
    let services = rake::Services::from_config(
        rake::http::HttpConfig::new(),
        rake::diagnostics::Collector::default(),
        rake::ServiceRegistry::new(),
    )
    .unwrap();
    let producer = services.platform();
    let consumer = services.registry().standard_services().unwrap().platform();
    assert!(producer.receive_open_url("invalid URL").is_err());
    for n in 0..35 {
        producer
            .receive_open_url(&format!("example://document/{n}"))
            .unwrap();
    }
    assert_eq!(block_on(consumer.next_open_url()).path(), "/3");
    assert_eq!(block_on(producer.next_open_url()).path(), "/4");
    assert!(rake::ServiceRegistry::new().platform().is_err());
}

#[cfg(feature = "gpui")]
#[test]
fn application_service_calls_preserve_activation_identity() {
    let app = rake::App::new("test.platform");
    app.services()
        .unwrap()
        .platform()
        .receive_open_url("test://first")
        .unwrap();
    assert_eq!(
        block_on(app.services().unwrap().platform().next_open_url()).host_str(),
        Some("first")
    );
}

#[cfg(feature = "gpui")]
#[test]
fn file_reads_are_bounded_and_preserve_empty_files() {
    use rake::platform::FileHandle;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("example.txt");
    std::fs::write(&path, b"hello").unwrap();
    let file = FileHandle::from_path(&path);
    assert_eq!(file.name(), "example.txt");
    assert_eq!(block_on(file.read(5)).unwrap(), b"hello");
    assert!(matches!(block_on(file.read(4)), Err(Error::SizeLimit(4))));
    std::fs::write(&path, b"").unwrap();
    assert!(block_on(file.read(0)).unwrap().is_empty());
    assert!(matches!(
        block_on(FileHandle::from_path(directory.path()).read(10)),
        Err(Error::InvalidInput(_))
    ));
    std::fs::remove_file(path).unwrap();
    assert!(matches!(block_on(file.read(10)), Err(Error::Io(_))));
}

#[test]
fn namespaced_preferences_reject_paths_before_accessing_the_host() {
    for invalid in ["", "..", "/absolute", "a/b", "a:b"] {
        assert!(matches!(
            Platform::default().preferences(invalid),
            Err(Error::InvalidInput(_))
        ));
    }
}
