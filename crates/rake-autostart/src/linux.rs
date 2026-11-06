//! XDG autostart files and sandbox-aware portal requests.
use crate::{Error, LaunchAtLogin, Operation, Status};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(crate) async fn apply(login: &LaunchAtLogin, operation: Operation) -> Result<Status, Error> {
    if Path::new("/.flatpak-info").exists() {
        return portal(login, operation).await;
    }
    if env::var_os("SNAP").is_some() {
        return Ok(Status::Unsupported(
            "Snap confinement requires snap-specific startup integration".into(),
        ));
    }

    let config = config_home(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))?;
    let executable = env::current_exe()?;
    let appimage = env::var_os("APPIMAGE").map(PathBuf::from);
    apply_xdg(
        login,
        operation,
        &config,
        launch_path(&executable, appimage.as_deref())?,
    )
}

fn config_home(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf, Error> {
    if let Some(xdg) = xdg.filter(|p| !p.is_empty()) {
        let path = PathBuf::from(xdg);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    home.map(|p| PathBuf::from(p).join(".config"))
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            Error::InvalidCommand("XDG_CONFIG_HOME or HOME must be an absolute path".into())
        })
}

fn launch_path<'a>(executable: &'a Path, appimage: Option<&'a Path>) -> Result<&'a Path, Error> {
    let path = appimage.unwrap_or(executable);
    if !path.is_absolute() {
        return Err(Error::InvalidCommand(
            "executable/APPIMAGE must be absolute".into(),
        ));
    }

    Ok(path)
}

// Desktop string decoding precedes Exec unquoting; both layers need escaping.
fn exec_arg(value: &str) -> Result<String, Error> {
    if value.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidCommand(
            "desktop Exec contains NUL or line breaks".into(),
        ));
    }
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str(r"\\\\"),
            '"' => quoted.push_str(r#"\\""#),
            '$' => quoted.push_str(r"\\$"),
            '`' => quoted.push_str(r"\\`"),
            '%' => quoted.push_str("%%"),
            '\t' => quoted.push_str(r"\t"),
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');

    Ok(quoted)
}

fn desktop_entry(login: &LaunchAtLogin, executable: &Path) -> Result<String, Error> {
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::InvalidCommand("desktop executable path is not UTF-8".into()))?;
    // Desktop Entry §7 forbids '=' in the executable name/path, even when quoted.
    // https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html
    if executable.contains('=') {
        return Err(Error::InvalidCommand(
            "desktop executable path cannot contain '='".into(),
        ));
    }
    let mut command = exec_arg(executable)?;
    for arg in &login.args {
        command.push(' ');
        command.push_str(&exec_arg(arg)?);
    }

    Ok(format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={}\nExec={command}\nTerminal=false\n",
        login.app_id
    ))
}

fn apply_xdg(
    login: &LaunchAtLogin,
    operation: Operation,
    config: &Path,
    executable: &Path,
) -> Result<Status, Error> {
    let directory = config.join("autostart");
    let path = directory.join(format!("{}.desktop", login.app_id));
    match operation {
        Operation::Enable => {
            let entry = desktop_entry(login, executable)?;
            fs::create_dir_all(&directory)?;
            let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
            temporary.write_all(entry.as_bytes())?;
            temporary.as_file().sync_all()?;
            temporary
                .persist(&path)
                .map_err(|error| Error::Io(error.error))?;
        }
        Operation::Disable => match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        },
        Operation::Status => {}
    }

    match fs::read_to_string(path) {
        Ok(entry) => {
            let mut desktop = false;
            let mut hidden = false;
            for line in entry.lines().map(str::trim) {
                if line.starts_with('[') {
                    desktop = line == "[Desktop Entry]";
                }
                if desktop
                    && line.split_once('=').is_some_and(|(key, value)| {
                        key.trim() == "Hidden" && value.trim() == "true"
                    })
                {
                    hidden = true;
                }
            }
            Ok(if hidden {
                Status::Disabled
            } else {
                Status::Enabled
            })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Status::Disabled),
        Err(error) => Err(error.into()),
    }
}

async fn portal(login: &LaunchAtLogin, operation: Operation) -> Result<Status, Error> {
    use futures::StreamExt;
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};

    if matches!(operation, Operation::Status) {
        return Ok(Status::Unsupported("the Flatpak Background portal has no read-only autostart status API; use the enable/disable response".into()));
    }

    let request = async {
        let connection = zbus::Connection::session().await?;
        let background = zbus::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Background",
        )
        .await?;
        static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let request = NEXT_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let token = format!("rake_{}_{request}", std::process::id());
        let sender = connection
            .unique_name()
            .ok_or_else(|| zbus::Error::Failure("session bus has no unique name".into()))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let response = zbus::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            path.as_str(),
            "org.freedesktop.portal.Request",
        )
        .await?;
        // Subscribe before the method call so immediate responses cannot race the listener.
        let mut responses = response.receive_signal("Response").await?;
        let enable = matches!(operation, Operation::Enable);
        let mut options: HashMap<&str, Value<'_>> = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            (
                "reason",
                Value::from("Start this application when you log in"),
            ),
            ("autostart", Value::from(enable)),
        ]);
        // Omit commandline when no custom args: the portal uses the packaged desktop Exec.
        let commandline;
        if !login.args.is_empty() {
            commandline = std::iter::once(env::current_exe()?.to_string_lossy().into_owned())
                .chain(login.args.iter().cloned())
                .collect::<Vec<_>>();
            options.insert("commandline", Value::from(commandline.as_slice()));
        }
        let returned: zbus::zvariant::OwnedObjectPath =
            background.call("RequestBackground", &("", options)).await?;
        if returned.as_str() != path {
            return Err(Error::Service(
                "portal returned an unexpected request path".into(),
            ));
        }
        let message = responses
            .next()
            .await
            .ok_or_else(|| Error::Service("portal closed without a response".into()))?;
        let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
        if code != 0 {
            return Ok(Status::RequiresApproval);
        }
        let autostart = results
            .get("autostart")
            .and_then(|value| bool::try_from(value).ok())
            .ok_or_else(|| Error::Service("portal response omitted autostart".into()))?;

        Ok(if autostart {
            Status::Enabled
        } else if enable {
            Status::RequiresApproval
        } else {
            Status::Disabled
        })
    };
    request.await
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Service(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

    #[test]
    fn filesystem_roundtrip_idempotence_hidden_and_atomic_replacement() {
        let config = tempfile::tempdir().unwrap();
        let login = LaunchAtLogin::new("dev.rake.test").unwrap();
        let executable = Path::new("/opt/Rake App/app");
        let apply = |operation| apply_xdg(&login, operation, config.path(), executable).unwrap();
        assert_eq!(apply(Operation::Status), Status::Disabled);
        assert_eq!(apply(Operation::Disable), Status::Disabled);
        assert_eq!(apply(Operation::Enable), Status::Enabled);
        let path = config.path().join("autostart/dev.rake.test.desktop");
        let old = fs::File::open(&path).unwrap();
        let inode = old.metadata().unwrap().ino();
        assert_eq!(apply(Operation::Enable), Status::Enabled);
        assert_ne!(
            inode,
            fs::metadata(&path).unwrap().ino(),
            "replace the inode rather than truncate a reader's file"
        );
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1,
            "no temporary files remain"
        );
        fs::write(&path, "[Desktop Entry]\nHidden=true\n").unwrap();
        assert_eq!(apply(Operation::Status), Status::Disabled);
        assert_eq!(apply(Operation::Enable), Status::Enabled);
        assert_eq!(apply(Operation::Disable), Status::Disabled);
        assert_eq!(apply(Operation::Disable), Status::Disabled);
    }

    #[test]
    fn atomic_replace_does_not_follow_symlinks_and_cleans_failed_temporary_files() {
        let config = tempfile::tempdir().unwrap();
        let directory = config.path().join("autostart");
        fs::create_dir(&directory).unwrap();
        let foreign = config.path().join("foreign");
        fs::write(&foreign, "must survive").unwrap();
        let path = directory.join("dev.rake.test.desktop");
        symlink(&foreign, &path).unwrap();
        let login = LaunchAtLogin::new("dev.rake.test").unwrap();

        apply_xdg(
            &login,
            Operation::Enable,
            config.path(),
            Path::new("/opt/app"),
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&foreign).unwrap(), "must survive");
        assert!(!path.symlink_metadata().unwrap().file_type().is_symlink());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        assert!(
            apply_xdg(
                &login,
                Operation::Enable,
                config.path(),
                Path::new("/opt/app")
            )
            .is_err()
        );
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            1,
            "failure cleans its temp file"
        );
    }

    #[test]
    fn equals_is_valid_in_arguments_but_not_the_desktop_executable_path() {
        let login = LaunchAtLogin::new("dev.rake.test")
            .unwrap()
            .args(["--profile=a=b"])
            .unwrap();

        let entry = desktop_entry(&login, Path::new("/opt/Rake App/app")).unwrap();

        assert!(
            entry.contains(r#"Exec="/opt/Rake App/app" "--profile=a=b""#),
            "{entry}"
        );
        assert!(matches!(
            desktop_entry(&login, Path::new("/opt/Rake=App/app")),
            Err(Error::InvalidCommand(_))
        ));
    }

    #[test]
    fn desktop_escaping_appimage_and_config_resolution() {
        let config = tempfile::tempdir().unwrap();
        let login = LaunchAtLogin::new("dev.rake.test")
            .unwrap()
            .args(["a b", "quote\" slash\\ dollar$ tick` percent%", ""])
            .unwrap();
        let executable = Path::new("/tmp/mount/AppRun");
        let image = Path::new("/home/test/My $App% \"x\".AppImage");
        let selected = launch_path(executable, Some(image)).unwrap();
        apply_xdg(&login, Operation::Enable, config.path(), selected).unwrap();
        let text =
            fs::read_to_string(config.path().join("autostart/dev.rake.test.desktop")).unwrap();
        assert!(text.contains(r#"Exec="/home/test/My \\$App%% \\"x\\".AppImage" "a b" "quote\\" slash\\\\ dollar\\$ tick\\` percent%%" """#), "{text}");
        assert!(!text.contains("/tmp/mount"));
        assert_eq!(launch_path(executable, None).unwrap(), executable);
        assert_eq!(
            config_home(Some("/tmp/xdg".into()), Some("/home/test".into())).unwrap(),
            Path::new("/tmp/xdg")
        );
        assert_eq!(
            config_home(Some("relative".into()), Some("/home/test".into())).unwrap(),
            Path::new("/home/test/.config")
        );
        assert!(desktop_entry(&login, Path::new("/tmp/bad=path")).is_err());
        assert!(exec_arg("bad\npath").is_err());
    }
}
