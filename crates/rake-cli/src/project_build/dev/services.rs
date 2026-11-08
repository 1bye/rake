//! Session-owned CDP, Web server, Swift capture compiler and Node companion lifecycle.

use super::{Supervisor, transport::MAX_BYTES};
use crate::project_build::{Processes, Result, processes::ProcessGroup, string, write_marker};
use serde_json::json;
use std::{
    fs,
    io::Read,
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::PathBuf,
    process::{ChildStdout, Command, Stdio},
};

impl Supervisor<'_> {
    fn service_command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command.env("RAKE_CONTROL_DIR", &self.control).env(
            "RAKE_SOURCE_ROOT",
            std::env::var_os("RAKE_SOURCE_ROOT")
                .unwrap_or_else(|| self.project.root.as_os_str().to_owned()),
        );

        command
    }

    fn start_cdp(&mut self) -> Result<Option<String>> {
        if !self.args.cdp {
            return Ok(None);
        }
        let executable = std::env::var_os("RAKE_CLI")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_exe()?);
        let mut command = self.service_command(executable);
        command
            .args(["cdp", "--session"])
            .arg(&self.session)
            .args(["--no-open", "--json"])
            .stdout(Stdio::piped());
        self.cdp = Some(Processes::new(self.cancelled).start(&mut command)?);
        let pipe = stdout(&mut self.cdp)?;
        let line = readiness(pipe, self.cancelled, "CDP adapter could not start")?;
        let ready: serde_json::Value = serde_json::from_str(&line)?;
        if ready["event"] != "cdp_listening" {
            return Err("CDP adapter could not start".into());
        }
        let url = string(&ready, "url")?.to_owned();
        write_marker(&self.session.join("cdp.json"), &json!({"url":url}))?;
        self.event(
            ready,
            &format!("Chrome DevTools discovery: {url}/json/list"),
        )?;

        Ok(Some(url))
    }

    pub(super) fn start_server(&mut self) -> Result<()> {
        let cdp_url = self.start_cdp()?;
        if !self.args.web && !self.args.preview {
            return Ok(());
        }
        let helpers = tempfile::Builder::new()
            .prefix("rake-tools-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()?;
        super::super::resources::extract(helpers.path())?;
        self.helpers = Some(helpers);
        let helpers = self
            .helpers
            .as_ref()
            .ok_or("development helpers missing")?
            .path();
        let waiting = self.session.join("waiting");
        fs::create_dir(&waiting)?;
        fs::write(
            waiting.join("index.html"),
            "<!doctype html><html><head><title>Rake development</title></head><body>Waiting for the first successful build…</body></html>",
        )?;
        let preview_helper = self
            .args
            .preview
            .then(|| self.session.join("preview-capture"));
        if let Some(helper) = &preview_helper {
            let mut command = Command::new("xcrun");
            command
                .arg("swiftc")
                .arg(helpers.join("preview.swift"))
                .arg("-module-cache-path")
                .arg(self.session.join("swift-cache"))
                .arg("-o")
                .arg(helper);
            let sink = |text: &str, pid, stream: &str| {
                self.output
                    .lock()
                    .expect("process output lock poisoned")
                    .append(text, pid, stream, None, false);
            };
            Processes::new(self.cancelled)
                .with_output(&sink)
                .run_output(&mut command, |_| {})?;
        }
        let listener = std::net::TcpListener::bind(("127.0.0.1", self.args.port))?;
        let origin = format!("http://{}", listener.local_addr()?);

        let broker = std::sync::Arc::new(crate::project_build::broker::Broker::new(
            crate::project_build::broker::Options {
                url: origin.clone(),
                directory: self.control.clone(),
                development: Some(self.session.join("status.json")),
                output: Some(self.session.join("output.json")),
                allow_native: !self.args.web,
                cdp_url,
                source_root: std::env::var_os("RAKE_SOURCE_ROOT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.project.root.clone()),
                cli: std::env::var_os("RAKE_CLI")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_exe()?),
            },
        )?);

        let preview = preview_helper.map(|helper| {
            std::sync::Arc::new(crate::project_build::preview::Preview::new(
                broker.clone(),
                helper,
            ))
        });

        let url = format!(
            "{origin}{}#__rake_control={}",
            if self.args.preview {
                "/__rake/preview"
            } else {
                "/"
            },
            broker.token()
        );

        self.http = Some(crate::project_build::serve::Server::start(
            listener,
            crate::project_build::serve::Config {
                directory: waiting,
                development: Some(self.session.join("status.json")),
                builds: Some(self.session.join("builds")),
                control_only: !self.args.web,
                preview,
                broker: Some(broker),
            },
        )?);
        let line = format!("Serving routing at {url}");

        self.launch_url = Some(url.clone());
        self.event(json!({"event":if self.args.preview {"preview_listening"} else if self.args.web {"web_listening"} else {"control_listening"}, "url":url, "session":self.session}),
            &format!("{}\nSession: {}", line.trim(), self.session.display()))?;
        if !self.args.no_open && (self.args.preview || (self.args.web && !self.args.cdp)) {
            self.open_browser(&url);
        }

        Ok(())
    }

    pub(super) fn start_embedded(&mut self) -> Result<()> {
        if !self.args.cdp || self.embedded.is_some() {
            return Ok(());
        }
        let helpers = self
            .helpers
            .as_ref()
            .ok_or("development helpers missing")?
            .path();
        let mut command = Command::new("node");
        command
            .arg(helpers.join("embedded-devtools.mjs"))
            .arg("--session")
            .arg(&self.session)
            .arg("--app-url")
            .arg(
                self.launch_url
                    .as_ref()
                    .ok_or("development launch URL missing")?,
            )
            .arg("--json")
            .stdout(Stdio::piped());
        self.embedded = Some(Processes::new(self.cancelled).start(&mut command)?);
        let line = readiness(
            stdout(&mut self.embedded)?,
            self.cancelled,
            "embedded Chrome DevTools exited before publishing its launch URL",
        )?;
        let ready: serde_json::Value = serde_json::from_str(&line)?;
        if ready["event"] != "embedded_devtools_listening" {
            return Err("embedded Chrome DevTools could not start".into());
        }
        let url = string(&ready, "url")?.to_owned();
        self.record("watching", json!({"devtools_url":url}))?;
        self.event(ready, &format!("Chrome DevTools toggle: {url}"))?;
        if !self.args.no_open {
            self.open_browser(&url);
        }

        Ok(())
    }

    fn open_browser(&mut self, url: &str) {
        for mut command in
            super::super::browser::commands(std::env::var_os("BROWSER").as_deref(), url)
        {
            command.stdout(Stdio::null()).stderr(Stdio::null());
            if let Ok(group) = Processes::new(self.cancelled).start(&mut command) {
                self.openers.push(group);
                break;
            }
        }
    }

    pub(super) fn observe_services(&mut self) -> Result<()> {
        if let Some(http) = &self.http {
            http.check()?;
        }
        for (group, message) in [(
            &mut self.embedded,
            "embedded Chrome DevTools companion exited",
        )] {
            if let Some(group) = group
                && group.0.try_wait()?.is_some()
            {
                return Err(message.into());
            }
        }
        // The CDP child retains its existing independent lifecycle semantics.
        Ok(())
    }
}

fn stdout(group: &mut Option<ProcessGroup>) -> Result<ChildStdout> {
    group
        .as_mut()
        .and_then(|group| group.0.stdout.take())
        .ok_or_else(|| "development service stdout missing".into())
}

fn readiness(
    mut pipe: ChildStdout,
    cancelled: &std::sync::atomic::AtomicBool,
    failure: &str,
) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        Processes::new(cancelled).check_cancelled()?;
        let mut descriptor = libc::pollfd {
            fd: pipe.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor points to one live owned pipe for this poll call.
        let ready = unsafe { libc::poll(&mut descriptor, 1, 25) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if ready == 0 {
            continue;
        }
        let mut byte = [0];
        if pipe.read(&mut byte)? == 0 {
            return if bytes.is_empty() {
                Err(failure.into())
            } else {
                Ok(String::from_utf8(bytes)?)
            };
        }
        bytes.push(byte[0]);
        if bytes.len() > MAX_BYTES {
            return Err("development readiness exceeds the control frame limit".into());
        }
        if byte[0] == b'\n' {
            return Ok(String::from_utf8(bytes)?);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Covers: readiness EOF and frame bounds shared by CDP, Web and Node children.
    //! Real: pipes, readiness parser and owned process groups. Doubles: shell output.
    //! Requires: Unix, /bin/sh and /bin/dd.
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn readiness_accepts_unterminated_last_line_and_rejects_empty_or_oversized_frames() {
        let cancelled = AtomicBool::new(false);
        for (source, expected) in [
            ("printf 'ready'", "ready"),
            ("exit 0", "service failed"),
            (
                "/bin/dd if=/dev/zero bs=1048576 count=2 2>/dev/null",
                "development readiness exceeds the control frame limit",
            ),
        ] {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", source]).stdout(Stdio::piped());
            let mut group = Some(Processes::new(&cancelled).start(&mut command).unwrap());
            let result = readiness(stdout(&mut group).unwrap(), &cancelled, "service failed");
            let actual = result.unwrap_or_else(|error| error.to_string());
            assert_eq!(actual, expected, "{source}");
        }
    }
}
