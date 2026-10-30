//! Opens captured Rust construction sites with the system file association.

use rake_protocol::inspection::SourceLocation;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn resolve(root: &Path, source: &SourceLocation) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let path = root.join(&source.file).canonicalize()?;

    if !path.starts_with(&root)
        || !path.is_file()
        || path.extension().is_none_or(|extension| extension != "rs")
        || source.line == 0
        || source.column == 0
    {
        return Err("Source must be an existing Rust file inside RAKE_SOURCE_ROOT (default: current directory)".into());
    }

    Ok(path)
}

pub(crate) fn relative(source: &SourceLocation) -> Result<String> {
    let root = std::env::var_os("RAKE_SOURCE_ROOT")
        .map(PathBuf::from)
        .map_or_else(std::env::current_dir, Ok)?
        .canonicalize()?;
    let path = resolve(&root, source)?;
    Ok(path
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/"))
}

pub(crate) fn read(source: &SourceLocation) -> Result<String> {
    use std::io::Read;
    let root = std::env::var_os("RAKE_SOURCE_ROOT")
        .map(PathBuf::from)
        .map_or_else(std::env::current_dir, Ok)?;
    let path = resolve(&root, source)?;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(256 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 256 * 1024 {
        return Err("Rust source exceeds 256 KiB".into());
    }
    Ok(text)
}

pub(crate) fn open(source: &SourceLocation) -> Result<serde_json::Value> {
    let root = std::env::var_os("RAKE_SOURCE_ROOT")
        .map(PathBuf::from)
        .map_or_else(std::env::current_dir, Ok)?;
    let path = resolve(&root, source)?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(opener)
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err(format!("System editor opener exited with {status}").into());
    }

    Ok(serde_json::json!({"path":path,"line":source.line,"column":source.column}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_resolution_rejects_escape_symlinks_and_non_rust_files() {
        let directory = std::env::temp_dir().join(format!("rake-source-{}", uuid::Uuid::new_v4()));
        let root = directory.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("app.rs"), "fn main() {}\n").unwrap();
        std::fs::write(directory.join("outside.rs"), "").unwrap();
        std::fs::write(root.join("notes.txt"), "").unwrap();
        std::os::unix::fs::symlink(directory.join("outside.rs"), root.join("escape.rs")).unwrap();
        let source = |file: &str| SourceLocation {
            file: file.into(),
            line: 3,
            column: 4,
        };

        assert_eq!(
            resolve(&root, &source("app.rs")).unwrap(),
            root.join("app.rs").canonicalize().unwrap()
        );
        for file in ["../outside.rs", "escape.rs", "notes.txt", "missing.rs"] {
            assert!(resolve(&root, &source(file)).is_err(), "{file}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
