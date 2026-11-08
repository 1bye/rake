//! Platform browser launch candidates, with explicit BROWSER commands first.

use std::{ffi::OsStr, process::Command};

pub(super) fn commands(browser: Option<&OsStr>, url: &str) -> Vec<Command> {
    let mut commands = Vec::new();

    if let Some(browser) = browser {
        let separator = if cfg!(windows) { ';' } else { ':' };

        for entry in browser.to_string_lossy().split(separator) {
            let Some(words) = shlex::split(entry).filter(|words| !words.is_empty()) else {
                continue;
            };

            let supplied = words.iter().any(|word| word.contains("%s"));
            let mut words = words.into_iter().map(|word| word.replace("%s", url));
            let mut command = Command::new(words.next().expect("nonempty browser command"));

            command.args(words);

            if !supplied {
                command.arg(url);
            }

            commands.push(command);
        }
    }

    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");

        command.args(["/c", "start", ""]);

        command
    };

    command.arg(url);
    commands.push(command);

    commands
}

/// Starts a browser opener that the session does not own.
///
/// Openers such as `xdg-open`, or a browser named by `BROWSER`, may become the user's
/// browser process. Like Python's `webbrowser`, a session never terminates them: the opener
/// runs in its own process group, outside the supervisor's groups and the terminal's
/// foreground group, and a detached thread only reaps it.
#[cfg(unix)]
pub(super) fn launch(command: &mut Command) -> std::io::Result<u32> {
    use std::{os::unix::process::CommandExt, process::Stdio};

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;

    let pid = child.id();

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn launched_openers_run_outside_session_process_groups() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("browser.pid");
        let mut command = Command::new("sh");

        command.arg("-c").arg(format!(
            "sleep 30 & echo $! > '{}'; exit 0",
            marker.display()
        ));
        let opener = launch(&mut command).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

        let browser = loop {
            if let Some(pid) = std::fs::read_to_string(&marker)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
            {
                break pid;
            }

            assert!(std::time::Instant::now() < deadline, "opener did not start");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };

        // SAFETY: getpgid and kill only inspect or signal the test's own child processes.
        unsafe {
            assert_eq!(libc::getpgid(browser), opener as i32);
            assert_ne!(libc::getpgid(browser), libc::getpgid(0));
            libc::kill(browser, libc::SIGKILL);
        }
    }

    #[test]
    fn browser_commands_preserve_quoted_arguments_and_substitute_urls_without_a_shell() {
        let url = "http://127.0.0.1:8080/#__rake_control=token";

        let commands = commands(
            Some(OsStr::new(
                "'browser with spaces' --profile 'local test' %s",
            )),
            url,
        );

        assert_eq!(commands[0].get_program(), "browser with spaces");
        assert_eq!(
            commands[0].get_args().collect::<Vec<_>>(),
            ["--profile", "local test", url]
        );

        let commands = super::commands(Some(OsStr::new("browser --new-window")), url);

        assert_eq!(
            commands[0].get_args().collect::<Vec<_>>(),
            ["--new-window", url]
        );
        assert_eq!(commands.last().unwrap().get_args().last().unwrap(), url);
    }
}
