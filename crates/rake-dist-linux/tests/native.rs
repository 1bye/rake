//! Covers: native ELF relocation, baseline rejection and real Debian tool consumption.
//! Real: C compiler, readelf, patchelf, dpkg-deb. Requires: Linux native-tests feature.
#![cfg(all(target_os = "linux", feature = "native-tests"))]
use rake_dist_linux::{ArchPackage, build_deb, normalize_elf};
use std::{fs, process::Command};

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();

    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn relocated_elf_and_debian_archive_run_without_the_original_library() {
    let root = tempfile::tempdir().unwrap();
    let payload = root.path().join("payload");
    let bin = payload.join("usr/bin");
    let lib = payload.join("usr/lib/rake-native-fixture");

    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&lib).unwrap();
    let source = root.path().join("source");

    fs::create_dir(&source).unwrap();
    fs::write(source.join("lib.c"), "int answer(void){return 42;}\n").unwrap();
    fs::write(source.join("main.c"),"#include <stdio.h>\nextern int answer(void); int main(void){printf(\"%d\\n\",answer());return 0;}\n").unwrap();
    run(Command::new("cc")
        .args(["-shared", "-fPIC", "-Wl,-soname,librakefixture.so"])
        .arg(source.join("lib.c"))
        .arg("-o")
        .arg(source.join("librakefixture.so")));
    run(Command::new("cc")
        .arg(source.join("main.c"))
        .arg(format!("-L{}", source.display()))
        .arg(format!("-Wl,-rpath,{}", source.display()))
        .arg("-lrakefixture")
        .arg("-o")
        .arg(bin.join("rake-native-fixture")));
    fs::copy(
        source.join("librakefixture.so"),
        lib.join("librakefixture.so"),
    )
    .unwrap();
    let report = normalize_elf(&payload, &Default::default(), None).unwrap();

    assert_eq!(report.binaries, 2);
    let too_old = rake_dist_core::LinuxBaseline {
        name: "old".into(),
        glibc: Some("1.0".into()),
        kernel: None,
    };

    assert!(normalize_elf(&payload, &Default::default(), Some(&too_old)).is_err());
    fs::remove_dir_all(&source).unwrap();
    assert_eq!(
        run(Command::new(bin.join("rake-native-fixture"))
            .current_dir("/")
            .env_remove("LD_LIBRARY_PATH"))
        .trim(),
        "42"
    );
    let package = build_deb(&ArchPackage {
        name: "rake-native-fixture",
        version: "1.0.0~beta.1",
        target: if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-gnu"
        } else {
            "x86_64-unknown-linux-gnu"
        },
        display_name: "Rake native fixture",
        dependencies: &["libc6 (>= 2.34)".into()],
        payload: &payload,
        output: &root.path().join("result"),
    })
    .unwrap();

    assert_eq!(
        run(Command::new("dpkg-deb")
            .arg("--field")
            .arg(&package)
            .arg("Version"))
        .trim(),
        "1.0.0~beta.1"
    );
    let extracted = root.path().join("extracted");

    run(Command::new("dpkg-deb")
        .arg("--extract")
        .arg(package)
        .arg(&extracted));
    assert_eq!(
        run(Command::new(extracted.join("usr/bin/rake-native-fixture"))
            .current_dir("/")
            .env_remove("LD_LIBRARY_PATH"))
        .trim(),
        "42"
    );
}
