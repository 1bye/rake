//! Stable, source-only project linting; never invokes Cargo or expands Rust macros.

mod catalogue;
mod source;
mod suppression;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::Span;
use serde::{Deserialize, Serialize};

use catalogue::{Level, SYNTAX, rule};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Levels = BTreeMap<String, Level>;

#[derive(Debug, Serialize)]
struct Position {
    line: usize,
    column: usize,
}

#[derive(Debug, Serialize)]
struct SourceSpan {
    start: Position,
    end: Position,
}

#[derive(Debug, Serialize)]
struct Diagnostic {
    rule_id: &'static str,
    level: Level,
    file: String,
    span: SourceSpan,
    message: String,
    explanation: &'static str,
    fix: &'static str,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    diagnostics: Vec<Diagnostic>,
    files_checked: usize,
    denied: bool,
}

impl Diagnostic {
    fn new(id: &'static str, file: &Path, span: Span) -> Self {
        let rule = rule(id).expect("diagnostics use catalogue IDs");
        let position = |point: proc_macro2::LineColumn| Position {
            line: point.line.max(1),
            column: point.column + 1,
        };

        Self {
            rule_id: id,
            level: rule.level,
            file: file
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            span: SourceSpan {
                start: position(span.start()),
                end: position(span.end()),
            },
            message: rule.message.into(),
            explanation: rule.explanation,
            fix: rule.fix,
        }
    }
}

pub(crate) fn run(arguments: &[String]) -> Result<bool> {
    let mut json = false;
    let mut explain = None;
    let mut path = None;
    let mut args = arguments.iter();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--explain" => explain = Some(args.next().ok_or("--explain requires a rule ID")?),
            "--help" | "-h" => {
                println!(
                    "Usage: rake lint [PATH] [--json]\n       rake lint --explain <ID> [--json]"
                );

                return Ok(false);
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown lint option: {value}").into());
            }
            _ if path.is_none() => path = Some(PathBuf::from(argument)),
            _ => return Err("lint accepts only one project path".into()),
        }
    }

    if let Some(id) = explain {
        if path.is_some() {
            return Err("--explain cannot be combined with a project path".into());
        }

        let rule = rule(id).ok_or_else(|| format!("unknown lint rule: {id}"))?;

        if json {
            println!(
                "{}",
                serde_json::json!({
                    "schema_version": 1, "rule": {
                        "id":rule.id, "default_level":rule.level,
                        "message":rule.message, "explanation":rule.explanation,
                        "fix":rule.fix, "before":rule.before, "after":rule.after
                    }
                })
            );
        } else {
            println!(
                "{} ({})\n\n{}\n\n{}\n\nFix: {}\n\nBefore:\n{}\n\nAfter:\n{}",
                rule.id,
                rule.level.name(),
                rule.message,
                rule.explanation,
                rule.fix,
                rule.before,
                rule.after
            );
        }

        return Ok(false);
    }

    let root = path.unwrap_or(std::env::current_dir()?);
    let report = check_project(&root)?;

    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        for diagnostic in &report.diagnostics {
            let source = fs::read_to_string(root.join(&diagnostic.file))?;

            eprint!("{}", render(diagnostic, &source));
        }

        println!(
            "Checked {} Rust files; {} diagnostics; {} denied.",
            report.files_checked,
            report.diagnostics.len(),
            report
                .diagnostics
                .iter()
                .filter(|d| d.level == Level::Deny)
                .count()
        );
    }

    Ok(report.denied)
}

fn render(diagnostic: &Diagnostic, source: &str) -> String {
    let start = &diagnostic.span.start;
    let end = &diagnostic.span.end;
    let line = source.lines().nth(start.line - 1).unwrap_or("");
    let width = if end.line == start.line {
        end.column.saturating_sub(start.column).max(1)
    } else {
        line.chars().count().saturating_sub(start.column - 1).max(1)
    };
    let severity = if diagnostic.level == Level::Deny {
        "error"
    } else {
        "warning"
    };

    let line_number = start.line;
    let gutter = line_number.to_string().len();
    let arrow = " ".repeat(gutter);
    let padding = " ".repeat(gutter + 1);

    format!(
        "{severity}[{}]: {}\n{arrow}--> {}:{}:{}\n{padding}|\n{line_number:>gutter$} | {line}\n{padding}| {}{}\n{padding}= note: {}\n{padding}= help: {}\n\n",
        diagnostic.rule_id,
        diagnostic.message,
        diagnostic.file,
        start.line,
        start.column,
        " ".repeat(start.column - 1),
        "^".repeat(width),
        diagnostic.explanation,
        diagnostic.fix
    )
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    levels: Levels,
    exclude: Vec<PathBuf>,
}

fn load_config(directory: &Path) -> Result<Config> {
    let manifest = directory.join("Cargo.toml");
    let value: toml::Value = toml::from_str(&fs::read_to_string(&manifest)?)?;
    let config = value
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("rake"))
        .and_then(|v| v.get("lint"))
        .cloned();
    let mut config: Config = config
        .map(|value| value.try_into())
        .transpose()?
        .unwrap_or_default();

    for id in config.levels.keys() {
        if rule(id).is_none() {
            return Err(format!("{}: unknown lint rule {id}", manifest.display()).into());
        }

        if id == SYNTAX {
            return Err("syntax-error is always deny and cannot be configured".into());
        }
    }

    for excluded in &mut config.exclude {
        if excluded.as_os_str().is_empty()
            || excluded.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "{}: lint exclude paths must be nonempty and package-relative without '..': {}",
                manifest.display(),
                excluded.display()
            )
            .into());
        }

        *excluded = directory.join(&*excluded);
    }

    Ok(config)
}

fn check_project(root: &Path) -> Result<Report> {
    if !root.join("Cargo.toml").is_file() {
        return Err(format!(
            "{}: expected a project directory containing Cargo.toml",
            root.display()
        )
        .into());
    }

    let mut report = Report {
        schema_version: 1,
        diagnostics: Vec::new(),
        files_checked: 0,
        denied: false,
    };

    scan_directory(root, root, &Config::default(), &mut report)?;
    report.diagnostics.sort_by(|left, right| {
        (
            &left.file,
            left.span.start.line,
            left.span.start.column,
            left.rule_id,
        )
            .cmp(&(
                &right.file,
                right.span.start.line,
                right.span.start.column,
                right.rule_id,
            ))
    });
    report.denied = report.diagnostics.iter().any(|d| d.level == Level::Deny);

    Ok(report)
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    inherited: &Config,
    report: &mut Report,
) -> Result<()> {
    let mut local;
    let config = if directory.join("Cargo.toml").is_file() {
        local = load_config(directory)?;
        local.exclude.extend_from_slice(&inherited.exclude);

        &local
    } else {
        inherited
    };
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;

    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let kind = entry.file_type()?;
        let path = entry.path();

        if config
            .exclude
            .iter()
            .any(|excluded| path.starts_with(excluded))
        {
            continue;
        }

        if kind.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some("target" | ".git" | "node_modules")
            ) {
                scan_directory(root, &path, config, report)?;
            }
        } else if kind.is_file()
            && path.extension().is_some_and(|extension| extension == "rs")
            && !path.with_extension("stderr").is_file()
        {
            let source = fs::read_to_string(&path)?;
            let relative = path.strip_prefix(root)?;

            report
                .diagnostics
                .extend(check_source(relative, &source, &config.levels));
            report.files_checked += 1;
        }
    }

    Ok(())
}

fn check_source(file: &Path, text: &str, levels: &Levels) -> Vec<Diagnostic> {
    let mut diagnostics = source::check(file, text);
    let suppressions = suppression::parse(file, text, &mut diagnostics);

    diagnostics.retain_mut(|diagnostic| {
        if let Some(level) = levels.get(diagnostic.rule_id) {
            diagnostic.level = *level;
        }

        diagnostic.level != Level::Allow
            && !suppressions.iter().any(|suppression| {
                catalogue::suppressible(diagnostic.rule_id)
                    && suppression.id == diagnostic.rule_id
                    && suppression.line == diagnostic.span.start.line
            })
    });

    diagnostics
}
