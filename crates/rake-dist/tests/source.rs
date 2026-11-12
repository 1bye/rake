//! Covers: standalone schema, migration, program selection and confined path provenance.
//! Real: filesystem, Config/schema, locked/offline Cargo metadata and native binary inspection.
//! Doubles: none. Requires: local Cargo/rustc and /usr/bin/true on Unix; no packaging tools.

use rake_dist::{
    BundleSource, Platform, Program, SourceBundleRequest, bundle_file_schema, load_bundle_file,
    plan_source, select_bundle_file,
};

use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn cargo_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();

    fs::create_dir(root.path().join("src")).unwrap();

    for file in ["Cargo.toml", "Cargo.lock", "Bundle.toml", "src/main.rs"] {
        fs::copy(fixture("plain-cargo").join(file), root.path().join(file)).unwrap();
    }

    root
}

fn native_executable() -> PathBuf {
    if cfg!(unix) {
        PathBuf::from("/usr/bin/true")
    } else {
        std::env::current_exe().unwrap()
    }
}

fn prebuilt_fixture() -> (tempfile::TempDir, SourceBundleRequest) {
    let root = tempfile::tempdir().unwrap();

    fs::create_dir(root.path().join("bin")).unwrap();
    let name = if cfg!(windows) {
        "bin/viewer.exe"
    } else {
        "bin/viewer"
    };

    fs::copy(native_executable(), root.path().join(name)).unwrap();
    fs::copy(
        fixture("prebuilt/Bundle.toml"),
        root.path().join("Bundle.toml"),
    )
    .unwrap();
    let source = load_bundle_file(&root.path().join("Bundle.toml")).unwrap();
    let target = rake_dist::host_target().unwrap();
    let mut request = SourceBundleRequest::new(source, Platform::from_target(&target).unwrap());

    request.target = Some(target);

    (root, request)
}

#[test]
fn plain_cargo_selects_package_binary_version_and_features_without_rake() {
    let root = cargo_fixture();
    let source = load_bundle_file(&root.path().join("Bundle.toml")).unwrap();
    let mut request = SourceBundleRequest::new(source, Platform::Linux);

    request.target = Some("x86_64-unknown-linux-gnu".into());
    request.features = vec!["smoke".into()];
    let plan = plan_source(&request).unwrap();
    let value = serde_json::to_value(&plan).unwrap();

    assert_eq!(value["program"]["package"], "plain-cargo-app");
    assert_eq!(value["program"]["bin"], "plain-cargo-app");
    assert_eq!(value["program"]["version"], "1.2.3");
    assert_eq!(value["program"]["profile"], "release");
    assert_eq!(value["program"]["features"], serde_json::json!(["smoke"]));
    assert_eq!(plan.output, request.source.root.join("dist"));
    assert_eq!(
        plan.source.lock_path,
        request.source.root.join("Bundle.lock")
    );
    assert!(!root.path().join("dist").exists());
    request.release = false;
    let debug = serde_json::to_value(plan_source(&request).unwrap()).unwrap();

    assert_eq!(debug["program"]["profile"], "debug");
}

#[test]
fn schema_extends_every_config_property_and_retains_legacy_contract() {
    let schema = bundle_file_schema();
    let legacy = rake_dist::configuration_schema();

    for key in legacy["properties"].as_object().unwrap().keys() {
        assert!(
            schema["properties"].get(key).is_some(),
            "lost Config property {key}"
        );

        if !["name", "identifier", "schema_version"].contains(&key.as_str()) {
            assert_eq!(
                schema["properties"][key], legacy["properties"][key],
                "changed {key}"
            );
        }
    }

    assert!(schema["properties"]["program"].is_object());
    assert!(schema["properties"]["root"].is_object());
    assert!(legacy["properties"].get("program").is_none());
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn current_examples_migrate_to_the_same_config() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut count = 0;

    for entry in fs::read_dir(fixture("migration")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap();

        let original: toml::Value =
            toml::from_str(&fs::read_to_string(examples.join(name).join("Cargo.toml")).unwrap())
                .unwrap();

        let mut expected: rake_dist::Config = original["package"]["metadata"]["rake"]["bundle"]
            .clone()
            .try_into()
            .unwrap();

        // Standalone requires explicit schema-1 identity. Preserve all other fields exactly.
        expected.schema_version = Some(1);
        let package = original["package"]["name"].as_str().unwrap();

        expected.name.get_or_insert_with(|| package.into());
        expected
            .identifier
            .get_or_insert_with(|| format!("dev.rake.{}", package.replace('_', "-")));
        let source = load_bundle_file(&path).unwrap();

        assert_eq!(
            serde_json::to_value(source.config).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "migration {name}"
        );
        count += 1;
    }

    assert_eq!(
        count, 10,
        "cover every current example declaring bundle metadata"
    );
}

#[test]
fn full_config_round_trips_all_nested_sections_without_a_second_config() {
    let root = tempfile::tempdir().unwrap();

    // Existing portable tests maintain broad platform/release/override coverage.
    // Serialize that same Config schema's defaults plus representative nested values.
    let config: rake_dist::Config = toml::from_str(r#"
schema_version=1
name="Full API"
identifier="dev.rake.full"
build_number="12"
package_version="1.2.3"
minimum_system_version="13.0"
resources=[{from="assets/help.txt",to="help.txt"}]
sidecars=[{name="worker",path="bin/worker",component="media"}]
components=[{name="media",target="aarch64-apple-darwin",source={type="https",url="https://example.com/media.zip",sha256="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",archive="zip"},licenses=["LICENSE"]}]
runtimes=[{name="codec",recipe="custom",source="vendor/codec",options={plugins="decoder"}}]
custom_formats=[{name="company-macos",platform="macos",input_format="app",command=["python3","packaging/company.py"],verify_command=["python3","packaging/verify.py"]}]
[component_recipes]
custom=["python3","packaging/codec.py"]
[web]
base_path="/app/"
routing="path"
offline=true
[desktop]
category="developer_tool"
url_schemes=["my-app"]
file_associations=[{extensions=["note"],name="Note",mime_type="application/x-note"}]
[desktop.localizations.fr]
name="Mes notes"
[macos]
formats=["app"]
entitlements="packaging/entitlements.plist"
[macos.privacy]
NSCameraUsageDescription="Capture a document"
[windows.msix]
publisher="CN=Example"
[linux.baseline]
name="ubuntu-24.04"
glibc="2.39"
kernel="6.8"
[linux.flatpak]
runtime="org.freedesktop.Platform"
sdk="org.freedesktop.Sdk"
runtime_version="25.08"
[target_overrides.x86_64-pc-windows-msvc]
formats=["msi"]
[format_overrides.msi]
package_version="1.2.301"
[dependencies.ffmpeg]
path="vendor/{target}"
executables={ffmpeg="bin/ffmpeg{exe}"}
licenses=["LICENSE"]
[updates]
enabled=true
base_url="https://example.com/app"
channel="stable"
[updates.public_keys]
appimage="keys/appimage.pub"
[release]
feed_validity_seconds=604800
sparkle_tool="/tools/generate_appcast"
sparkle_account="ed25519"
gpg_key="0123456789ABCDEF0123456789ABCDEF01234567"
[release.publisher]
kind="directory"
path="release-site"
public_url="https://example.com/app"
[release.required]
x86_64-unknown-linux-gnu=["appimage"]
[release.custom_required]
aarch64-apple-darwin=["company-macos"]
[release.signing_keys.appimage]
kind="environment"
variable="APPIMAGE_SEED"
[release.repositories]
x86_64-unknown-linux-gnu=["deb","rpm"]
"#).unwrap();

    // TOML has no null: serialize Config directly, then append the input envelope.
    let mut text = toml::to_string(&config).unwrap();

    text.push_str("\n[program]\ntype='cargo'\n");
    let path = root.path().join("Bundle.toml");

    fs::write(&path, text).unwrap();
    let source = load_bundle_file(&path).unwrap();

    assert_eq!(
        serde_json::to_value(&source.config).unwrap(),
        serde_json::to_value(&config).unwrap()
    );
}

#[test]
fn typo_diagnostics_name_file_nested_path_and_closest_key() {
    for (addition, needle, suggestion) in [
        ("identifer='typo'\n", "identifer", "identifier"),
        (
            "[macos]\nentitlemnts='app.plist'\n",
            "macos.entitlemnts",
            "entitlements",
        ),
        (
            "[target_overrides.x86_64-unknown-linux-gnu.updates]\nenabeld=true\n",
            "updates.enabeld",
            "enabled",
        ),
        (
            "[[components]]\nname='media'\ntarget='x86_64-unknown-linux-gnu'\nsource={type='local',paht='vendor'}\n",
            "source.paht",
            "path",
        ),
        (
            "[release.signing_keys.appimage]\nkind='environment'\nvarible='KEY'\n",
            "varible",
            "variable",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Typo.toml");

        fs::write(&path, format!("schema_version=1\nname='App'\nidentifier='dev.rake.app'\n{addition}\n[program]\ntype='cargo'\n")).unwrap();
        let error = load_bundle_file(&path).unwrap_err().to_string();

        assert!(
            error.contains("Typo.toml")
                && error.contains(needle)
                && error.contains(&format!("did you mean `{suggestion}`")),
            "{error}"
        );
    }
}

#[test]
fn program_forms_are_exclusive_and_cargo_version_cannot_override_metadata() {
    for program in [
        "type='cargo'\nversion='1.0.0'",
        "type='cargo'\npath='build/app'",
        "type='prebuilt'\npath='app'\nversion='1.0.0'\nfeatures=[]",
        "type='prebuilt'\npath='app'\nversion='1.0.0'\nmanifest='Cargo.toml'",
        "type='prebuilt'\npath='app'\nversion='1.0.0'\nbin='worker'",
        "type='unknown'",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Bundle.toml");

        fs::write(
            &path,
            format!(
                "schema_version=1\nname='App'\nidentifier='dev.rake.app'\n[program]\n{program}\n"
            ),
        )
        .unwrap();
        assert!(load_bundle_file(&path).is_err(), "accepted {program}");
    }
}

#[test]
fn selection_precedence_and_ancestor_root_are_explicit() {
    let root = cargo_fixture();

    fs::create_dir(root.path().join("packaging")).unwrap();
    let mut text = fs::read_to_string(root.path().join("Bundle.toml")).unwrap();

    text.insert_str(0, "root='..'\n");
    fs::write(root.path().join("packaging/Custom.toml"), text).unwrap();
    let source = select_bundle_file(
        root.path(),
        Some(Path::new("packaging/Custom.toml")),
        None,
        None,
        None,
    )
    .unwrap()
    .unwrap();

    assert_eq!(source.root, dunce::canonicalize(root.path()).unwrap());
    assert_eq!(source.lock_path, source.root.join("packaging/Custom.lock"));
    assert!(
        select_bundle_file(root.path(), None, None, None, None)
            .unwrap()
            .is_some()
    );
    assert!(
        select_bundle_file(
            root.path(),
            None,
            Some(Path::new("Cargo.toml")),
            Some("plain-cargo-app"),
            None
        )
        .unwrap()
        .is_none()
    );
    assert!(
        select_bundle_file(
            root.path(),
            Some(Path::new("Bundle.toml")),
            Some(Path::new("Cargo.toml")),
            None,
            None
        )
        .is_err()
    );

    for (package, bin) in [(Some("app"), None), (None, Some("app"))] {
        assert!(select_bundle_file(root.path(), None, None, package, bin).is_err());
    }

    assert!(
        select_bundle_file(&root.path().join("src"), None, None, None, None)
            .unwrap()
            .is_none(),
        "discovery walked ancestors"
    );
    assert!(
        select_bundle_file(
            root.path(),
            Some(Path::new("missing.toml")),
            None,
            None,
            None
        )
        .is_err(),
        "explicit missing config fell back"
    );
}

#[test]
fn shadowed_metadata_is_reported_but_not_merged() {
    let root = cargo_fixture();
    let manifest = root.path().join("Cargo.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();

    text.push_str("\n[package.metadata.rake.bundle]\nname='Shadowed'\nidentifier='dev.rake.shadowed'\nunknown_legacy_key='must not parse'\n");
    fs::write(&manifest, text).unwrap();
    let mut request = SourceBundleRequest::new(
        load_bundle_file(&root.path().join("Bundle.toml")).unwrap(),
        Platform::Linux,
    );

    request.target = Some("x86_64-unknown-linux-gnu".into());
    let plan = plan_source(&request).unwrap();

    assert_eq!(plan.plan.name(), "Plain Cargo App");
    assert_eq!(
        plan.shadowed_metadata,
        vec![dunce::canonicalize(manifest).unwrap()]
    );
}

#[test]
fn cargo_ambiguity_lists_available_binaries_and_packages() {
    let root = cargo_fixture();

    fs::create_dir(root.path().join("src/bin")).unwrap();
    fs::write(root.path().join("src/bin/worker.rs"), "fn main() {}\n").unwrap();
    let mut request = SourceBundleRequest::new(
        load_bundle_file(&root.path().join("Bundle.toml")).unwrap(),
        Platform::Linux,
    );

    request.target = Some("x86_64-unknown-linux-gnu".into());
    let error = plan_source(&request).unwrap_err().to_string();

    assert!(
        error.contains("plain-cargo-app") && error.contains("worker"),
        "{error}"
    );

    if let Program::Cargo { package, bin, .. } = &mut request.source.program {
        *bin = Some("worker".into());
        *package = Some("missing".into());
    }

    let error = plan_source(&request).unwrap_err().to_string();

    assert!(
        error.contains("program.package") && error.contains("plain-cargo-app"),
        "{error}"
    );
}

#[test]
fn prebuilt_requires_target_and_rejects_conflicts_cargo_settings_and_bad_binaries() {
    let (root, request) = prebuilt_fixture();

    assert!(plan_source(&request).is_ok());
    let mut changed = request.clone();

    changed.target = None;
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("requires program.target")
    );

    if let Program::Prebuilt { target, .. } = &mut changed.source.program {
        *target = Some("aarch64-apple-darwin".into());
    }

    changed.target = Some("x86_64-unknown-linux-gnu".into());
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("conflicts")
    );

    for debug in [true, false] {
        let mut changed = request.clone();

        changed.release = !debug;

        if !debug {
            changed.features.push("cargo-only".into());
        }

        assert!(
            plan_source(&changed)
                .unwrap_err()
                .to_string()
                .contains("Cargo-only")
        );
    }

    let mut changed = request.clone();

    changed.source.config.macos.universal = true;
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("universal")
    );
    let mut changed = request.clone();

    changed.target = Some(
        if request.target.as_deref().unwrap().starts_with("aarch64") {
            "x86_64-unknown-linux-gnu"
        } else {
            "aarch64-apple-darwin"
        }
        .into(),
    );
    assert!(
        plan_source(&changed).is_err(),
        "wrong architecture accepted"
    );
    let executable = root.path().join(if cfg!(windows) {
        "bin/viewer.exe"
    } else {
        "bin/viewer"
    });

    fs::write(executable, "not a native executable").unwrap();
    assert!(
        plan_source(&request).is_err(),
        "script accepted as native executable"
    );
}

#[test]
fn output_and_declared_inputs_cannot_escape_or_overlap() {
    let (root, request) = prebuilt_fixture();

    for path in [
        "../outside",
        "/absolute",
        "bin/../../outside",
        "C:\\outside",
        "bin\\..\\outside",
    ] {
        let mut changed = request.clone();

        if let Program::Prebuilt { path: input, .. } = &mut changed.source.program {
            *input = path.into();
        }

        assert!(plan_source(&changed).is_err(), "accepted {path}");
    }

    let mut changed = request.clone();

    changed.output = root.path().to_owned();
    assert!(plan_source(&changed).is_err());
    let mut changed = request.clone();

    changed
        .source
        .config
        .resources
        .push(rake_dist_core::Resource {
            from: "../secret".into(),
            to: "safe".into(),
        });
    assert!(plan_source(&changed).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_escapes_in_inputs_config_lock_and_nested_payloads_fail() {
    use std::os::unix::fs::symlink;
    let (root, request) = prebuilt_fixture();
    let outside = tempfile::tempdir().unwrap();

    fs::copy(
        std::env::current_exe().unwrap(),
        outside.path().join("outside"),
    )
    .unwrap();
    symlink(outside.path().join("outside"), root.path().join("escape")).unwrap();
    let mut changed = request.clone();

    if let Program::Prebuilt { path, .. } = &mut changed.source.program {
        *path = "escape".into();
    }

    assert!(plan_source(&changed).is_err());
    fs::create_dir(root.path().join("assets")).unwrap();
    symlink(
        outside.path().join("outside"),
        root.path().join("assets/escape"),
    )
    .unwrap();
    let mut changed = request.clone();

    changed.source.config.assets = Some("assets".into());
    assert!(plan_source(&changed).is_err());
    symlink(
        root.path().join("Bundle.toml"),
        outside.path().join("Bundle.toml"),
    )
    .unwrap();
    assert!(load_bundle_file(&outside.path().join("Bundle.toml")).is_err());
    symlink(
        outside.path().join("outside"),
        root.path().join("Bundle.lock"),
    )
    .unwrap();
    assert!(plan_source(&request).is_err());
}

#[test]
fn owned_source_round_trip_does_not_reparse_the_live_file() {
    let (root, request) = prebuilt_fixture();
    let bytes = serde_json::to_vec(&request).unwrap();

    fs::write(
        root.path().join("Bundle.toml"),
        "invalid = ['live config changed'",
    )
    .unwrap();
    let request: SourceBundleRequest = serde_json::from_slice(&bytes).unwrap();

    assert!(plan_source(&request).is_ok());
    assert_eq!(request.source.config.name.as_deref(), Some("Native Viewer"));
}

#[test]
fn programmatic_sources_require_canonical_root_and_explicit_lock() {
    let (_root, mut request) = prebuilt_fixture();

    request.source.origin = rake_dist::SourceOrigin::Programmatic;
    request.source.lock_path = "relative.lock".into();
    assert!(
        plan_source(&request)
            .unwrap_err()
            .to_string()
            .contains("lock_path")
    );
    let value = serde_json::to_value(&request.source).unwrap();
    let mut missing = value;

    missing.as_object_mut().unwrap().remove("lock_path");
    assert!(serde_json::from_value::<BundleSource>(missing).is_err());
}

#[test]
fn standalone_required_fields_and_conflicting_worker_selectors_are_rejected() {
    for identity in [
        "name='App'\nidentifier='dev.rake.app'",
        "schema_version=1\nidentifier='dev.rake.app'",
        "schema_version=1\nname='App'",
        "schema_version=2\nname='App'\nidentifier='dev.rake.app'",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Bundle.toml");

        fs::write(&path, format!("{identity}\n[program]\ntype='cargo'\n")).unwrap();
        assert!(load_bundle_file(&path).is_err(), "accepted {identity}");
    }

    let (_root, request) = prebuilt_fixture();

    for (field, value) in [
        ("manifest", "Cargo.toml"),
        ("package", "other"),
        ("binary", "other"),
    ] {
        let mut document = serde_json::to_value(&request).unwrap();

        document[field] = value.into();
        assert!(
            serde_json::from_value::<SourceBundleRequest>(document).is_err(),
            "accepted conflicting {field}"
        );
    }
}

#[test]
fn unsupported_prebuilt_integrations_fail_during_planning() {
    let (_root, request) = prebuilt_fixture();
    let mut changed = request.clone();

    changed.source.config.web.offline = true;
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("Web configuration")
    );
    let mut changed = request.clone();

    changed.source.config.updates.enabled = true;
    changed.source.config.updates.base_url = Some("https://example.com/app".into());
    // Supply a valid public configuration so rejection is about unsupported integration.
    changed.formats = vec![match changed.platform {
        Platform::Linux => rake_dist::Format::Tar,
        Platform::Macos => rake_dist::Format::App,
        _ => rake_dist::Format::Zip,
    }];
    assert!(plan_source(&changed).is_err());
}

#[test]
fn root_must_contain_config_and_cargo_manifest_cannot_traverse() {
    let root = cargo_fixture();
    let config = root.path().join("Bundle.toml");
    let original = fs::read_to_string(&config).unwrap();

    fs::write(&config, format!("root='src'\n{original}")).unwrap();
    assert!(
        load_bundle_file(&config)
            .unwrap_err()
            .to_string()
            .contains("root must contain")
    );
    fs::write(&config, original).unwrap();
    let mut request = SourceBundleRequest::new(load_bundle_file(&config).unwrap(), Platform::Linux);

    request.target = Some("x86_64-unknown-linux-gnu".into());

    if let Program::Cargo { manifest, .. } = &mut request.source.program {
        *manifest = "../Cargo.toml".into();
    }

    assert!(
        plan_source(&request)
            .unwrap_err()
            .to_string()
            .contains("confined relative path")
    );
}

#[test]
fn non_existing_output_parents_are_resolved_without_creating_them() {
    let (root, mut request) = prebuilt_fixture();

    request.output = root.path().join("not-created/../chosen-output");
    let plan = plan_source(&request).unwrap();

    assert_eq!(plan.output, request.source.root.join("chosen-output"));
    assert!(!root.path().join("not-created").exists());
    assert!(!plan.output.exists());
}

#[cfg(unix)]
#[test]
fn prebuilt_input_must_be_an_executable_image_not_a_native_object() {
    use std::os::unix::fs::PermissionsExt;
    let (root, request) = prebuilt_fixture();
    let source = root.path().join("library.rs");
    let path = root.path().join("bin/viewer");

    fs::write(&source, "pub fn entry() {}\n").unwrap();
    assert!(
        std::process::Command::new("rustc")
            .args(["--crate-type=lib", "--emit=obj"])
            .arg(&source)
            .arg("-o")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let error = plan_source(&request).unwrap_err().to_string();

    assert!(error.contains("executable image"), "{error}");
}

#[test]
fn prebuilt_component_cache_cannot_be_payload_or_output() {
    let (root, request) = prebuilt_fixture();
    let cache = root.path().join(".rake-component-cache");

    fs::create_dir(&cache).unwrap();
    fs::write(cache.join("private-cache-entry"), "not application payload").unwrap();

    for from in [
        ".rake-component-cache",
        ".rake-component-cache/private-cache-entry",
    ] {
        let mut changed = request.clone();

        changed
            .source
            .config
            .resources
            .push(rake_dist_core::Resource {
                from: from.into(),
                to: "payload".into(),
            });
        let error = plan_source(&changed).unwrap_err().to_string();

        assert!(error.contains("prebuilt component cache"), "{error}");
    }

    let mut changed = request.clone();

    changed.source.config.assets = Some(".rake-component-cache".into());
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("prebuilt component cache")
    );

    let mut changed = request.clone();

    changed
        .source
        .config
        .components
        .push(rake_dist_core::Component {
            name: "cache-payload".into(),
            target: request.target.clone().unwrap(),
            source: rake_dist_core::ComponentSource::Local {
                path: ".rake-component-cache".into(),
            },
            licenses: Vec::new(),
        });
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("prebuilt component cache")
    );

    let mut changed = request.clone();

    changed.output = cache.join("output");
    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("prebuilt component cache")
    );

    let cached_binary = cache.join(if cfg!(windows) {
        "cached-viewer.exe"
    } else {
        "cached-viewer"
    });

    fs::copy(native_executable(), &cached_binary).unwrap();
    let mut changed = request.clone();

    if let Program::Prebuilt { path, .. } = &mut changed.source.program {
        *path = cached_binary.strip_prefix(root.path()).unwrap().into();
    }

    assert!(
        plan_source(&changed)
            .unwrap_err()
            .to_string()
            .contains("prebuilt component cache")
    );

    #[cfg(unix)]
    {
        // A confined alias can cover the entire root while output lives elsewhere.
        std::os::unix::fs::symlink(".", root.path().join("whole-root")).unwrap();
        let outside = tempfile::tempdir().unwrap();

        for assets in [false, true] {
            let mut changed = request.clone();

            changed.output = outside.path().join("output");

            if assets {
                changed.source.config.assets = Some("whole-root".into());
            } else {
                changed
                    .source
                    .config
                    .resources
                    .push(rake_dist_core::Resource {
                        from: "whole-root".into(),
                        to: "payload".into(),
                    });
            }

            let error = plan_source(&changed).unwrap_err().to_string();

            assert!(error.contains("prebuilt component cache"), "{error}");
        }
    }
    assert!(
        plan_source(&request).is_ok(),
        "ordinary inputs must remain usable with an existing cache"
    );
}

#[cfg(unix)]
#[test]
fn prebuilt_component_cache_must_not_be_a_symlink() {
    let (root, request) = prebuilt_fixture();
    let outside = tempfile::tempdir().unwrap();

    std::os::unix::fs::symlink(outside.path(), root.path().join(".rake-component-cache")).unwrap();
    let error = plan_source(&request).unwrap_err().to_string();

    assert!(error.contains("cache must not be a symlink"), "{error}");
}

#[test]
fn prebuilt_native_separators_are_normalized_before_confinement() {
    let (_root, mut request) = prebuilt_fixture();

    if let Program::Prebuilt { path, .. } = &mut request.source.program {
        *path = if cfg!(windows) {
            "bin\\viewer.exe"
        } else {
            "bin\\viewer"
        }
        .into();
    }

    assert!(plan_source(&request).is_ok());
}
