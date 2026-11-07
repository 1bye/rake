//! Argument compatibility with the embedded dev.py build/doctor entry points.

use super::{PathBuf, resolve};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Args {
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
            if positional.is_some() || index != 1 {
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
            print!("{}", include_str!("help.txt"));
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
            return Err(format!(
                "argument --target: invalid choice: '{value}' (choose from 'desktop', 'web')"
            ));
        }
        if option == "--routing" && !["path", "hash"].contains(&value.as_str()) {
            return Err(format!(
                "argument --routing: invalid choice: '{value}' (choose from 'path', 'hash')"
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
        return Err(format!(
            "argument status: invalid choice: '{status}' (choose from 'status')"
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
        ("--elements", "--elements applies to inspect"),
    ] {
        if values.contains_key(flag) {
            return Err(message.into());
        }
    }
    if values.contains_key("--base-path") || values.contains_key("--routing") {
        return Err("--base-path/--routing require bundle --target web".into());
    }
    if values.contains_key("--sign-identity") || values.contains_key("--entitlements") {
        return Err("--sign-identity/--entitlements require a macOS desktop bundle".into());
    }
    if positional.is_some() {
        return Err("use rake dev status --session PATH".into());
    }
    if command != "doctor" {
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
    let output = if command != "doctor" {
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
        release: profile == Some("--release"),
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
            "{}\ndev.py: error: {error}",
            help.split("\n\n").next().unwrap_or_default()
        );
    }
}
