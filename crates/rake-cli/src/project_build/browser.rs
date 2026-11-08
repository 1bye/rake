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

#[cfg(test)]
mod tests {
    use super::*;

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
