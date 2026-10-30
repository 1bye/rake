//! Covers: relocated resources/private helpers and path confinement.
//! Real: descriptor decoding and temporary installation filesystem.
//! Doubles: installed files. Requires: no native tools.
use rake_resources::Resources;
use std::fs;

#[test]
fn relocated_installation_resolves_without_the_build_directory() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("rake/helpers")).unwrap();
    fs::write(root.path().join("rake/helpers/media"), "helper").unwrap();
    fs::write(root.path().join("message"), "hello").unwrap();
    fs::write(root.path().join("rake-installation.json"), r#"{"schema_version":1,"identifier":"dev.rake.fixture","version":"1.0.0","target":"aarch64-apple-darwin","platform":"macos","format":"app","update_owner":"disabled","updates":{},"helpers":{"media":"rake/helpers/media"},"runtimes":{}}"#).unwrap();
    let resources = Resources::load(root.path()).unwrap();
    assert_eq!(
        fs::read_to_string(resources.path("message").unwrap()).unwrap(),
        "hello"
    );
    assert_eq!(
        fs::read_to_string(resources.helper("media").unwrap()).unwrap(),
        "helper"
    );
    assert!(resources.path("../outside").is_err());
    assert!(resources.path("C:\\secret").is_err());
    assert!(resources.helper("not-on-PATH").is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", root.path().join("escape")).unwrap();
        assert!(resources.path("escape").is_err());
    }
}
