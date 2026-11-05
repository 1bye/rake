//! Covers: exact build revisions across cached Git installs and unpacked sources.
//! Real: build script, Git, Cargo, rustc and filesystem. Doubles: tiny CLI fixture.
//! Requires: installed Git/Rust; no network, GUI, SDK or third-party dependencies.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();

    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    output
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = run(Command::new("git")
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .args([
            "-c",
            "user.name=Rake Test",
            "-c",
            "user.email=rake@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(arguments));

    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn fixture(root: &Path) {
    let package = root.join("crates/rake-cli");

    fs::create_dir_all(package.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers=[\"crates/rake-cli\"]\nresolver=\"3\"\n",
    )
    .unwrap();
    fs::write(package.join("Cargo.toml"), "[package]\nname=\"rake-cli\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[[bin]]\nname=\"revision-probe\"\npath=\"src/main.rs\"\n").unwrap();
    fs::write(
        package.join("src/main.rs"),
        "fn main() { println!(\"{}\", option_env!(\"RAKE_BUILD_REV\").unwrap_or_default()); }\n",
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"),
        package.join("build.rs"),
    )
    .unwrap();
}

#[test]
fn cached_git_install_tracks_new_checkout_and_unpacked_source_has_no_revision() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let repository = root.join("repository");
    let cargo_home = root.join("cargo-home");
    let target = root.join("target");

    fixture(&repository);
    run(Command::new(env!("CARGO"))
        .current_dir(&repository)
        .args(["generate-lockfile", "--offline"]));
    git(&repository, &["init", "--quiet"]);
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "--quiet", "-m", "first"]);
    let first = git(&repository, &["rev-parse", "HEAD"]);

    let url = url::Url::from_directory_path(&repository)
        .unwrap()
        .to_string();

    let install = |number, source: &[&str]| {
        let destination = root.join(format!("install-{number}"));

        run(Command::new(env!("CARGO"))
            .current_dir(&root)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_TARGET_DIR", &target)
            .env("RAKE_BUILD_REV", "ambient-revision-must-not-leak")
            .env("CARGO_NET_OFFLINE", "false")
            .env("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
            .args(["install", "--debug", "--locked", "--root"])
            .arg(&destination)
            .args(source));
        let binary = destination
            .join("bin")
            .join(format!("revision-probe{}", std::env::consts::EXE_SUFFIX));

        let output = run(&mut Command::new(binary));

        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };

    assert_eq!(install(1, &["--git", &url]), first);

    fs::write(repository.join("README.md"), "Only the revision changed.\n").unwrap();
    git(&repository, &["add", "README.md"]);
    git(&repository, &["commit", "--quiet", "-m", "second"]);
    let second = git(&repository, &["rev-parse", "HEAD"]);

    assert_ne!(first, second);
    assert_eq!(install(2, &["--git", &url]), second);

    let unpacked = root.join("unpacked");

    fixture(&unpacked);
    fs::copy(repository.join("Cargo.lock"), unpacked.join("Cargo.lock")).unwrap();
    assert_eq!(
        install(
            3,
            &["--path", unpacked.join("crates/rake-cli").to_str().unwrap()]
        ),
        ""
    );
}
