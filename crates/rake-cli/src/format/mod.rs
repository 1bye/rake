//! Project and editor formatting: Rust first, then source-preserving view markup.

mod markup;
mod rustfmt;
mod source;

#[cfg(test)]
mod tests;

use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use rustfmt::Rustfmt;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Problem {
    line: usize,
    column: usize,
    message: String,
}

impl Problem {
    fn parse(error: syn::Error) -> Self {
        let point = error.span().start();
        Self {
            line: point.line.max(1),
            column: point.column + 1,
            message: error.to_string(),
        }
    }
}

pub(crate) fn run(arguments: &[String]) -> Result<bool> {
    let mut check = false;
    let mut stdin = false;
    let mut paths = Vec::new();
    let mut edition = None;
    let mut arguments = arguments.iter();
    let mut positional = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--" if !positional => positional = true,
            "--check" if !positional => check = true,
            "--stdin" if !positional => stdin = true,
            "--edition" if !positional => {
                edition = Some(
                    arguments
                        .next()
                        .ok_or("--edition requires a value")?
                        .clone(),
                )
            }
            "--emit" if !positional => {
                if arguments.next().map(String::as_str) != Some("stdout") {
                    return Err("only --emit stdout is supported".into());
                }
            }
            "--emit=stdout" if !positional => {}
            "--help" | "-h" if !positional => {
                println!(
                    "Usage: rake format [FILES...] [--check]\n       rake format --stdin [--edition EDITION]\nAlias: rake fmt\n\nFormat Rust first, then view! markup. --check prints diffs without writing.\n--stdin reads one Rust file and writes only the formatted file to stdout."
                );
                return Ok(false);
            }
            option if !positional && option.starts_with('-') => {
                return Err(format!("unknown format option: {option}").into());
            }
            _ => paths.push(PathBuf::from(argument)),
        }
    }
    if stdin && (check || !paths.is_empty()) {
        return Err("--stdin cannot be combined with --check or file arguments".into());
    }
    let cwd = std::env::current_dir()?;
    if stdin {
        let mut source = String::new();
        std::io::stdin().read_to_string(&mut source)?;
        let mut rustfmt = Rustfmt::load(&cwd)?;
        if let Some(edition) = edition {
            rustfmt.edition = edition;
        }
        let (output, problems) = format_file(&source, &mut rustfmt)?;
        std::io::stdout().write_all(output.as_bytes())?;
        report(Path::new("<stdin>"), &problems);
        return Ok(!problems.is_empty());
    }
    if edition.is_some() {
        return Err(
            "--edition is supported only with --stdin; files use their Cargo edition".into(),
        );
    }
    let project = paths.is_empty();
    if project && !cwd.join("Cargo.toml").is_file() {
        return Err(
            "rake format requires Cargo.toml in the current directory, or explicit Rust files"
                .into(),
        );
    }
    // Resolve and validate all requested files before invoking any formatter.
    let paths = if project {
        crate::lint::files(&cwd)?
    } else {
        let mut resolved = Vec::new();
        for path in paths {
            let path = path.canonicalize()?;
            if !path.is_file() || path.extension().is_none_or(|extension| extension != "rs") {
                return Err(format!("{}: expected a Rust file", path.display()).into());
            }
            resolved.push(path);
        }
        resolved.sort();
        resolved.dedup();
        resolved
    };
    let inputs = paths
        .iter()
        .map(fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut failed = false;
    if project {
        let mut cargo = Command::new("cargo");
        cargo.args(["fmt", "--all"]);
        if check {
            cargo.arg("--check");
        }
        // Diagnostics stay off stdout, including when cargo fmt fails.
        let output = cargo.output()?;
        std::io::stderr().write_all(&output.stderr)?;
        std::io::stdout().write_all(&output.stdout)?;
        if !output.status.success() {
            if !check {
                return Err("cargo fmt failed".into());
            }
            failed = true;
        }
    }
    for (path, before_cargo) in paths.into_iter().zip(inputs) {
        let original = fs::read_to_string(&path)?;
        let input = source::preserve_views(&before_cargo, &original);
        let mut rustfmt = Rustfmt::load(path.parent().ok_or("file has no parent directory")?)?;
        let has_views = crate::view_source::invocations(&input)
            .is_ok_and(|invocations| !invocations.is_empty());
        let (formatted, problems) = if !project || has_views {
            format_file(&input, &mut rustfmt)?
        } else {
            markup::format_source(&input, &mut rustfmt)?
        };
        report(&path, &problems);
        failed |= !problems.is_empty();
        if formatted != original {
            if check {
                diff(&path, &original, &formatted)?;
                failed = true;
            } else {
                fs::write(&path, formatted)?;
            }
        }
    }
    Ok(failed)
}

fn report(path: &Path, problems: &[Problem]) {
    for problem in problems {
        eprintln!(
            "{}:{}:{}: {} (view! left unchanged)",
            path.display(),
            problem.line,
            problem.column,
            problem.message
        );
    }
}

fn diff(path: &Path, original: &str, formatted: &str) -> Result<()> {
    let mut before = tempfile::NamedTempFile::new()?;
    let mut after = tempfile::NamedTempFile::new()?;
    before.write_all(original.as_bytes())?;
    after.write_all(formatted.as_bytes())?;
    let output = Command::new("diff")
        .args(["-u", "-L"])
        .arg(path)
        .arg("-L")
        .arg(format!("{} (formatted)", path.display()))
        .arg(before.path())
        .arg(after.path())
        .output()?;
    if output.status.code() != Some(1) {
        return Err(format!("diff failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    std::io::stdout().write_all(&output.stdout)?;
    Ok(())
}

fn format_file(source: &str, rustfmt: &mut Rustfmt) -> Result<(String, Vec<Problem>)> {
    let mut current = source.to_owned();
    for _ in 0..8 {
        let rust = rustfmt.run(&current, rustfmt.width)?;
        let (formatted, problems) = markup::format_source(&rust, rustfmt)?;
        if formatted == current {
            return Ok((formatted, problems));
        }
        current = formatted;
    }
    Err("Rust and markup formatting did not converge; file left unchanged".into())
}

/// Format a lint-edited source with the same convergence and comment guarantees.
pub(crate) fn fixed_source(path: &Path, source: &str) -> Result<String> {
    let mut rustfmt = Rustfmt::load(path.parent().ok_or("file has no parent directory")?)?;
    let (output, problems) = format_file(source, &mut rustfmt)?;
    if !problems.is_empty() {
        return Err(format!("{}: cannot format lint fix safely", path.display()).into());
    }
    Ok(output)
}
