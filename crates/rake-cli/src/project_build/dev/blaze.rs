//! Optional Blaze protocol, artifact identity and live-outcome state; no engine is linked here.

use super::super::{Args, Processes, Project, Result, string};
use super::{MAX_BYTES, Supervisor, exchange};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::{io::Write, os::unix::net::UnixStream};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum BuildKind {
    #[default]
    Full,
    Edit,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Patch,
    Rebuild,
    Restart,
}

#[derive(Default)]
pub(super) struct Live {
    pub(super) kind: BuildKind,
    pub(super) full_boundary: bool,
    pub(super) identity: Option<String>,
    pub(super) artifact: Option<PathBuf>,
    pub(super) cursor: u64,
    pub(super) sequence: u64,
    pub(super) published: u64,
    pub(super) restart: Option<String>,
    pub(super) declined: Option<String>,
    pub(super) needs_build: bool,
    pub(super) boundary: Option<(Value, String)>,
}

impl Live {
    pub(super) fn observe(&mut self, record: &Value) -> bool {
        let Some(cursor) = record["sequence"]
            .as_u64()
            .filter(|cursor| *cursor > self.cursor)
        else {
            return false;
        };

        self.cursor = cursor;

        match record["kind"].as_str() {
            Some("live_patch_applied") => {
                let Some(sequence) = record["fields"]["sequence"].as_u64() else {
                    return false;
                };

                self.sequence = sequence;
                self.restart = None;
                self.declined = None;
            }
            Some("live_restart_requested") => {
                let Some(reason) = record["fields"]["reason"].as_str() else {
                    return false;
                };

                self.restart = Some(reason.into());
                self.declined = None;
            }
            Some("live_patch_declined") => {
                let Some(reason) = record["fields"]["reason"].as_str() else {
                    return false;
                };

                self.declined = Some(reason.into());
            }
            _ => return false,
        }

        true
    }

    pub(super) fn decision(&self, identity: &str, kind: BuildKind) -> Decision {
        if self.identity.as_deref() != Some(identity) {
            Decision::Restart
        } else if self.restart.is_none() {
            Decision::Patch
        } else if kind == BuildKind::Edit {
            Decision::Rebuild
        } else {
            Decision::Restart
        }
    }
}

pub(super) fn environment(mut env: BTreeMap<OsString, OsString>) -> BTreeMap<OsString, OsString> {
    for (name, value) in [
        ("BLAZE_PROC_MACRO_WASM", "0"),
        ("BLAZE_PROC_MACRO_SERVER", "0"),
        ("BLAZE_SANDBOX", "on"),
        ("LIBLZMA_NO_PKG_CONFIG", "1"),
        ("RAKE_LIVE", "blaze"),
    ] {
        env.insert(name.into(), value.into());
    }

    for (name, value) in [
        ("RUST_MIN_STACK", "67108864"),
        ("BLAZE_MEMORY_MAX", "16G"),
        ("BLAZE_TIER", "off"),
        ("BLAZE_SPARE", "0"),
        ("BLAZE_STD_FROM", "off"),
    ] {
        env.entry(name.into()).or_insert_with(|| value.into());
    }

    env
}

pub(super) fn configure(command: &mut Command, args: &Args) {
    command.envs(environment(std::env::vars_os().collect()));

    if let Some(binary) = args.blaze_binary.as_ref()
        && let Some(dir) = binary.parent()
    {
        command.env(
            "BLAZE_TOOLCHAIN_PROC_MACRO_SERVER",
            dir.join("rust-analyzer-proc-macro-srv"),
        );
        command.env("BLAZE_HOME", dir.join("runtime"));
        command.env("BLAZE_CACHE", dir.join("store"));
        command.env("RUST_MIN_STACK", "67108864");
    }
}

fn socket(args: &Args) -> Result<PathBuf> {
    if let Some(binary) = args.blaze_binary.as_ref()
        && let Some(dir) = binary.parent()
    {
        return Ok(dir.join("runtime/daemon.sock"));
    }

    let home = std::env::var_os("BLAZE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|path| PathBuf::from(path).join("blaze"))
        })
        .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".cache/blaze")))
        .ok_or("no Blaze home directory")?;

    Ok(home.join("daemon.sock"))
}

#[cfg(unix)]
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

#[cfg(unix)]
fn unescape(value: &str) -> String {
    let mut chars = value.chars();
    let mut output = String::new();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('r') => output.push('\r'),
                Some(ch) => output.push(ch),
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }

    output
}

#[cfg(unix)]
pub(super) fn edit(
    socket: &Path,
    path: &Path,
    text: &str,
    processes: &Processes<'_>,
) -> Result<()> {
    processes.check_cancelled()?;
    let request = format!(
        "edit\t{}\t{}\n",
        escape(&std::path::absolute(path)?.to_string_lossy()),
        escape(text)
    );

    let mut connection = UnixStream::connect(socket)?;

    connection.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut written = 0;
    let mut response = Vec::new();
    let mut buffer = [0; 65536];

    loop {
        processes.check_cancelled()?;

        if Instant::now() >= deadline {
            return Err("Blaze edit request timed out".into());
        }

        let result = if written < request.len() {
            connection
                .write(&request.as_bytes()[written..])
                .inspect(|&count| {
                    written += count;
                })
        } else {
            let remaining = (MAX_BYTES + 1 - response.len()).min(buffer.len());

            connection.read(&mut buffer[..remaining]).inspect(|&count| {
                response.extend_from_slice(&buffer[..count]);
            })
        };

        match result {
            Ok(0) => return Err("Blaze edit response is truncated".into()),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(Duration::from_millis(2))
            }
            Err(error) => return Err(error.into()),
        }

        if response.len() > MAX_BYTES {
            return Err("Blaze edit response exceeds the limit".into());
        }

        if let Some(end) = response.iter().position(|byte| *byte == b'\n') {
            let reply = std::str::from_utf8(&response[..end])?.trim_end_matches('\r');

            if !reply.starts_with("ok") {
                return Err(unescape(reply).into());
            }

            return Ok(());
        }
    }
}

#[cfg(windows)]
pub(super) fn edit(
    _socket: &Path,
    _path: &Path,
    _text: &str,
    _processes: &Processes<'_>,
) -> Result<()> {
    Err("Blaze live editing is available only on Apple Silicon macOS".into())
}

pub(super) fn identity(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut bytes = [0; 32];
    let count = file.read(&mut bytes)?;

    if count >= 4 && bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe] {
        if count != 32 {
            return Err("truncated Mach-O header".into());
        }

        let commands = u32::from_le_bytes(bytes[16..20].try_into()?);
        let size = u32::from_le_bytes(bytes[20..24].try_into()?) as usize;

        if commands > 4096 || size > 16 * MAX_BYTES {
            return Err("invalid Mach-O load command table".into());
        }

        let mut data = vec![0; size];

        file.read_exact(&mut data)?;
        let mut offset = 0;

        for _ in 0..commands {
            let header = data
                .get(offset..offset + 8)
                .ok_or("truncated Mach-O load command")?;

            let command = u32::from_le_bytes(header[..4].try_into()?);
            let length = u32::from_le_bytes(header[4..].try_into()?) as usize;

            if length < 8
                || offset
                    .checked_add(length)
                    .is_none_or(|end| end > data.len())
            {
                return Err("invalid Mach-O load command size".into());
            }

            if command == 0x1b {
                if length != 24 {
                    return Err("invalid LC_UUID size".into());
                }

                return Ok(format!(
                    "uuid:{}",
                    data[offset + 8..offset + 24]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                ));
            }

            offset += length;
        }

        return Err("Blaze executable has no LC_UUID".into());
    }

    let mut digest = Sha256::new();

    digest.update(&bytes[..count]);
    let mut buffer = [0; 65536];

    loop {
        let count = file.read(&mut buffer)?;

        if count == 0 {
            break;
        }

        digest.update(&buffer[..count]);
    }

    Ok(format!("sha256:{:x}", digest.finalize()))
}

pub(super) fn artifact(target: &Path, profile: &str, name: &str) -> Result<PathBuf> {
    let copied = target.join(profile).join(name);
    let id = identity(&copied)?;
    let mut candidates = Vec::new();

    if let Ok(entries) = fs::read_dir(target.join("blaze/whole")) {
        for entry in entries {
            let path = entry?.path().join(name);

            if path.is_file() && identity(&path)? == id {
                candidates.push(path);
            }
        }
    }

    if candidates.len() != 1 {
        return Err(format!(
            "expected one daemon-watched Blaze executable matching {}; found {}",
            copied.display(),
            candidates.len()
        )
        .into());
    }

    Ok(candidates.remove(0))
}

pub(super) fn configuration(project: &Project) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();

    for root in &project.watch_roots {
        for name in [
            "Cargo.toml",
            "Cargo.lock",
            ".cargo/config.toml",
            ".cargo/config",
        ] {
            let path = root.join(name);

            digest.update(path.as_os_str().as_encoded_bytes());

            if path.is_file() {
                digest.update(fs::read(path)?);
            } else {
                digest.update(b"missing");
            }
        }
    }

    Ok(digest.finalize().into())
}

fn compiler_command(args: &Args) -> Result<Command> {
    if args.live {
        let binary = args
            .blaze_binary
            .as_ref()
            .ok_or("the pinned live compiler was not initialized")?;

        // The thin `blaze` launcher can honor a workspace pin to another release.
        // Enter this exact engine directly so --live cannot fetch/run another Blaze.
        let mut command = Command::new(binary.with_file_name("blaze-engine"));

        command.arg("cargo");

        Ok(command)
    } else {
        Ok(Command::new("blaze"))
    }
}

pub(super) fn build(project: &Project, args: &Args, processes: &Processes<'_>) -> Result<PathBuf> {
    let mut command = compiler_command(args)?;

    if !args.json && !args.verbose {
        command.env("CARGO_TERM_COLOR", "never");
    }

    command
        .args(["build", "--locked", "--events=json", "--manifest-path"])
        .arg(string(&project.package, "manifest_path")?)
        .arg("--package")
        .arg(string(&project.package, "name")?)
        .arg("--bin")
        .arg(&project.target)
        .args(["--config", "profile.dev.debug=\"line-tables-only\""])
        .current_dir(&project.root);

    if args.release {
        command.arg("--release");
    }

    if let Some(features) = project.build_features(args) {
        command.arg("--features").arg(features);
    }

    configure(&mut command, args);
    processes.run_output(&mut command, |_| {})?;

    artifact(
        &project.target_directory,
        if args.release { "release" } else { "debug" },
        &project.target,
    )
}

pub(super) fn update(
    project: &Project,
    args: &Args,
    processes: &Processes<'_>,
    paths: &[PathBuf],
    previous: &Path,
) -> Result<(PathBuf, BuildKind)> {
    if !paths.is_empty()
        && paths
            .iter()
            .all(|path| path.extension().is_some_and(|ext| ext == "rs") && path.is_file())
    {
        let result = (|| {
            let socket = socket(args)?;

            for path in paths {
                edit(&socket, path, &fs::read_to_string(path)?, processes)?;
            }

            Ok::<_, Box<dyn std::error::Error>>(())
        })();

        if result.is_ok() {
            return Ok((previous.to_owned(), BuildKind::Edit));
        }

        processes.check_cancelled()?;
    }

    Ok((build(project, args, processes)?, BuildKind::Full))
}

impl Supervisor<'_> {
    pub(super) fn poll_live_events(&mut self) -> Result<()> {
        if self.live.is_none()
            || self
                .app
                .as_mut()
                .is_none_or(|app| app.0.try_wait().ok().flatten().is_some())
        {
            return Ok(());
        }

        let Some(instance) = self.instance() else {
            return Ok(());
        };

        let cursor = self.live.as_ref().map_or(0, |live| live.cursor);

        let snapshot = exchange(
            &instance,
            "query",
            json!({"type":"diagnostics","after":cursor,"kind":null}),
            Duration::from_millis(500),
            None,
            self.cancelled,
        )?;

        if let Some(records) = snapshot["records"].as_array() {
            for record in records {
                if self.live.as_mut().is_some_and(|live| live.observe(record)) {
                    let mut fields = record["fields"].clone();

                    fields["runtime_pid"] = self.status["runtime_pid"].clone();
                    self.record(
                        record["kind"].as_str().ok_or("live event kind missing")?,
                        fields,
                    )?;
                }
            }
        }

        Ok(())
    }

    pub(super) fn prepare_live_boundary(&mut self) -> Result<()> {
        if self.live.is_none()
            || !self.live.as_ref().is_some_and(|live| live.full_boundary)
            || self
                .app
                .as_mut()
                .is_none_or(|app| app.0.try_wait().ok().flatten().is_some())
        {
            return Ok(());
        }

        let instance = self
            .instance()
            .ok_or("full live boundary requires the app's control endpoint")?;

        let id = uuid::Uuid::new_v4().to_string();
        let start = Instant::now();

        self.live.as_mut().ok_or("live state missing")?.boundary =
            Some((instance.clone(), id.clone()));
        exchange(
            &instance,
            "command",
            json!({"type":"prepare_reload","request":{"id":id,"mechanism":"native_code_replacement"}}),
            Duration::from_secs(2),
            Some(&format!("{id}/prepare")),
            self.cancelled,
        )?;

        while self.query(&instance, json!({"type":"reload"}))?["phase"] != "ready" {
            if start.elapsed() > Duration::from_secs(15) {
                return Err("live reload boundary timed out".into());
            }

            thread::sleep(Duration::from_millis(10));
        }

        self.record(
            "live_boundary_prepared",
            json!({"boundary_id":id,"took_us":start.elapsed().as_micros() as u64}),
        )
    }

    pub(super) fn finish_live_boundary(&mut self, applied: bool) -> Result<()> {
        let Some((instance, id)) = self.live.as_mut().and_then(|live| live.boundary.take()) else {
            return Ok(());
        };

        // Cleanup also runs on cancellation, with the same keyed abort as preparation.
        let cleanup = std::sync::atomic::AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(2);

        loop {
            let status = exchange(
                &instance,
                "query",
                json!({"type":"reload"}),
                Duration::from_secs(2),
                None,
                &cleanup,
            )?;

            if !matches!(status["phase"].as_str(), Some("ready" | "quiescing")) {
                return Ok(());
            }

            if !applied || Instant::now() >= deadline {
                exchange(
                    &instance,
                    "command",
                    json!({"type":"finish_reload","id":id,"applied":false}),
                    Duration::from_secs(2),
                    Some(&format!("{id}/abort")),
                    &cleanup,
                )?;

                return Ok(());
            }

            thread::sleep(Duration::from_millis(10));
        }
    }
}

pub(super) fn stamp_identity(stamps: &super::fingerprint::Stamps) -> [u8; 32] {
    let mut digest = Sha256::new();

    for (path, stamp) in stamps {
        digest.update(path.as_os_str().as_encoded_bytes());
        digest.update(stamp.0.to_le_bytes());
        digest.update(stamp.1.to_le_bytes());
        digest.update(stamp.2.to_le_bytes());
        digest.update(stamp.3.to_le_bytes());
    }

    digest.finalize().into()
}

#[cfg(test)]
mod pinned_socket_tests {
    use super::*;

    #[test]
    fn pinned_edits_use_the_same_home_as_the_build_and_app() {
        let args = Args {
            blaze_binary: Some(PathBuf::from("/pinned/blaze")),
            ..Args::default()
        };

        let mut command = Command::new("/app");

        configure(&mut command, &args);
        let home = command
            .get_envs()
            .find(|(key, _)| *key == "BLAZE_HOME")
            .unwrap()
            .1
            .unwrap();

        assert_eq!(
            socket(&args).unwrap(),
            PathBuf::from(home).join("daemon.sock")
        );
    }
}

#[cfg(test)]
mod pinned_command_tests {
    use super::*;

    #[test]
    fn live_bypasses_the_workspace_version_redirector_and_never_uses_path() {
        let mut args = Args {
            live: true,
            ..Args::default()
        };

        assert!(compiler_command(&args).is_err());
        args.blaze_binary = Some(PathBuf::from("/pinned/blaze"));
        let command = compiler_command(&args).unwrap();

        assert_eq!(
            Path::new(command.get_program()),
            Path::new("/pinned/blaze-engine")
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("cargo")]
        );
        args.live = false;
        assert_eq!(compiler_command(&args).unwrap().get_program(), "blaze");
    }
}
