//! Server-owned capture helper. Each capture owns a bounded, killable child process.

use super::{TIMEOUT, wire::Error};
use base64::Engine;
use rake_client::local::Instance;
use rake_protocol::inspection::preview::Window;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) struct Screenshots {
    directory: Option<PathBuf>,
    executable: Result<PathBuf, String>,
}

impl Screenshots {
    pub fn prepare() -> Self {
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::DirBuilderExt;
            let directory =
                std::env::temp_dir().join(format!("rake-cdp-capture-{}", uuid::Uuid::new_v4()));
            let result = (|| -> Result<PathBuf, String> {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&directory)
                    .map_err(|e| e.to_string())?;
                let source = directory.join("capture.swift");
                std::fs::write(&source, include_str!("../../../../scripts/preview.swift"))
                    .map_err(|e| e.to_string())?;
                let executable = directory.join("capture");
                let mut compiler = Command::new("xcrun");
                compiler
                    .args(["swiftc", "-module-cache-path"])
                    .arg(directory.join("swift-cache"))
                    .arg(&source)
                    .arg("-o")
                    .arg(&executable);
                output(compiler, Duration::from_secs(20), 64 * 1024)?;
                Ok(executable)
            })();
            Self {
                directory: Some(directory),
                executable: result,
            }
        }
        #[cfg(not(target_os = "macos"))]
        Self {
            directory: None,
            executable: Err("Native screenshots require macOS 14 or later".into()),
        }
    }

    pub fn available(&self) -> bool {
        self.executable.is_ok()
    }

    pub fn error(&self) -> Option<&str> {
        self.executable.as_ref().err().map(String::as_str)
    }

    pub fn capture(
        &self,
        instance: &Instance,
        window: &Window,
        params: &Value,
    ) -> Result<Value, Error> {
        let format = params["format"].as_str().unwrap_or("png");
        if !matches!(format, "png" | "jpeg") {
            return Err(Error::new(-32602, "Screenshot format must be png or jpeg"));
        }
        if params.get("clip").is_some()
            || params["captureBeyondViewport"] == true
            || params["fromSurface"] == false
        {
            return Err(Error::new(
                -32602,
                "Native screenshots capture the selected whole window; viewport clipping is unavailable",
            ));
        }
        let quality = match params.get("quality") {
            None => 80,
            Some(n) => n
                .as_u64()
                .filter(|n| *n <= 100)
                .ok_or_else(|| Error::new(-32602, "quality must be within 0–100"))?,
        };
        let executable = self
            .executable
            .as_ref()
            .map_err(|e| Error::new(-32000, e.clone()))?;
        let mut process = Command::new(executable);
        process.args([
            instance.pid.to_string(),
            window.native_id.to_string(),
            window.frame.width.to_string(),
            window.frame.height.to_string(),
            if format == "png" {
                "png".into()
            } else {
                "snapshot-jpeg".into()
            },
            (quality as f64 / 100.).to_string(),
        ]);
        let timeout = rake_client::current_deadline().map_or(TIMEOUT, |deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .min(TIMEOUT)
        });
        let bytes = output(process, timeout, 768 * 1024).map_err(|e| Error::new(-32000, e))?;
        let (metadata, image) = packet(&bytes)?;
        Ok(
            json!({"data":base64::engine::general_purpose::STANDARD.encode(image),"rake":{
                "window":window,"format":format,"image":metadata,
                "scaleX":metadata["width"].as_f64().unwrap_or(0.)/f64::from(window.frame.width),
                "scaleY":metadata["height"].as_f64().unwrap_or(0.)/f64::from(window.frame.height),
                "inspectionGeneration":window.generation,"sampling":"native image and inspection generation are independently sampled"
            }}),
        )
    }
}

impl Drop for Screenshots {
    fn drop(&mut self) {
        if let Some(path) = &self.directory {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn packet(bytes: &[u8]) -> Result<(Value, &[u8]), Error> {
    let fail = || Error::new(-32000, "Invalid native screenshot packet");
    if bytes.len() < 8 {
        return Err(fail());
    }
    let metadata_len = u32::from_be_bytes(bytes[..4].try_into().map_err(|_| fail())?) as usize;
    let image_len = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| fail())?) as usize;
    let start = 8usize.checked_add(metadata_len).ok_or_else(fail)?;
    if metadata_len > 16 * 1024
        || image_len > 750 * 1024
        || start.checked_add(image_len) != Some(bytes.len())
    {
        return Err(fail());
    }
    let metadata: Value = serde_json::from_slice(&bytes[8..start]).map_err(|_| fail())?;
    let image = &bytes[start..];
    let dimensions = metadata["width"]
        .as_u64()
        .is_some_and(|n| (1..=1920).contains(&n))
        && metadata["height"]
            .as_u64()
            .is_some_and(|n| (1..=1440).contains(&n));
    let format_matches = (metadata["kind"] == "png" && image.starts_with(b"\x89PNG\r\n\x1a\n"))
        || (metadata["kind"] == "jpeg" && image.starts_with(&[0xff, 0xd8, 0xff]));
    if !dimensions || !format_matches {
        return Err(fail());
    }
    Ok((metadata, image))
}

fn output(mut command: Command, timeout: Duration, limit: usize) -> Result<Vec<u8>, String> {
    use std::os::unix::process::CommandExt;
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let read = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(16 * 1024).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            result => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    // SAFETY: this child was started in its own process group; cancel its compiler/capture descendants too.
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                break Err(match result {
                    Err(e) => e.to_string(),
                    _ => "Native capture process timed out".into(),
                });
            }
        }
    };
    let bytes = read
        .join()
        .map_err(|_| "Capture reader failed")?
        .map_err(|e| e.to_string())?;
    let error = String::from_utf8_lossy(&errors.join().unwrap_or_default())
        .trim()
        .to_owned();
    let status = status?;
    if bytes.len() > limit {
        return Err(
            "Native screenshot exceeds the CDP response budget; use JPEG or a smaller window"
                .into(),
        );
    }
    if !status.success() {
        return Err(format!("Native capture failed: {status}: {error}"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packets_reject_truncation_and_foreign_payloads() {
        assert!(packet(&[]).is_err());
        let header = br#"{"kind":"png","width":2,"height":2}"#;
        let png = b"\x89PNG\r\n\x1a\n";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&(png.len() as u32).to_be_bytes());
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(png);
        assert_eq!(packet(&bytes).unwrap().1, png);
        bytes.pop();
        assert!(packet(&bytes).is_err());
    }
    #[test]
    fn subprocess_bounds_and_timeout_terminate_owned_work() {
        let mut large = Command::new("sh");
        large.args(["-c", "printf 123456789"]);
        assert!(
            output(large, Duration::from_secs(2), 4)
                .unwrap_err()
                .contains("budget")
        );
        let mut blocked = Command::new("sh");
        blocked.args(["-c", "sleep 30"]);
        assert!(
            output(blocked, Duration::from_millis(20), 1024)
                .unwrap_err()
                .contains("timed out")
        );
    }
}
