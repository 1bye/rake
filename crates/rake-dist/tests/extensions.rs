//! Public Rust backend/recipe consumers and real bounded native command transport.
use rake_dist::{CommandExtension, run_component_recipe, run_package_backend};
use rake_dist_core::{
    BackendOutput, BackendRequest, ComponentRecipe, ExtensionError, PackageBackend,
    PreparedComponent, RecipeRequest,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
struct Backend {
    path: PathBuf,
}
impl PackageBackend for Backend {
    fn package(&self, request: &BackendRequest) -> Result<BackendOutput, ExtensionError> {
        if self.path == Path::new("payload.pkg") {
            fs::write(request.output.join(&self.path), b"native extension payload")?;
        }
        Ok(BackendOutput {
            schema_version: 1,
            artifacts: vec![self.path.clone()],
        })
    }
}
fn request(root: &Path) -> BackendRequest {
    fs::write(root.join("input"), b"input").unwrap();
    fs::create_dir(root.join("output")).unwrap();
    BackendRequest {
        schema_version: 1,
        name: "company-installer".into(),
        identifier: "dev.rake.extension".into(),
        version: "1.0.0".into(),
        target: rake_dist::host_target().unwrap(),
        input: root.join("input"),
        output: root.join("output"),
        tool_references: BTreeMap::new(),
    }
}
#[test]
fn registered_backend_outputs_are_hashed_and_confined() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let checked = run_package_backend(
        &Backend {
            path: "payload.pkg".into(),
        },
        &request,
    )
    .unwrap();
    assert_eq!(checked.len(), 1);
    assert_eq!(checked[0].sha256.len(), 64);
    assert!(
        checked[0]
            .path
            .starts_with(fs::canonicalize(&request.output).unwrap())
    );
    assert!(
        run_package_backend(
            &Backend {
                path: "../input".into()
            },
            &request
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<BackendOutput>(
            r#"{"schema_version":1,"artifacts":["payload.pkg"],"signature":"trusted"}"#
        )
        .is_err()
    );
}
#[cfg(unix)]
#[test]
fn an_escaping_output_symlink_is_rejected_before_native_verification() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    fs::create_dir(request.output.join("tree")).unwrap();
    std::os::unix::fs::symlink(&request.input, request.output.join("tree/escape")).unwrap();
    assert!(
        run_package_backend(
            &Backend {
                path: "tree".into()
            },
            &request
        )
        .is_err()
    );
}
struct Recipe {
    declare: bool,
}
impl ComponentRecipe for Recipe {
    fn prepare(&self, request: &RecipeRequest) -> Result<PreparedComponent, ExtensionError> {
        let root = request.output.join("runtime");
        fs::create_dir(&root)?;
        fs::copy(std::env::current_exe()?, root.join("helper"))?;
        Ok(PreparedComponent {
            schema_version: 1,
            root: "runtime".into(),
            native_code: if self.declare {
                vec!["helper".into()]
            } else {
                vec![]
            },
        })
    }
}
#[test]
fn recipe_inventory_must_declare_the_actual_target_binary() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("first")).unwrap();
    fs::create_dir(root.path().join("second")).unwrap();
    let mut request = RecipeRequest {
        schema_version: 1,
        name: "native-sdk".into(),
        target: rake_dist::host_target().unwrap(),
        input: root.path().into(),
        output: root.path().join("first"),
        options: BTreeMap::new(),
    };
    assert!(run_component_recipe(&Recipe { declare: false }, &request).is_err());
    request.output = root.path().join("second");
    let output = run_component_recipe(&Recipe { declare: true }, &request).unwrap();
    assert!(output.join("helper").is_file());
}
#[cfg(unix)]
#[test]
fn runaway_extension_stdout_is_bounded_and_the_process_is_stopped() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    let argv = vec!["/bin/cat".into(), "/dev/zero".into()];
    let cancelled = AtomicBool::new(false);
    let adapter = CommandExtension::new(&argv, root.path(), &cancelled).unwrap();
    let started = std::time::Instant::now();
    let error = adapter.package(&request).unwrap_err().to_string();
    assert!(error.contains("exceeds bound"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
