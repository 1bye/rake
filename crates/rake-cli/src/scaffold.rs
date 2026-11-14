//! Offline project creation from embedded, native/browser templates.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const HELP: &str = "rake new NAME [--rake-path PATH | --rake-rev REV] [--git] [--json]
Creates a routed desktop/browser app in NAME, an absent or empty directory.
PATH is the local crates/rake directory. REV must be a full Git commit hash.
Without either option, use the tested SDK release or the CLI checkout revision.";

const FILES: &[(&str, &str)] = &[
    ("AGENTS.md", include_str!("../templates/default/AGENTS.md")),
    ("CLAUDE.md", include_str!("../templates/default/CLAUDE.md")),
    (
        "Cargo.toml",
        include_str!("../templates/default/Cargo.toml.template"),
    ),
    (
        "rust-toolchain.toml",
        include_str!("../templates/default/rust-toolchain.toml"),
    ),
    (
        "src/lib.rs",
        include_str!("../templates/default/src/lib.rs"),
    ),
    (
        "src/main.rs",
        include_str!("../templates/default/src/main.rs"),
    ),
    (
        "web/index.html",
        include_str!("../templates/default/web/index.html"),
    ),
    (".gitignore", include_str!("../templates/default/gitignore")),
    ("README.md", include_str!("../templates/default/README.md")),
];

fn registry_dependency() -> Result<String> {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml"))?;

    let version = manifest
        .get("dev-dependencies")
        .and_then(|dependencies| dependencies.get("rake"))
        .and_then(|sdk| sdk.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("CLI manifest is missing its tested SDK version")?;

    Ok(format!(
        "{{ package = \"rake-app\", version = \"{version}\" }}"
    ))
}

fn validate_name(name: &str) -> Result<()> {
    const RESERVED: &[&str] = &[
        "as",
        "async",
        "await",
        "break",
        "const",
        "continue",
        "crate",
        "dyn",
        "else",
        "enum",
        "extern",
        "false",
        "fn",
        "for",
        "gen",
        "if",
        "impl",
        "in",
        "let",
        "loop",
        "match",
        "mod",
        "move",
        "mut",
        "pub",
        "ref",
        "return",
        "self",
        "Self",
        "static",
        "struct",
        "super",
        "trait",
        "true",
        "type",
        "unsafe",
        "use",
        "where",
        "while",
        "abstract",
        "become",
        "box",
        "do",
        "final",
        "macro",
        "override",
        "priv",
        "try",
        "typeof",
        "unsized",
        "virtual",
        "yield",
        "rake",
        "std",
        "core",
        "alloc",
        "proc-macro",
        "proc_macro",
        "test",
        "build",
        "deps",
        "examples",
        "incremental",
    ];
    let valid_start = name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    let windows_name = name.to_ascii_uppercase();

    let windows_reserved = ["CON", "PRN", "AUX", "NUL"].contains(&windows_name.as_str())
        || (1..=9).any(|number| {
            windows_name == format!("COM{number}") || windows_name == format!("LPT{number}")
        });

    if !valid_start
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || RESERVED.contains(&name)
        || windows_reserved
    {
        return Err("NAME must be a portable Cargo package name: start with an ASCII letter, use only letters, digits, hyphens or underscores (at most 64 characters), and avoid reserved names".into());
    }

    Ok(())
}

fn validate_revision(revision: &str) -> Result<()> {
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("--rake-rev must be a full 40-character Git commit hash".into());
    }

    Ok(())
}

fn value(arguments: &mut std::slice::Iter<'_, String>, option: &str) -> Result<String> {
    arguments
        .next()
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("{option} requires a value").into())
}

// Only created files and empty created directories are removed on a failed write.
struct Created {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    complete: bool,
}

impl Drop for Created {
    fn drop(&mut self) {
        if !self.complete {
            for file in self.files.iter().rev() {
                let _ = fs::remove_file(file);
            }

            for directory in self.directories.iter().rev() {
                let _ = fs::remove_dir(directory);
            }
        }
    }
}

fn install(directory: &Path, files: &[(&str, String)], git: Option<&Path>) -> Result<()> {
    let mut created = Created {
        files: Vec::new(),
        directories: Vec::new(),
        complete: false,
    };

    match fs::symlink_metadata(directory) {
        Ok(metadata) => {
            if !metadata.is_dir() || fs::read_dir(directory)?.next().transpose()?.is_some() {
                return Err(format!(
                    "refusing to write into existing non-empty directory or non-directory: {}",
                    directory.display()
                )
                .into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(directory)?;
            created.directories.push(directory.to_owned());
        }
        Err(error) => return Err(error.into()),
    }

    for name in [
        "src",
        "web",
        ".agents",
        ".agents/skills",
        ".agents/skills/rake-gpui",
    ] {
        let path = directory.join(name);

        fs::create_dir(&path)?;
        created.directories.push(path);
    }

    for (name, content) in files {
        let path = directory.join(name);

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        created.files.push(path);
        file.write_all(content.as_bytes())?;
    }

    if let Some(git) = git {
        let destination = directory.join(".git");

        if destination.symlink_metadata().is_ok() {
            return Err("refusing to replace an existing .git".into());
        }

        // Git runs in a private staging directory; a failed init touches no user files.
        fs::rename(git, destination)?;
    }

    created.complete = true;

    Ok(())
}

pub(crate) fn run(arguments: &[String]) -> Result<()> {
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("{HELP}");

        return Ok(());
    }

    let mut name = None;
    let mut rake_path = None;
    let mut revision = None;
    let mut git = false;
    let mut json = false;
    let mut arguments = arguments.iter();

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--rake-path" if rake_path.is_none() => {
                rake_path = Some(value(&mut arguments, argument)?)
            }
            "--rake-rev" if revision.is_none() => revision = Some(value(&mut arguments, argument)?),
            "--git" if !git => git = true,
            "--json" if !json => json = true,
            argument if !argument.starts_with('-') && name.is_none() => name = Some(argument),
            _ => return Err(format!("unexpected new argument: {argument}\n{HELP}").into()),
        }
    }

    let name = name.ok_or(HELP)?;

    validate_name(name)?;

    if rake_path.is_some() && revision.is_some() {
        return Err("--rake-path and --rake-rev cannot be combined".into());
    }

    let dependency = if let Some(path) = rake_path {
        let path = Path::new(&path).canonicalize()?;

        if !path.join("Cargo.toml").is_file() {
            return Err("--rake-path must contain Rake's Cargo.toml (usually crates/rake)".into());
        }

        format!(
            "{{ package = \"rake-app\", path = {} }}",
            serde_json::to_string(&path)?
        )
    } else if let Some(revision) = revision
        .as_deref()
        .or(option_env!("RAKE_BUILD_REV").filter(|revision| !revision.is_empty()))
    {
        validate_revision(revision)?;

        format!(
            "{{ package = \"rake-app\", git = \"https://github.com/1bye/rake\", rev = \"{revision}\" }}"
        )
    } else {
        registry_dependency()?
    };

    let crate_name = name.replace('-', "_");

    let application_id = format!(
        "com.example.{}",
        name.replace('_', "-").to_ascii_lowercase()
    );

    let parent = std::env::current_dir()?;
    let directory = parent.join(name);

    let placeholders = regex::Regex::new(r"\{\{(name|crate_name|application_id|dependency)\}\}")?;

    let mut files: Vec<_> = FILES
        .iter()
        .map(|(path, content)| {
            let content = placeholders.replace_all(content, |capture: &regex::Captures<'_>| {
                match &capture[1] {
                    "name" => name,
                    "crate_name" => &crate_name,
                    "application_id" => &application_id,
                    "dependency" => &dependency,
                    _ => unreachable!("only declared template placeholders match"),
                }
                .to_owned()
            });

            (*path, content.into_owned())
        })
        .collect();

    files.push((
        ".agents/skills/rake-gpui/SKILL.md",
        crate::lint::agent_skill(),
    ));

    let staging = if git {
        let staging = tempfile::Builder::new()
            .prefix(".rake-new-")
            .tempdir_in(&parent)?;

        let output = Command::new("git")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .args(["init", "--quiet", "--template="])
            .arg(staging.path())
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Some(staging)
    } else {
        None
    };

    let staged_git = staging.as_ref().map(|staging| staging.path().join(".git"));

    install(&directory, &files, staged_git.as_deref())?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "name": name, "path": directory, "application_id": application_id,
                "git": git, "next_steps": [format!("cd {name}"), "rake dev".to_owned(), "rake dev --target web".to_owned()]
            })
        );
    } else {
        println!(
            "Created {}\n\n  cd {name}\n  rake dev\n  rake dev --target web",
            directory.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    //! Covers: registry SDK selection, portable names and rollback after a partial write.
    //! Real: name validation and filesystem. Doubles: conflicting template input.
    //! Requires: temporary directory access; no network or compiler.
    use super::*;

    #[test]
    fn registry_scaffold_uses_the_verified_sdk_release() {
        let dependency: toml::Value =
            toml::from_str(&format!("rake = {}", registry_dependency().unwrap())).unwrap();

        assert_eq!(dependency["rake"]["package"].as_str(), Some("rake-app"));
        assert_eq!(
            dependency["rake"]["version"].as_str(),
            Some("=0.1.0-alpha.5")
        );
    }

    #[test]
    fn validates_portable_cargo_names() {
        for name in ["my-app", "my_app", "Hello2"] {
            validate_name(name).unwrap();
        }

        for name in [
            "",
            ".",
            "..",
            "../app",
            "foo/bar",
            "foo\\bar",
            "/tmp/app",
            "1app",
            "hello world",
            "gen",
            "Self",
            "CON",
            "lpt1",
            "rake",
            "std",
            "core",
            "alloc",
            "proc-macro",
            "proc_macro",
            "test",
            "a\n",
            "éclair",
        ] {
            assert!(validate_name(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn git_revision_requires_a_full_hash() {
        assert!(validate_revision("0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_revision("main").is_err());
        assert!(validate_revision("").is_err());
    }

    #[test]
    fn failed_install_removes_only_created_files_and_directories() {
        let parent = tempfile::tempdir().unwrap();

        let files = [
            ("src/lib.rs", "first".into()),
            ("src/lib.rs", "conflict".into()),
        ];

        let new = parent.path().join("new");

        assert!(install(&new, &files, None).is_err());
        assert!(!new.exists());

        let existing = parent.path().join("existing");

        fs::create_dir(&existing).unwrap();
        assert!(install(&existing, &files, None).is_err());
        assert_eq!(fs::read_dir(&existing).unwrap().count(), 0);

        fs::write(existing.join("keep"), "user data").unwrap();
        assert!(install(&existing, &files, None).is_err());
        assert_eq!(
            fs::read_to_string(existing.join("keep")).unwrap(),
            "user data"
        );
    }
}
