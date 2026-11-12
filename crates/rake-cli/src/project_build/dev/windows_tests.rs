//! Real Windows Jobs, named pipes, private checkpoints, reloads and detached openers.
use super::*;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        TerminateProcess, WaitForSingleObject,
    },
};

pub(super) struct Fixture {
    pub(super) root: tempfile::TempDir,
    pub(super) binary: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let name = format!("win-supervisor-{}", uuid::Uuid::new_v4().simple());
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace.join("target"));
        let quoted = |path: PathBuf| serde_json::to_string(&path).unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(
            root.path().join("src/main.rs"),
            include_str!("../../../tests/fixtures/windows-supervisor.rs"),
        )
        .unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            format!(
                r#"[package]
name = "{name}"
version = "0.0.0"
edition = "2024"
[dependencies]
rake-client = {{ path = {} }}
rake-protocol = {{ path = {} }}
serde_json = "1"
futures = "0.3"
async-io = "2.6"
uuid = {{ version = "1", features = ["v4"] }}
"#,
                quoted(workspace.join("crates/rake-client")),
                quoted(workspace.join("crates/rake-protocol"))
            ),
        )
        .unwrap();
        fs::copy(workspace.join("Cargo.lock"), root.path().join("Cargo.lock")).unwrap();
        let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["build", "--offline", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let binary = target.join("debug").join(format!("{name}.exe"));
        assert!(binary.is_file());

        Self { root, binary }
    }

    pub(super) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(args);
        command
    }
}

pub(super) struct Process(OwnedHandle);

impl Process {
    pub(super) fn observe(pid: u32) -> Self {
        // SAFETY: pid comes only from a process created by this test or its fixture.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        assert!(
            !handle.is_null(),
            "cannot observe fixture PID {pid}: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: OpenProcess transferred ownership of this valid handle.
        Self(unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    pub(super) fn stopped(&self) {
        // SAFETY: the owned handle remains valid for the bounded wait.
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "owned process survived shutdown"
        );
    }

    fn running(&self) {
        // SAFETY: zero timeout only observes this owned handle.
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) },
            WAIT_TIMEOUT,
            "browser opener was terminated with the session"
        );
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: only our fixture's original process handle is terminated, with a bounded reap.
        unsafe {
            TerminateProcess(self.0.as_raw_handle(), 1);
            WaitForSingleObject(self.0.as_raw_handle(), 5000);
        }
    }
}

pub(super) fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture marker missing: {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn project(root: &Path) -> Project {
    Project {
        package: json!({"name":"windows-fixture","features":{},"dependencies":[]}),
        root: root.into(),
        target: "fixture".into(),
        watch_roots: vec![root.into()],
        web_dir: root.join("web"),
        assets: None,
        bindgen: None,
    }
}

fn instance(supervisor: &Supervisor<'_>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(instance) = supervisor.instance() {
            return instance;
        }
        assert!(
            Instant::now() < deadline,
            "application did not advertise its endpoint"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn descendant(supervisor: &Supervisor<'_>) -> Process {
    let pid = supervisor.app.as_ref().unwrap().0.id();
    let path = supervisor.session.join(format!("descendant-{pid}"));
    wait_for(&path);
    Process::observe(fs::read_to_string(path).unwrap().parse().unwrap())
}

#[test]
fn desktop_reload_and_stop_clean_jobs_but_never_kill_browser_openers() {
    let fixture = Fixture::new();
    let directory = super::super::windows::temporary("rake-win-session-").unwrap();
    let args = Args {
        no_open: true,
        json: true,
        ..Args::default()
    };
    let cancelled = AtomicBool::new(false);
    let mut supervisor = Supervisor::new(
        &args,
        project(fixture.root.path()),
        directory.path().into(),
        &cancelled,
    )
    .unwrap();
    supervisor.publish(1, &fixture.binary).unwrap();
    let first = instance(&supervisor);
    let old_app = Process::observe(supervisor.app.as_ref().unwrap().0.id());
    let old_child = descendant(&supervisor);
    let browser =
        Process::observe(super::super::browser::launch(&mut fixture.command(&["--leaf"])).unwrap());
    assert_eq!(
        supervisor.query(&first, json!({"type":"reload"})).unwrap()["generation"],
        1
    );
    supervisor.publish(2, &fixture.binary).unwrap();
    old_app.stopped();
    old_child.stopped();
    let second = instance(&supervisor);
    assert_ne!(first["info"]["runtime_id"], second["info"]["runtime_id"]);
    let reload = supervisor.query(&second, json!({"type":"reload"})).unwrap();
    assert_eq!(reload["generation"], 2);
    assert_eq!(reload["phase"], "restored");
    let app = Process::observe(supervisor.app.as_ref().unwrap().0.id());
    let child = descendant(&supervisor);
    supervisor.finish().unwrap();
    app.stopped();
    child.stopped();
    browser.running();
    let status = read_json(&supervisor.session.join("status.json")).unwrap();
    assert_eq!(status["phase"], "stopped");
    assert_eq!(fs::read_dir(&supervisor.control).unwrap().count(), 0);
    let output = read_json(&supervisor.session.join("output.json")).unwrap();
    assert!(output["records"].as_array().unwrap().iter().any(|item| {
        item["text"]
            .as_str()
            .is_some_and(|text| text.contains("é漢字"))
    }));
}

#[test]
fn cancelling_a_checkpoint_aborts_reload_and_leaves_no_processes() {
    let fixture = Fixture::new();
    let directory = super::super::windows::temporary("rake-win-cancel-").unwrap();
    let args = Args {
        no_open: true,
        json: true,
        ..Args::default()
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut supervisor = Supervisor::new(
        &args,
        project(fixture.root.path()),
        directory.path().into(),
        &cancelled,
    )
    .unwrap();
    supervisor.publish(1, &fixture.binary).unwrap();
    let instance = instance(&supervisor);
    let app = Process::observe(supervisor.app.as_ref().unwrap().0.id());
    let child = descendant(&supervisor);
    fs::write(supervisor.session.join("stall-checkpoint"), "").unwrap();
    let waiting = supervisor.session.join("checkpoint-started");
    let flag = cancelled.clone();
    let cancel = thread::spawn(move || {
        wait_for(&waiting);
        flag.store(true, Ordering::Release);
    });
    let started = Instant::now();
    assert!(
        supervisor
            .prepare(&instance, &fixture.binary)
            .unwrap_err()
            .to_string()
            .contains("interrupted")
    );
    cancel.join().unwrap();
    let reload = exchange(
        &instance,
        "query",
        json!({"type":"reload"}),
        Duration::from_secs(1),
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reload["phase"], "idle",
        "cancelled preparation did not abort quiescence"
    );
    assert!(!supervisor.checkpoint.exists());
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "checkpoint cancellation exceeded its bounded abort exchange"
    );
    supervisor.finish().unwrap();
    app.stopped();
    child.stopped();
}

#[test]
fn web_publication_reload_and_stop_release_the_origin() {
    use std::io::Write;
    let directory = super::super::windows::temporary("rake-win-web-").unwrap();
    let args = Args {
        web: true,
        no_open: true,
        json: true,
        port: 0,
        ..Args::default()
    };
    let cancelled = AtomicBool::new(false);
    let mut supervisor = Supervisor::new(
        &args,
        project(directory.path()),
        directory.path().into(),
        &cancelled,
    )
    .unwrap();
    supervisor.start_server().unwrap();
    let url = url::Url::parse(supervisor.launch_url.as_ref().unwrap()).unwrap();
    let address = format!("127.0.0.1:{}", url.port().unwrap());
    for number in [1, 2] {
        let build = supervisor.session.join("builds").join(number.to_string());
        fs::create_dir(&build).unwrap();
        fs::write(
            build.join("index.html"),
            format!("<html><body>generation-{number}</body></html>"),
        )
        .unwrap();
        supervisor.publish(number, &build).unwrap();
        let mut socket = std::net::TcpStream::connect(&address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .write_all(format!("GET / HTTP/1.0\r\nHost: {address}\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(
            response.contains(&format!("generation-{number}")),
            "{response}"
        );
    }
    supervisor.finish().unwrap();
    assert!(
        std::net::TcpStream::connect(&address).is_err(),
        "origin remained listening after stop"
    );
    assert_eq!(
        read_json(&supervisor.session.join("status.json")).unwrap()["phase"],
        "stopped"
    );
}

#[test]
fn windows_fingerprint_ignores_junctions_and_tracks_source_edits() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("app.rs"), "first").unwrap();
    fs::write(outside.path().join("external.rs"), "external").unwrap();
    let junction = root.path().join("alias");
    assert!(
        Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .status()
            .unwrap()
            .success()
    );
    let project = project(root.path());
    let first = fingerprint(&project, None).unwrap();
    fs::write(outside.path().join("external.rs"), "changed").unwrap();
    assert_eq!(first, fingerprint(&project, None).unwrap());
    fs::write(root.path().join("app.rs"), "second").unwrap();
    assert_ne!(first, fingerprint(&project, None).unwrap());
    fs::remove_dir(junction).unwrap();
}
