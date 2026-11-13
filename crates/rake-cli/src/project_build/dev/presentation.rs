//! Human development output. The supervisor owns state; pipe readers share only the writer.

use super::super::{Args, Compiler, Project};
use serde_json::Value;

use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use termcolor::{Buffer, BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) type Shared = Arc<Mutex<Presentation>>;

// Keep abnormal tools and misconfigured terminal widths from growing output unboundedly.
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
const MAX_COLUMNS: usize = 120;

pub(super) struct Presentation {
    writer: BufferWriter,
    buffer: Buffer,
    enabled: bool,
    interactive: bool,
    unicode: bool,
    progress: usize,
    started: Instant,
    build_started: Instant,
    directory: PathBuf,
    root: PathBuf,
    name: String,
    web: bool,
    live: bool,
    preview: bool,
    header: bool,
    pending: bool,
    diagnostic: bool,
    restarting: bool,
    paths: String,
    url: Option<String>,
    devtools: Option<String>,
    reset_views: Vec<String>,
    diagnostics: String,
    diagnostics_truncated: bool,
    last_notice: Option<(String, String)>,
}

impl Presentation {
    pub(super) fn new(args: &Args) -> Shared {
        let terminal = io::stdout().is_terminal();
        let dumb = std::env::var_os("TERM").is_some_and(|value| value == "dumb");

        let writer = BufferWriter::stdout(if terminal && !dumb {
            ColorChoice::Auto
        } else {
            ColorChoice::Never
        });

        let now = Instant::now();

        let unicode = terminal
            && !dumb
            && (!cfg!(windows)
                || std::env::var_os("WT_SESSION").is_some()
                || std::env::var_os("TERM").is_some());

        Arc::new(Mutex::new(Self {
            buffer: writer.buffer(),
            writer,
            enabled: !args.json && !args.verbose,
            interactive: terminal && io::stderr().is_terminal() && !dumb,
            unicode,
            progress: 0,
            started: now,
            build_started: now,
            directory: PathBuf::new(),
            root: PathBuf::new(),
            name: String::new(),
            web: args.web,
            live: args.compiler == Compiler::Blaze,
            preview: args.preview,
            header: false,
            pending: false,
            diagnostic: false,
            restarting: false,
            paths: String::new(),
            url: None,
            devtools: None,
            reset_views: Vec::new(),
            diagnostics: String::new(),
            diagnostics_truncated: false,
            last_notice: None,
        }))
    }

    pub(super) fn project(&mut self, project: &Project) {
        if !self.enabled {
            return;
        }

        self.directory = project.package["manifest_path"]
            .as_str()
            .and_then(|path| std::path::Path::new(path).parent())
            .unwrap_or(&project.root)
            .to_path_buf();
        self.root = project.root.clone();
        self.name = project
            .package
            .pointer("/metadata/rake/bundle/name")
            .and_then(Value::as_str)
            .or_else(|| project.package["name"].as_str())
            .unwrap_or(&project.target)
            .to_owned();
    }

    pub(super) fn begin(&mut self, paths: &[PathBuf], forced: bool) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        self.pending = true;

        if !forced {
            self.build_started = Instant::now();
            self.diagnostic = false;
            self.restarting = false;
            self.last_notice = None;
            self.paths = paths
                .iter()
                .take(2)
                .map(|path| {
                    path.strip_prefix(&self.directory)
                        .or_else(|_| path.strip_prefix(&self.root))
                        .unwrap_or(path)
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect::<Vec<_>>()
                .join(", ");

            if paths.len() > 2 {
                self.paths.push_str(&format!(" (+{})", paths.len() - 2));
            }
        }

        self.progress(if self.header {
            "Updating..."
        } else {
            "Starting app..."
        })
    }

    pub(super) fn reset_views(&mut self, views: &[String]) {
        if !self.enabled {
            return;
        }

        self.reset_views = views
            .iter()
            .map(|view| view.split(" (entry-").next().unwrap_or(view).to_owned())
            .collect();
    }

    pub(super) fn event(&mut self, value: &Value, available: bool) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        self.buffer.clear();
        self.render(value, available, Instant::now())?;
        self.commit()?;

        if matches!(
            value["event"].as_str(),
            Some(
                "running"
                    | "watching"
                    | "build_failed"
                    | "reload_blocked"
                    | "restore_failed"
                    | "stopped"
            )
        ) {
            self.flush_diagnostics()?;
        }

        if value["event"] == "build_failed" {
            self.buffer.clear();

            if !self.header
                && let Some(url) = self.url.clone()
            {
                self.link("Development page", &url)?;
            }

            self.retained(available)?;
            self.note("Fix the error and save.")?;

            return self.commit();
        }

        Ok(())
    }

    fn commit(&self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        self.writer.print(&self.buffer)
    }

    fn flush_diagnostics(&mut self) -> io::Result<()> {
        if self.diagnostics.is_empty() {
            return Ok(());
        }

        raw_stderr(&self.diagnostics)?;

        if !self.diagnostics.ends_with('\n') {
            raw_stderr("\n")?;
        }

        if self.diagnostics_truncated {
            raw_stderr("Additional build output omitted; use --verbose for full diagnostics.\n")?;
        }

        self.diagnostics.clear();
        self.diagnostics_truncated = false;

        Ok(())
    }

    fn render(&mut self, value: &Value, available: bool, now: Instant) -> io::Result<()> {
        match value["event"].as_str().unwrap_or_default() {
            "web_listening" | "preview_listening" => {
                self.url = value["url"].as_str().map(str::to_owned);
            }
            "cdp_listening" => {
                self.devtools = value["url"].as_str().map(|url| format!("{url}/json/list"));
            }
            "embedded_devtools_listening" => {
                self.url = value["url"].as_str().map(str::to_owned);

                if self.header {
                    self.link("Open", &self.url.clone().unwrap_or_default())?;
                }
            }
            "running" => {
                self.clear_progress()?;

                if !self.header {
                    self.startup(now)?;
                } else if self.pending {
                    self.result("Restarted", Color::Green, now)?;
                }

                self.pending = false;
            }
            "watching" if self.web => {
                self.clear_progress()?;

                if !self.header {
                    self.startup(now)?;
                } else if self.pending {
                    self.result("Update ready", Color::Green, now)?;
                }

                self.pending = false;
            }
            "watching" if self.pending && value["outcome"] == "applied" => {
                self.result("Updated", Color::Green, now)?;
                self.pending = false;
            }
            "live_patch_applied" if self.pending => {
                self.result("Updated", Color::Green, now)?;
                self.pending = false;
            }
            "watching" if self.pending && value["outcome"] == "unchanged" => {
                self.result("Update ready", Color::Blue, now)?;
            }
            "live_patch_declined" if self.pending => {
                self.result("Update not applied", Color::Yellow, now)?;

                if let Some(reason) = value["reason"].as_str() {
                    self.note(reason)?;
                }

                self.retained(available)?;
                self.pending = false;
            }
            "rebuild_required" | "live_relink" if !self.restarting => {
                self.result("Restart needed", Color::Yellow, now)?;
                self.restarting = true;
            }
            "restarted" => {
                self.clear_progress()?;
                let views = if self.reset_views.is_empty() {
                    "App view state".to_owned()
                } else {
                    self.reset_views.join(", ")
                };

                self.buffer
                    .set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))?;
                writeln!(
                    self.buffer,
                    "     {} will reset. No reload state is registered.",
                    clean(&views)
                )?;
                self.buffer.reset()?;
                self.reset_views.clear();
            }
            "restarting_without_state" => {
                self.clear_progress()?;
                self.buffer
                    .set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))?;
                writeln!(
                    self.buffer,
                    "     App view state may reset; restarting without a development connection."
                )?;
                self.buffer.reset()?;
            }
            "build_failed" => {
                self.result("Update failed", Color::Red, now)?;

                if !self.diagnostic
                    && let Some(error) = value["error"].as_str()
                {
                    self.note(error)?;
                }

                self.pending = false;
            }
            "reload_blocked" | "restore_failed" | "reload_waiting" => {
                let phase = value["event"].as_str().unwrap_or_default();
                let error = value["error"].as_str().unwrap_or_default();

                if self
                    .last_notice
                    .as_ref()
                    .is_some_and(|(last_phase, last_error)| {
                        last_phase == phase && last_error == error
                    })
                {
                    return Ok(());
                }

                self.last_notice = Some((phase.to_owned(), error.to_owned()));
                self.result("Update paused", Color::Yellow, now)?;

                if let Some(error) = value["error"].as_str() {
                    self.note(reason(error))?;
                }

                if value["event"] != "restore_failed" {
                    self.retained(available)?;
                }
            }
            "app_exited" => {
                self.clear_progress()?;
                let label = if value["exit_code"] == 0 {
                    "App stopped"
                } else {
                    "App stopped unexpectedly"
                };

                self.result(
                    label,
                    if value["exit_code"] == 0 {
                        Color::Blue
                    } else {
                        Color::Red
                    },
                    now,
                )?;
                self.note("Save a code change to start it again.")?;
                self.pending = false;
            }
            "build_superseded" => {
                self.clear_progress()?;
                self.diagnostics.clear();
            }
            "stopped" => {
                self.clear_progress()?;
            }
            "watching"
                if self.pending
                    && !self.live
                    && value["mechanism"] == "native_code_replacement" =>
            {
                self.result("Updated", Color::Green, now)?;
                self.pending = false;
            }
            _ => {}
        }

        Ok(())
    }

    fn startup(&mut self, now: Instant) -> io::Result<()> {
        writeln!(self.buffer)?;
        self.buffer.set_color(ColorSpec::new().set_bold(true))?;
        write!(self.buffer, "  rake")?;
        self.buffer.reset()?;
        self.buffer.set_color(ColorSpec::new().set_dimmed(true))?;
        write!(self.buffer, " v{}", env!("CARGO_PKG_VERSION"))?;
        self.buffer.reset()?;
        writeln!(self.buffer, "  {}", clean(&self.name))?;
        self.buffer.set_color(ColorSpec::new().set_dimmed(true))?;
        writeln!(
            self.buffer,
            "        {} {}",
            if self.web {
                "Web app serving ·"
            } else {
                "Desktop app started ·"
            },
            duration(now.duration_since(self.started))
        )?;
        self.buffer.reset()?;

        if !self.web {
            if self.live {
                self.buffer
                    .set_color(ColorSpec::new().set_fg(Some(Color::Blue)))?;
                write!(self.buffer, "        Live updates")?;
                self.buffer.reset()?;
                writeln!(self.buffer, " · experimental")?;
            } else {
                writeln!(self.buffer, "        Restarts on save")?;
            }
        }

        if let Some(url) = self.url.clone() {
            self.link(if self.preview { "Preview" } else { "Open" }, &url)?;
        }

        if let Some(url) = self.devtools.clone() {
            self.link("DevTools discovery", &url)?;
        }

        self.buffer.set_color(ColorSpec::new().set_dimmed(true))?;
        writeln!(self.buffer, "\n        Ctrl+C to stop\n")?;
        self.buffer.reset()?;
        self.header = true;

        Ok(())
    }

    fn link(&mut self, label: &str, url: &str) -> io::Result<()> {
        write!(self.buffer, "        {label} ")?;
        self.buffer
            .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
        writeln!(self.buffer, "{}", clean(url))?;

        self.buffer.reset()
    }

    fn result(&mut self, label: &str, color: Color, now: Instant) -> io::Result<()> {
        self.clear_progress()?;
        let symbol = match color {
            Color::Green if self.unicode => "✓",
            Color::Green => "ok",
            Color::Red if self.unicode => "×",
            Color::Red => "x",
            Color::Blue if self.unicode => "·",
            Color::Blue => ".",
            _ => "!",
        };

        let width = columns().saturating_sub(2).clamp(20, MAX_COLUMNS);

        let elapsed = if matches!(color, Color::Green) {
            duration(now.duration_since(self.build_started))
        } else {
            String::new()
        };

        let label_width = UnicodeWidthStr::width(label) + UnicodeWidthStr::width(symbol) + 3;

        let path_width =
            width.saturating_sub(label_width + UnicodeWidthStr::width(elapsed.as_str()) + 4);

        let path = truncate(&clean(&self.paths), path_width);

        self.buffer
            .set_color(ColorSpec::new().set_fg(Some(color)))?;
        write!(self.buffer, "  {symbol} {label}")?;
        self.buffer.reset()?;
        self.buffer.set_color(ColorSpec::new().set_dimmed(true))?;

        if !path.is_empty() {
            write!(self.buffer, "  {path}")?;
        }

        if !elapsed.is_empty() {
            let used = label_width
                + if path.is_empty() {
                    0
                } else {
                    2 + UnicodeWidthStr::width(path.as_str())
                };

            write!(
                self.buffer,
                "{:padding$}{elapsed}",
                "",
                padding = width.saturating_sub(used + elapsed.len()).max(2)
            )?;
        }

        writeln!(self.buffer)?;

        self.buffer.reset()
    }

    fn note(&mut self, text: &str) -> io::Result<()> {
        for line in text.lines() {
            writeln!(self.buffer, "     {}", clean(line))?;
        }

        Ok(())
    }

    fn retained(&mut self, available: bool) -> io::Result<()> {
        if available {
            self.note(if self.web {
                "Last successful build is still being served."
            } else {
                "Previous version is still running."
            })?;
        }

        Ok(())
    }

    fn progress(&mut self, label: &str) -> io::Result<()> {
        if !self.enabled || !self.interactive || columns() < 24 {
            return Ok(());
        }

        self.buffer.clear();
        self.clear_progress()?;
        write!(self.buffer, "  {label}")?;
        self.progress = label.len() + 2;

        self.commit()
    }

    fn clear_progress(&mut self) -> io::Result<()> {
        if self.progress != 0 {
            write!(self.buffer, "\r{:width$}\r", "", width = self.progress)?;
            self.progress = 0;
        }

        Ok(())
    }

    pub(super) fn build_output(&mut self, text: &str) -> io::Result<()> {
        if !self.enabled {
            return raw_stderr(text);
        }

        let message = text
            .trim_start()
            .starts_with('{')
            .then(|| serde_json::from_str::<Value>(text).ok())
            .flatten();

        let structured = message
            .as_ref()
            .and_then(|value| value.pointer("/error/message"))
            .and_then(Value::as_str);

        let text = structured.unwrap_or(text);

        if self.diagnostic
            && [
                "error: could not compile ",
                "rake worker: distribution: Cargo build failed",
                "distribution: worker exited ",
            ]
            .iter()
            .any(|prefix| text.trim_start().starts_with(prefix))
        {
            return Ok(());
        }

        if progress_message(text) {
            return Ok(());
        }

        if structured.is_some() || text.trim_start().starts_with("error") {
            self.diagnostic = true;
        }

        // Bound abnormal tool output. Ordinary progress is discarded before allocation.
        let mut length = text
            .len()
            .min(MAX_DIAGNOSTIC_BYTES.saturating_sub(self.diagnostics.len()));

        while !text.is_char_boundary(length) {
            length -= 1;
        }

        self.diagnostics.push_str(&text[..length]);

        if structured.is_some()
            && !text.ends_with('\n')
            && self.diagnostics.len() < MAX_DIAGNOSTIC_BYTES
        {
            self.diagnostics.push('\n');
        }

        self.diagnostics_truncated |= length < text.len();

        Ok(())
    }

    pub(super) fn app_output(&mut self, text: &str) -> io::Result<()> {
        if !self.enabled {
            return raw_stderr(text);
        }

        self.buffer.clear();
        self.clear_progress()?;
        self.commit()?;
        let mut stderr = io::stderr().lock();

        write!(stderr, "  app  {text}")?;

        stderr.flush()
    }
}

impl Drop for Presentation {
    fn drop(&mut self) {
        self.buffer.clear();

        if self.clear_progress().is_ok() {
            let _ = self.commit();
        }

        let _ = self.flush_diagnostics();
    }
}

fn raw_stderr(text: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();

    stderr.write_all(text.as_bytes())?;

    stderr.flush()
}

fn progress_message(text: &str) -> bool {
    let text = text.trim();

    [
        "Compiling ",
        "Checking ",
        "Finished ",
        "Fresh ",
        "Building ",
        "Bundling ",
        "Packaging ",
        "Signing ",
        "Downloading ",
        "Downloaded ",
        "Updating ",
        "Locking ",
        "blaze time:",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .collect()
}

fn reason(text: &str) -> &str {
    if text.contains("\"Busy\"") {
        "Waiting for the app to finish pending work or decisions."
    } else if text.contains("\"MissingState\"") {
        "A view has no registered reload state. Save your work before restarting."
    } else {
        text.strip_prefix("Blaze restart blocked: ").unwrap_or(text)
    }
}

fn truncate(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }

    if width < 3 {
        return String::new();
    }

    let mut used = 0;
    let mut result = String::new();

    for character in text.chars() {
        used += character.width().unwrap_or(0);

        if used > width - 3 {
            break;
        }

        result.push(character);
    }

    result.push_str("...");

    result
}

fn duration(elapsed: Duration) -> String {
    if elapsed.as_secs() == 0 {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.1} s", elapsed.as_secs_f64())
    }
}

fn columns() -> usize {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();

        // SAFETY: ioctl receives writable space for one winsize; only read it on success.
        if unsafe {
            libc::ioctl(
                io::stdout().as_raw_fd(),
                libc::TIOCGWINSZ,
                size.as_mut_ptr(),
            )
        } == 0
        {
            // SAFETY: the successful TIOCGWINSZ call initialized size.
            let size = unsafe { size.assume_init() };

            if size.ws_col > 0 {
                return usize::from(size.ws_col);
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
        };
        let mut info = std::mem::MaybeUninit::<CONSOLE_SCREEN_BUFFER_INFO>::uninit();

        // SAFETY: stdout is borrowed; the API writes a complete structure on success.
        if unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), info.as_mut_ptr()) }
            != 0
        {
            // SAFETY: the successful console query initialized info.
            let info = unsafe { info.assume_init() };
            let width = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;

            if width > 0 {
                return width as usize;
            }
        }
    }

    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

#[cfg(test)]
mod tests;
