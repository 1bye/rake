//! Real PE imports and independent helper DLL versions, using the native MSVC toolchain.
#![cfg(all(windows, feature = "native-signing-tests"))]
use rake_dist_windows::{DllDeployment, deploy_dlls, pe_imports};
use std::{fs, path::Path, process::Command};
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
fn compile(root: &Path, value: u32) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("codec.c"),
        format!("__declspec(dllexport) int answer(void){{return {value};}}"),
    )
    .unwrap();
    fs::write(root.join("helper.c"),"#include <stdio.h>\n__declspec(dllimport) int answer(void); int main(void){printf(\"%d\",answer());return 0;}").unwrap();
    run(Command::new("cl").current_dir(root).args([
        "/nologo",
        "/LD",
        "/MT",
        "codec.c",
        "/Fe:rake_codec_fixture.dll",
        "/link",
        "/IMPLIB:codec.lib",
    ]));
    run(Command::new("cl").current_dir(root).args([
        "/nologo",
        "/MT",
        "helper.c",
        "codec.lib",
        "/Fe:helper.exe",
    ]));
}
#[test]
fn relocated_helpers_use_their_own_dll_versions_without_developer_path() {
    let root = tempfile::tempdir().unwrap();
    let one = root.path().join("source-one");
    let two = root.path().join("source-two");
    compile(&one, 1);
    compile(&two, 2);
    assert!(
        pe_imports(&one.join("helper.exe"))
            .unwrap()
            .contains(&"rake_codec_fixture.dll".into())
    );
    assert!(
        deploy_dlls(&DllDeployment {
            executable: &one.join("helper.exe"),
            roots: &[one.clone(), two.clone()],
            destination: &root.path().join("ambiguous"),
            fallback_roots: &[],
            system_dlls: &[]
        })
        .is_err()
    );
    for (source, name) in [(&one, "one"), (&two, "two")] {
        let installed = root.path().join(name);
        fs::create_dir(&installed).unwrap();
        fs::copy(source.join("helper.exe"), installed.join("helper.exe")).unwrap();
        deploy_dlls(&DllDeployment {
            executable: &installed.join("helper.exe"),
            roots: &[source.clone()],
            destination: &installed,
            fallback_roots: &[],
            system_dlls: &[],
        })
        .unwrap();
    }
    fs::remove_dir_all(one).unwrap();
    fs::remove_dir_all(two).unwrap();
    let system = std::env::var_os("SystemRoot").unwrap();
    let path = Path::new(&system).join("System32");
    for (name, value) in [("one", "1"), ("two", "2")] {
        assert_eq!(
            run(Command::new(root.path().join(name).join("helper.exe"))
                .current_dir(std::env::temp_dir())
                .env("PATH", &path)),
            value
        );
    }
}
