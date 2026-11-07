//! Portable Windows command construction; tested on every host.
use crate::{Error, LaunchAtLogin};

pub(crate) const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub(crate) fn value_name(login: &LaunchAtLogin) -> &str {
    &login.app_id
}

// CommandLineToArgvW / CRT quoting: double backslashes before quotes and the closing quote.
fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;

    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }

        result.extend(std::iter::repeat_n(
            '\\',
            if ch == '"' { slashes * 2 + 1 } else { slashes },
        ));
        result.push(ch);
        slashes = 0;
    }

    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');

    result
}

pub(crate) fn command(login: &LaunchAtLogin, executable: &str) -> Result<String, Error> {
    if executable.is_empty() || executable.contains(['\0', '\r', '\n', '"']) {
        return Err(Error::InvalidCommand(
            "invalid Windows executable path".into(),
        ));
    }

    // current_exe may use an extended-length spelling while installer locators do not.
    let normalized;

    let executable = if let Some(unc) = executable.strip_prefix(r"\\?\UNC\") {
        normalized = format!(r"\\{unc}");

        normalized.as_str()
    } else {
        executable.strip_prefix(r"\\?\").unwrap_or(executable)
    };

    let mut command = quote(executable);

    for arg in &login.args {
        command.push(' ');
        command.push_str(&quote(arg));
    }

    // Run documents a 260-character command-line limit.
    if command.encode_utf16().count() > 260 {
        return Err(Error::InvalidCommand(
            "Windows Run command exceeds 260 UTF-16 code units".into(),
        ));
    }

    Ok(command)
}

pub(crate) fn belongs_to(command: &str, prefix: &str) -> bool {
    command.eq_ignore_ascii_case(prefix)
        || command
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            && command.as_bytes().get(prefix.len()) == Some(&b' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_matches_only_the_installed_executable_including_login_args() {
        let login = LaunchAtLogin::new("dev.rake.app").unwrap();
        let prefix = command(&login, r"C:\Program Files\Rake\app.exe").unwrap();

        assert!(belongs_to(&format!("{prefix} \"--background\""), &prefix));
        assert!(belongs_to(&prefix.to_lowercase(), &prefix));
        assert!(!belongs_to(r#""C:\Other\app.exe""#, &prefix));
        assert!(!belongs_to(&format!("{prefix}.other"), &prefix));
        assert_eq!(
            command(&login, r"\\?\C:\Program Files\Rake\app.exe").unwrap(),
            prefix
        );
        assert_eq!(
            command(&login, r"\\?\UNC\server\share\app.exe").unwrap(),
            r#""\\server\share\app.exe""#
        );
    }

    #[test]
    fn value_name_and_command_preserve_identity_spaces_quotes_and_trailing_slashes() {
        let login = LaunchAtLogin::new("dev.rake.app")
            .unwrap()
            .args(["", "a b", r#"say "hello""#, r"C:\tail\"])
            .unwrap();

        assert_eq!(value_name(&login), "dev.rake.app");
        assert_eq!(RUN_KEY, r"Software\Microsoft\Windows\CurrentVersion\Run");
        assert_eq!(
            command(&login, r"C:\Program Files\Rake\app.exe").unwrap(),
            r#""C:\Program Files\Rake\app.exe" "" "a b" "say \"hello\"" "C:\tail\\""#
        );
        assert!(command(&login, &"x".repeat(261)).is_err());
    }
}
