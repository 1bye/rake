//! Argument compatibility with the embedded dev.py build/doctor entry points.

use super::{PathBuf, resolve};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub(super) struct Args {
    pub(super) dev: bool,
    pub(super) bundle: bool,
    pub(super) base_path: Option<String>,
    pub(super) routing: Option<String>,
    pub(super) cdp: bool,
    pub(super) preview: bool,
    pub(super) no_open: bool,
    pub(super) port: u16,
    pub(super) restart_without_state: bool,
    pub(super) status: bool,
    pub(super) session: Option<PathBuf>,
    pub(super) web: bool,
    pub(super) manifest: PathBuf,
    pub(super) package: Option<String>,
    pub(super) binary: Option<String>,
    pub(super) features: Option<String>,
    pub(super) release: bool,
    pub(super) web_dir: Option<PathBuf>,
    pub(super) output: Option<PathBuf>,
    pub(super) json: bool,
    pub(super) native_reloader: Option<PathBuf>,
}

const OPTIONS: &[(&str, bool)] = &[
    ("--target", true),
    ("--manifest-path", true),
    ("--package", true),
    ("--bin", true),
    ("--features", true),
    ("--release", false),
    ("--debug", false),
    ("--web-dir", true),
    ("--output", true),
    ("--native-reloader", true),
    ("--restart-without-state", false),
    ("--port", true),
    ("--instance", true),
    ("--cdp", false),
    ("--preview", false),
    ("--elements", false),
    ("--no-open", false),
    ("--json", false),
    ("--session", true),
    ("--base-path", true),
    ("--routing", true),
    ("--sign-identity", true),
    ("--entitlements", true),
    ("--help", false),
];

pub(super) fn parse(
    command: &str,
    arguments: &[String],
) -> std::result::Result<Option<Args>, String> {
    let mut values = BTreeMap::new();
    let mut positional = None;
    let mut profile = None;
    let mut unknown = Vec::new();
    let mut index = 0;

    while index < arguments.len() {
        let argument = &arguments[index];

        index += 1;

        if argument == "--" {
            if index == arguments.len() {
                break;
            }

            if index + 1 == arguments.len() && positional.is_none() {
                positional = Some(arguments[index].clone());
                break;
            }

            return Err(format!(
                "unrecognized arguments: {}",
                arguments[index..].join(" ")
            ));
        }

        if !argument.starts_with('-') {
            if positional.is_some() {
                unknown.push(argument.clone());
                continue;
            }

            positional = Some(argument.clone());
            continue;
        }

        let (name, attached) = if argument == "-h" {
            ("--help", None)
        } else if argument.starts_with("-p") && !argument.starts_with("--") {
            (
                "--package",
                (!argument[2..].is_empty()).then_some(&argument[2..]),
            )
        } else {
            argument
                .split_once('=')
                .map_or((argument.as_str(), None), |(name, value)| {
                    (name, Some(value))
                })
        };

        let matching: Vec<_> = OPTIONS
            .iter()
            .filter(|(option, _)| *option == name)
            .collect();

        let matching = if matching.is_empty() && name.starts_with("--") {
            OPTIONS
                .iter()
                .filter(|(option, _)| option.starts_with(name))
                .collect()
        } else {
            matching
        };

        if matching.len() > 1 {
            return Err(format!(
                "ambiguous option: {name} could match {}",
                matching
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let Some(&&(option, takes_value)) = matching.first() else {
            unknown.push(argument.clone());
            continue;
        };

        if option == "--help" {
            print!(
                "{}",
                if command == "dev" {
                    include_str!("dev/help.txt")
                } else {
                    include_str!("help.txt")
                }
            );

            return Ok(None);
        }

        let value = if takes_value {
            if let Some(value) = attached {
                value.to_owned()
            } else {
                let value = arguments
                    .get(index)
                    .filter(|value| {
                        !value.starts_with('-') || value.parse::<i64>().is_ok() || *value == "-"
                    })
                    .ok_or_else(|| format!("argument {option}: expected one argument"))?;

                index += 1;

                value.clone()
            }
        } else {
            if let Some(value) = attached {
                return Err(format!(
                    "argument {option}: ignored explicit argument '{value}'"
                ));
            }

            String::new()
        };

        if matches!(option, "--release" | "--debug") {
            if let Some(previous) = profile
                && previous != option
            {
                return Err(format!(
                    "argument {option}: not allowed with argument {previous}"
                ));
            }

            profile = Some(option);
        }

        if option == "--target" && !["desktop", "web"].contains(&value.as_str()) {
            let choices = if command == "dev" {
                "desktop, web"
            } else {
                "'desktop', 'web'"
            };

            return Err(format!(
                "argument --target: invalid choice: '{value}' (choose from {choices})"
            ));
        }

        if option == "--routing" && !["path", "hash"].contains(&value.as_str()) {
            let choices = if command == "dev" {
                "path, hash"
            } else {
                "'path', 'hash'"
            };

            return Err(format!(
                "argument --routing: invalid choice: '{value}' (choose from {choices})"
            ));
        }

        if option == "--base-path" {
            super::web::valid_base(&serde_json::Value::String(value.clone()))
                .map_err(|_| format!("argument --base-path: invalid base_path value: '{value}'"))?;
        }

        if option == "--port" && value.trim().parse::<i64>().is_err() {
            return Err(format!("argument --port: invalid int value: '{value}'"));
        }

        values.insert(option, value);
    }

    if let Some(status) = &positional
        && status != "status"
    {
        let choices = if command == "dev" {
            "status"
        } else {
            "'status'"
        };

        return Err(format!(
            "argument status: invalid choice: '{status}' (choose from {choices})"
        ));
    }

    if !unknown.is_empty() {
        return Err(format!("unrecognized arguments: {}", unknown.join(" ")));
    }

    let web = values.get("--target").is_some_and(|value| value == "web");

    for (flag, message) in [
        (
            "--preview",
            "--preview requires rake dev --target desktop on macOS 14 or later",
        ),
        ("--cdp", "--cdp applies to dev"),
        (
            "--elements",
            if command == "dev" {
                "The Rake inspector is deprecated; use Chrome DevTools with --cdp"
            } else {
                "--elements applies to inspect"
            },
        ),
    ] {
        if flag == "--elements" && command == "dev" && web && values.contains_key("--cdp") {
            let version = std::process::Command::new("node").arg("--version").output();

            let valid = version
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|version| {
                    version
                        .trim()
                        .trim_start_matches('v')
                        .split('.')
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
                .is_some_and(|major| major >= 22);

            if !valid {
                return Err("Web Chrome DevTools requires Node 22 or later".into());
            }
        }

        if values.contains_key(flag)
            && !(command == "dev"
                && (flag == "--cdp" || (flag == "--preview" && !web && cfg!(target_os = "macos"))))
        {
            return Err(message.into());
        }
    }

    if command == "dev"
        && values
            .get("--package")
            .is_some_and(|name| name == "rake-inspector")
    {
        return Err(
            "The Rake inspector is deprecated and cannot be built or bundled; use Chrome DevTools"
                .into(),
        );
    }

    if (values.contains_key("--base-path") || values.contains_key("--routing"))
        && !(command == "bundle" && web)
    {
        return Err("--base-path/--routing require bundle --target web".into());
    }

    if values.contains_key("--sign-identity") || values.contains_key("--entitlements") {
        return Err("--sign-identity/--entitlements require a macOS desktop bundle".into());
    }

    if positional.is_some() && (command != "dev" || !values.contains_key("--session")) {
        return Err("use rake dev status --session PATH".into());
    }

    if command != "doctor" && positional.is_none() {
        if let Some(port) = values.get("--port") {
            let port: i64 = port.trim().parse().map_err(|_| "port must be 0–65535")?;

            if !(0..=65535).contains(&port) {
                return Err("port must be 0–65535".into());
            }
        }

        if values.contains_key("--native-reloader") && web {
            return Err("--native-reloader applies to desktop only".into());
        }
    }

    let output = if command != "doctor" && positional.is_none() {
        values
            .get("--output")
            .map(|value| {
                let path = PathBuf::from(value);

                if path.is_symlink() {
                    return Err("output cannot be a symlink".into());
                }

                resolve(&path).map_err(|error| error.to_string())
            })
            .transpose()?
    } else {
        None
    };

    Ok(Some(Args {
        dev: command == "dev",
        bundle: command == "bundle",
        base_path: values.remove("--base-path"),
        routing: values.remove("--routing"),
        cdp: values.contains_key("--cdp"),
        preview: values.contains_key("--preview"),
        no_open: values.contains_key("--no-open"),
        port: values
            .get("--port")
            .and_then(|port| port.trim().parse().ok())
            .unwrap_or(if command == "dev" && web { 8081 } else { 0 }),
        restart_without_state: values.contains_key("--restart-without-state"),
        status: positional.is_some(),
        session: values.get("--session").map(PathBuf::from),
        web,
        manifest: values
            .get("--manifest-path")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("Cargo.toml")),
        package: values.remove("--package"),
        binary: values.remove("--bin"),
        features: values
            .remove("--features")
            .filter(|value| !value.is_empty()),
        release: profile.map_or(command == "bundle", |profile| profile == "--release"),
        web_dir: values.get("--web-dir").map(PathBuf::from),
        output,
        json: values.contains_key("--json"),
        native_reloader: values.get("--native-reloader").map(PathBuf::from),
    }))
}

pub(super) fn report_error(arguments: &[String], error: &str) {
    if arguments.iter().any(|argument| argument == "--json") {
        super::report_json_error(error);
    } else {
        let help = include_str!("help.txt");

        eprintln!(
            "{}\nrake: error: {error}",
            help.split("\n\n").next().unwrap_or_default()
        );
    }
}

/// Inspect target selection without parsing twice, consuming values of the shared option inventory.
pub(super) fn web_target(arguments: &[String]) -> bool {
    let mut web = false;
    let mut index = 0;

    while let Some(argument) = arguments.get(index) {
        index += 1;

        if argument == "--" {
            break;
        }

        let (name, attached) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(name, value)| {
                (name, Some(value))
            });

        let exact = OPTIONS.iter().find(|(option, _)| *option == name);

        let matches: Vec<_> = OPTIONS
            .iter()
            .filter(|(option, _)| name.starts_with("--") && option.starts_with(name))
            .collect();

        let option = exact.or_else(|| {
            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        });

        let Some(&(option, takes_value)) = option else {
            continue;
        };

        let value = if takes_value && attached.is_none() {
            let value = arguments.get(index).map(String::as_str);

            index += 1;

            value
        } else {
            attached
        };

        if option == "--target" {
            web = value == Some("web");
        }
    }

    web
}
