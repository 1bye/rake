//! Covers: platform-independent CLI version output and successful exit.
//! Real: compiled rake binary. Doubles: none. Requires: no runtime services.

#[test]
fn version_reports_the_package_version() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rake"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!("rake ", env!("CARGO_PKG_VERSION"), "\n")
    );
    assert!(output.stderr.is_empty());
}
