//! Owns desktop/Web sessions, asynchronous builds, publication and retained discovery records.
//! The session owns its native HTTP broker, preview helper process and optional CDP companion.

mod fingerprint;
mod output;
mod services;
mod transport;

use super::{
    Args, Processes, Project, Result, json_line, processes::ProcessGroup, string, write_marker,
};

use fingerprint::fingerprint;
use output::{ProcessOutput, SharedOutput};
use serde_json::{Value, json};

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::{
    fs::{DirBuilderExt, PermissionsExt},
    process::ExitStatusExt,
};

use transport::{MAX_BYTES, exchange};

pub(super) fn run(args: &Args, cancelled: &AtomicBool) -> Result<()> {
    #[cfg(windows)]
    if args.preview {
        return Err("Native preview capture is available only on macOS".into());
    }

    if args.status {
        let session = args
            .session
            .as_ref()
            .ok_or("use rake dev status --session PATH")?;

        let status = read_json(&session.join("status.json"))?;

        println!(
            "{}",
            if args.json {
                json_line(&status)?
            } else {
                super::ascii_json(&serde_json::to_string_pretty(&status)?)
            }
        );

        return Ok(());
    }

    let mut args = args.clone();

    args.native_reloader = args.native_reloader.map(fs::canonicalize).transpose()?;
    let project = Project::load(&args, &Processes::new(cancelled))?;

    #[cfg(unix)]
    let session = tempfile::Builder::new()
        .prefix("rake-dev-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp")?
        .keep();

    #[cfg(windows)]
    let session = super::windows::temporary("rake-dev-")?.keep();

    let mut supervisor = Supervisor::new(&args, project, session, cancelled)?;
    let result = supervisor.run();

    // Also cancel a build when another owned resource fails, before joining its worker.
    supervisor.finish()?;

    result
}

#[cfg(unix)]
type HelperDirectory = tempfile::TempDir;
#[cfg(windows)]
type HelperDirectory = super::windows::Directory;

struct Supervisor<'a> {
    args: &'a Args,
    project: Project,
    session: PathBuf,
    control: PathBuf,
    checkpoint: PathBuf,
    status: Value,
    app: Option<ProcessGroup>,
    cdp: Option<ProcessGroup>,
    http: Option<super::serve::Server>,
    embedded: Option<ProcessGroup>,
    helpers: Option<HelperDirectory>,
    launch_url: Option<String>,
    output: SharedOutput,
    readers: Vec<JoinHandle<std::result::Result<(), String>>>,
    cancelled: &'a AtomicBool,
}

impl<'a> Supervisor<'a> {
    fn new(
        args: &'a Args,
        project: Project,
        session: PathBuf,
        cancelled: &'a AtomicBool,
    ) -> Result<Self> {
        let control = session.join("control");

        #[cfg(unix)]
        fs::DirBuilder::new().mode(0o700).create(&control)?;
        #[cfg(windows)]
        rake_client::private::create_directory(&control)?;
        fs::create_dir(session.join("builds"))?;
        let status = json!({"protocol":1, "pid":std::process::id(), "target":if args.web {"web"} else {"desktop"}, "package":project.package["name"],
            "phase":"starting", "published":0, "build":0, "error":null,
            "mechanism":if args.web {"web_page_reload"} else if args.native_reloader.is_some() {"native_code_replacement"} else {"native_restart"}});

        let mut supervisor = Self {
            args,
            project,
            checkpoint: session.join("checkpoint.json"),
            session,
            control,
            status,
            app: None,
            cdp: None,
            http: None,
            embedded: None,
            helpers: None,
            launch_url: None,
            output: ProcessOutput::new(),
            readers: Vec::new(),
            cancelled,
        };

        supervisor.record("starting", json!({}))?;

        Ok(supervisor)
    }

    fn event(&self, value: Value, text: &str) -> Result<()> {
        let mut stdout = std::io::stdout().lock();

        writeln!(
            stdout,
            "{}",
            if self.args.json {
                json_line(&value)?
            } else {
                text.to_owned()
            }
        )?;
        stdout.flush()?;

        Ok(())
    }

    fn record(&mut self, phase: &str, fields: Value) -> Result<()> {
        self.status["phase"] = json!(phase);

        if let (Some(status), Some(fields)) = (self.status.as_object_mut(), fields.as_object()) {
            status.extend(fields.clone());
        }

        write_marker(&self.session.join("status.json"), &self.status)?;
        let mut event = fields;

        event["event"] = json!(phase);
        let error = event["error"]
            .as_str()
            .filter(|error| !error.is_empty())
            .map(|error| format!(" — {error}"))
            .unwrap_or_default();

        self.event(event, &format!("rake: {phase}{error}"))
    }

    fn instance(&self) -> Option<Value> {
        let pid = self.app.as_ref()?.0.id();

        fs::read_dir(&self.control)
            .ok()?
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| read_json(&entry.path()).ok())
            .find(|value| value["pid"] == pid)
    }

    fn launch(&mut self, artifact: &Path) -> Result<()> {
        let mut command = Command::new(artifact);

        command
            .current_dir(&self.project.root)
            .env("RAKE_DEV_CONTROL", "1")
            .env("RAKE_CONTROL_DIR", &self.control)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if std::env::var_os("RUST_BACKTRACE").is_none() {
            command.env("RUST_BACKTRACE", "1");
        }

        if self.args.preview {
            command.env("RAKE_DEV_PREVIEW", "1");
        }

        if self.checkpoint.exists() {
            command.env("RAKE_RELOAD_CHECKPOINT", &self.checkpoint);
        }

        let mut app = Processes::new(self.cancelled).start(&mut command)?;
        let pid = app.0.id();

        self.readers = self
            .readers
            .drain(..)
            .filter_map(|reader| {
                if reader.is_finished() {
                    let _ = reader.join();

                    None
                } else {
                    Some(reader)
                }
            })
            .collect();
        self.readers.push(output::collect(
            app.0.stdout.take().ok_or("app stdout missing")?,
            self.output.clone(),
            pid,
            "stdout",
        ));
        self.readers.push(output::collect(
            app.0.stderr.take().ok_or("app stderr missing")?,
            self.output.clone(),
            pid,
            "stderr",
        ));
        self.app = Some(app);

        self.record("running", json!({"runtime_pid":pid, "app_running":true, "mechanism":"native_restart", "error":null}))
    }

    fn query(&self, instance: &Value, value: Value) -> Result<Value> {
        exchange(
            instance,
            "query",
            value,
            Duration::from_secs(2),
            None,
            self.cancelled,
        )
    }

    fn prepare(&mut self, instance: &Value, artifact: &Path) -> Result<bool> {
        let mechanism = if self.args.native_reloader.is_some() {
            "native_code_replacement"
        } else {
            "native_restart"
        };

        self.record("preparing_reload", json!({"mechanism":mechanism}))?;
        let boundary = uuid::Uuid::new_v4().to_string();

        let result = (|| {
            let deadline = Instant::now() + Duration::from_secs(15);

            exchange(
                instance,
                "command",
                json!({"type":"prepare_reload", "request":{"id":boundary,"mechanism":mechanism}}),
                Duration::from_secs(2),
                Some(&format!("{boundary}/prepare")),
                self.cancelled,
            )?;

            while self.query(instance, json!({"type":"reload"}))?["phase"] != "ready" {
                if Instant::now() >= deadline {
                    return Err("reload quiescence timed out".into());
                }

                thread::sleep(Duration::from_millis(50));
            }

            let checkpoint = self.query(
                instance,
                json!({"type":"checkpoint", "application_id":instance["application_id"]}),
            )?;

            write_marker(&self.checkpoint, &checkpoint)?;

            if let Some(reloader) = &self.args.native_reloader {
                self.record("replacing_native_code", json!({}))?;
                let mut command = Command::new(reloader);

                command
                    .current_dir(&self.project.root)
                    .env("RAKE_RELOAD_ARTIFACT", artifact)
                    .env("RAKE_RELOAD_CHECKPOINT", &self.checkpoint)
                    .env(
                        "RAKE_RELOAD_RUNTIME",
                        string(&instance["info"], "runtime_id")?,
                    )
                    .env(
                        "RAKE_RELOAD_GENERATION",
                        (checkpoint["generation"]
                            .as_u64()
                            .ok_or("checkpoint generation missing")?
                            + 1)
                        .to_string(),
                    )
                    .env("RAKE_RELOAD_PROJECT", &self.project.root);
                let sink = |text: &str, pid, stream: &str| {
                    self.output
                        .lock()
                        .expect("process output lock poisoned")
                        .append(text, pid, stream, None, false)
                };

                Processes::new(self.cancelled)
                    .with_output(&sink)
                    .run_output(&mut command, |_| {})?;
                exchange(
                    instance,
                    "command",
                    json!({"type":"finish_reload", "id":boundary, "applied":true}),
                    Duration::from_secs(2),
                    Some(&format!("{boundary}/apply")),
                    self.cancelled,
                )?;
                remove_file(&self.checkpoint)?;

                return Ok(true);
            }

            Ok(false)
        })();

        if result.is_err() {
            // An interrupted preparation still needs its matching abort to release quiescence.
            let _ = exchange(
                instance,
                "command",
                json!({"type":"finish_reload", "id":boundary,"applied":false}),
                Duration::from_secs(2),
                Some(&format!("{boundary}/abort")),
                &AtomicBool::new(false),
            );
        }

        result
    }

    fn publish(&mut self, number: u64, artifact: &Path) -> Result<()> {
        if self.args.web {
            self.record(
                "watching",
                json!({"published":number, "directory":artifact, "error":null}),
            )?;

            return self.start_embedded();
        }

        if let Some(app) = &mut self.app
            && app.0.try_wait()?.is_none()
        {
            if let Some(instance) = self.instance() {
                let reload = self.query(&instance, json!({"type":"reload"}))?;

                if reload["phase"] != "restore_failed" {
                    let operations = self.query(&instance, json!({"type":"operations"}))?;

                    if operations.as_array().is_some_and(|operations| {
                        operations.iter().any(|operation| {
                            let state = &operation[1];

                            state == "Queued"
                                || state == "Running"
                                || state.get("AwaitingDecision").is_some()
                        })
                    }) {
                        return Err("\"Busy\": waiting for pending operations and decisions".into());
                    }

                    if self.prepare(&instance, artifact)? {
                        return self.record("watching", json!({"published":number, "error":null}));
                    }
                }
            } else if !self.args.restart_without_state {
                return Err("live app has no development control endpoint; enable ControlConfig::FromEnvironment, or explicitly use --restart-without-state".into());
            } else {
                self.record("restarting_without_state", json!({}))?;
            }

            let old_pid = self.app.as_ref().ok_or("app missing")?.0.id();

            drop(self.app.take());

            for entry in fs::read_dir(&self.control)?.flatten() {
                let path = entry.path();

                if path
                    .extension()
                    .is_some_and(|extension| extension == "json")
                    && read_json(&path).is_ok_and(|value| value["pid"] == old_pid)
                {
                    let _ = remove_file(&path);
                    let _ = remove_file(&path.with_extension("sock"));
                }
            }
        }

        // A crashed app retains the latest captured checkpoint, just as dev.py does.
        self.launch(artifact)?;

        self.record("watching", json!({"published":number, "error":null}))
    }

    fn publication(&mut self, number: u64, artifact: &Path) -> Result<bool> {
        match self.publish(number, artifact) {
            Ok(()) => Ok(true),
            Err(error) => {
                if self.cancelled.load(Ordering::Acquire) {
                    return Err(error);
                }

                let error = error.to_string();
                let busy = error.contains("\"Busy\"");

                self.record(
                    if busy {
                        "reload_waiting"
                    } else {
                        "reload_blocked"
                    },
                    json!({"error":error}),
                )?;

                Ok(!busy)
            }
        }
    }

    fn observe_app(&mut self) -> Result<()> {
        let Some(app) = &mut self.app else {
            return Ok(());
        };

        if let Some(status) = app.0.try_wait()? {
            if self.status["app_running"] == true {
                #[cfg(unix)]
                let code = status
                    .code()
                    .unwrap_or_else(|| -status.signal().unwrap_or(1));

                #[cfg(windows)]
                let code = status.code().unwrap_or(1);

                self.output
                    .lock()
                    .expect("process output lock poisoned")
                    .append(
                        &format!("Application exited with code {code}"),
                        app.0.id(),
                        "lifecycle",
                        Some(if code == 0 { "info" } else { "error" }),
                        false,
                    );
                self.record("app_exited", json!({"app_running":false, "exit_code":code}))?;
            }
        } else if ["watching", "running", "restore_failed"]
            .iter()
            .any(|phase| self.status["phase"] == *phase)
            && let Some(instance) = self.instance()
            && let Ok(reload) = exchange(
                &instance,
                "query",
                json!({"type":"reload"}),
                Duration::from_millis(500),
                None,
                self.cancelled,
            )
        {
            if reload["phase"] == "restore_failed" {
                self.record("restore_failed", json!({"error":reload["error"]}))?;
            } else if self.status["runtime_id"] != instance["info"]["runtime_id"]
                || self.status["phase"] == "restore_failed"
            {
                self.record("watching", json!({"runtime_id":instance["info"]["runtime_id"], "reload":reload, "error":null}))?;
            }
        }

        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        if !self.args.web {
            self.event(json!({"event":"session", "session":self.session, "mechanism":self.status["mechanism"]}),
            &format!("Session: {}; reload mechanism: {}", self.session.display(), self.status["mechanism"].as_str().unwrap_or_default()))?;
        }

        self.start_server()?;
        let stopping = AtomicBool::new(false);

        thread::scope(|scope| {
            let mut worker = None;

            type BuildResult = (
                Option<Project>,
                std::result::Result<PathBuf, String>,
                [u8; 32],
            );
            let (sender, receiver) = mpsc::sync_channel::<BuildResult>(1);

            let result: Result<()> = (|| {
                let mut previous = None;
                let mut changed_at = Instant::now();
                let mut built = None;
                let mut number = 0;
                let mut pending: Option<(u64, PathBuf)> = None;
                let mut retry_after = Instant::now();
                let mut output_revision = None;

                loop {
                    Processes::new(self.cancelled).check_cancelled()?;
                    self.observe_services()?;
                    let output = self
                        .output
                        .lock()
                        .expect("process output lock poisoned")
                        .snapshot();

                    if output_revision != output["revision"].as_u64() {
                        write_marker(&self.session.join("output.json"), &output)?;
                        output_revision = output["revision"].as_u64();
                    }

                    let current = fingerprint(&self.project, self.args.output.as_deref())?;

                    if previous != Some(current) {
                        previous = Some(current);
                        changed_at = Instant::now();
                        pending = None;
                    }

                    if let Ok((project, result, started)) = receiver.try_recv() {
                        if let Some(worker) = worker.take() {
                            let worker: thread::ScopedJoinHandle<'_, ()> = worker;

                            worker.join().map_err(|_| "build worker panicked")?;
                        }

                        if let Some(project) = project {
                            self.project = project;
                        }

                        // Metadata can add local watch roots while the worker is building.
                        let current = fingerprint(&self.project, self.args.output.as_deref())?;

                        if current != started {
                            let _ = fs::remove_dir_all(
                                self.session.join("builds").join(number.to_string()),
                            );

                            self.record("build_superseded", json!({}))?;
                        } else {
                            match result {
                                Err(error) => {
                                    self.record("build_failed", json!({"error":error}))?;
                                    let _ = fs::remove_dir_all(
                                        self.session.join("builds").join(number.to_string()),
                                    );
                                }
                                Ok(artifact) => {
                                    if !self.publication(number, &artifact)? {
                                        pending = Some((number, artifact));
                                        retry_after = Instant::now() + Duration::from_secs(1);
                                    }
                                }
                            }
                        }
                    }

                    if worker.is_none()
                        && Instant::now() >= retry_after
                        && let Some((number, artifact)) = &pending
                    {
                        match self.publish(*number, artifact) {
                            Ok(()) => pending = None,
                            Err(error) if self.cancelled.load(Ordering::Acquire) => {
                                return Err(error);
                            }
                            Err(error) if error.to_string().contains("\"Busy\"") => {}
                            Err(error) => {
                                self.record("reload_blocked", json!({"error":error.to_string()}))?;
                                pending = None;
                            }
                        }

                        retry_after = Instant::now() + Duration::from_secs(1);
                    }

                    if worker.is_none()
                        && previous != built
                        && changed_at.elapsed() >= Duration::from_millis(250)
                    {
                        number += 1;
                        built = previous;
                        self.record("building", json!({"build":number, "error":null}))?;
                        let sender = sender.clone();
                        let args = self.args;
                        let cancelled = self.cancelled;
                        let stopping = &stopping;
                        let output = self.session.join("builds").join(number.to_string());
                        let process_output = self.output.clone();

                        worker = Some(scope.spawn(move || {
                            let sink = |text: &str, pid, stream: &str| {
                                process_output
                                    .lock()
                                    .expect("process output lock poisoned")
                                    .append(text, pid, stream, None, false);
                            };

                            let processes = Processes::new(cancelled)
                                .with_stopping(stopping)
                                .with_output(&sink);

                            let (project, result) = match Project::load(args, &processes) {
                                Ok(project) => {
                                    let result = project
                                        .build(args, &processes, &output)
                                        .map_err(|error| error.to_string());

                                    (Some(project), result)
                                }
                                Err(error) => (None, Err(error.to_string())),
                            };

                            let _ = sender.send((project, result, current));
                        }));
                    }

                    self.observe_app()?;
                    thread::sleep(Duration::from_millis(100));
                }
            })();

            if result.is_err() {
                // The worker owns its compiler process group; cancellation releases its pipe and joins it.
                stopping.store(true, Ordering::Release);
            }

            if let Some(worker) = worker {
                worker.join().map_err(|_| "build worker panicked")?;
            }

            result
        })
    }

    fn finish(&mut self) -> Result<()> {
        drop(self.app.take());
        drop(self.cdp.take());
        drop(self.http.take());
        drop(self.embedded.take());
        drop(self.helpers.take());

        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }

        write_marker(
            &self.session.join("output.json"),
            &self
                .output
                .lock()
                .expect("process output lock poisoned")
                .snapshot(),
        )?;
        self.record("stopped", json!({"app_running":false}))?;

        for entry in fs::read_dir(&self.control)? {
            remove_file(&entry?.path())?;
        }

        Ok(())
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let mut bytes = Vec::new();

    #[cfg(unix)]
    let file = fs::File::open(path)?;

    #[cfg(windows)]
    let file = rake_client::private::open(path)?;

    file.take(MAX_BYTES as u64).read_to_end(&mut bytes)?;

    Ok(serde_json::from_slice(&bytes)?)
}

fn remove_file(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests;
