//! Source linting with an opt-in, type-aware Dylint tier.

mod catalogue;
mod deep;
mod design;
mod fix;
mod guide;
mod hook;
mod manifest;
mod source;
mod suppression;

pub(crate) use guide::generate as agent_skill;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    machine_fix: Option<MachineFix>,
    #[serde(skip)]
    edit: Option<Edit>,
}

#[derive(Debug, Serialize)]
struct MachineFix {
    applicability: &'static str,
    span: SourceSpan,
    replacement: String,
}

#[derive(Debug)]
struct Edit {
    range: std::ops::Range<usize>,
    original: String,
    replacement: String,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    diagnostics: Vec<Diagnostic>,
    files_checked: usize,
    denied: bool,
    #[serde(skip)]
    original_sources: BTreeMap<String, String>,
}

impl Diagnostic {
    fn at_bytes(id: &'static str, file: &Path, text: &str, range: std::ops::Range<usize>) -> Self {
        let mut diagnostic = Self::new(id, file, Span::call_site());

        diagnostic.span = byte_span(text, range);

        diagnostic
    }

    fn set_edit(&mut self, text: &str, range: std::ops::Range<usize>, replacement: String) {
        self.machine_fix = Some(MachineFix {
            applicability: "machine-applicable",
            span: byte_span(text, range.clone()),
            replacement: replacement.clone(),
        });
        self.edit = Some(Edit {
            original: text[range.clone()].into(),
            range,
            replacement,
        });
    }

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
            machine_fix: None,
            edit: None,
        }
    }
}

fn byte_span(text: &str, range: std::ops::Range<usize>) -> SourceSpan {
    let position = |offset| {
        let prefix = &text[..offset];

        Position {
            line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
            column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        }
    };

    SourceSpan {
        start: position(range.start),
        end: position(range.end),
    }
}

pub(crate) fn run(arguments: &[String]) -> Result<bool> {
    if arguments.iter().any(|argument| argument == "--hook") {
        hook::run(arguments);

        return Ok(false);
    }

    let mut json = false;
    let mut guide = None;
    let mut update = None;
    let mut fix = false;
    let mut deep = false;
    let mut explain = None;
    let mut path = None;
    let mut args = arguments.iter();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--guide" => guide = Some(false),
            "--guide=agents" => guide = Some(true),
            "--update" => {
                update = Some(PathBuf::from(
                    args.next()
                        .filter(|value| !value.starts_with('-'))
                        .ok_or("--update requires a file path")?,
                ))
            }
            "--fix" => fix = true,
            "--deep" => deep = true,
            "--explain" => explain = Some(args.next().ok_or("--explain requires a rule ID")?),
            "--help" | "-h" => {
                println!(
                    "Usage: rake lint [PATH] [--deep] [--fix] [--json]\n       rake lint --explain <ID> [--json]\n       rake lint --guide[=agents] [--json]\n       rake lint --guide=agents --update AGENTS.md\n       rake lint --hook"
                );

                return Ok(false);
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown lint option: {value}").into());
            }
            _ if path.is_none() => path = Some(PathBuf::from(argument)),
            _ => return Err("lint accepts only one path".into()),
        }
    }

    if let Some(agents) = guide {
        if path.is_some() || explain.is_some() || fix || deep {
            return Err(
                "--guide cannot be combined with a project path, --explain or --fix".into(),
            );
        }

        if update.is_some() && !agents {
            return Err("--update requires --guide=agents".into());
        }

        if let Some(path) = update {
            let original = fs::read_to_string(&path)?;
            let refreshed = guide::refresh_agents(&original)?;
            let changed = refreshed != original;

            if changed {
                fs::write(&path, refreshed)?;
            }

            if json {
                println!(
                    "{}",
                    serde_json::json!({"schema_version": 1, "path": path, "updated": changed})
                );
            } else if changed {
                println!("updated rake-lint block in {}", path.display());
            } else {
                println!("{} already up to date", path.display());
            }

            return Ok(false);
        }

        let guide = if agents {
            guide::agents()
        } else {
            guide::generate()
        };

        if json {
            println!(
                "{}",
                serde_json::json!({"schema_version": 1, "guide": guide})
            );
        } else {
            print!("{guide}");
        }

        return Ok(false);
    }

    if update.is_some() {
        return Err("--update requires --guide=agents".into());
    }

    if let Some(id) = explain {
        if path.is_some() || fix || deep {
            return Err("--explain cannot be combined with a project path or --fix".into());
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

    let path = path.unwrap_or(std::env::current_dir()?);

    let check = || {
        if deep {
            let root = path.canonicalize()?;

            if !root.is_dir() || !root.join("Cargo.toml").is_file() {
                return Err("--deep expects a project directory containing Cargo.toml".into());
            }

            let mut findings = deep::collect(&root)?;
            let report = check_project_with_deep(&root, Some(&mut findings))?;

            Ok((root, report))
        } else {
            check_target(&path)
        }
    };

    let (root, mut report) = check()?;

    if fix {
        fix::apply(&root, &report)?;
        report = check()?.1;
    }

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

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    levels: Levels,
    exclude: Vec<PathBuf>,
    #[serde(skip)]
    package: PathBuf,
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

    config.package = directory.to_owned();

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
    check_project_with_deep(root, None)
}

fn check_project_with_deep(root: &Path, mut deep: Option<&mut deep::Findings>) -> Result<Report> {
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
        original_sources: BTreeMap::new(),
    };

    let mut sources = Vec::new();
    let mut manifests = BTreeMap::new();

    scan_directory(root, &Config::default(), None, &mut |path, config| {
        manifests.insert(config.package.clone(), config.clone());

        if path.extension().is_some_and(|ext| ext == "rs") {
            sources.push((path.to_owned(), fs::read_to_string(path)?, config.clone()));
        }

        Ok(())
    })?;
    let mut themed = std::collections::BTreeSet::new();

    for (_, text, config) in &sources {
        if source::installs_theme(text) {
            themed.insert(config.package.clone());
        }
    }

    for (path, text, config) in sources {
        let relative = path.strip_prefix(root)?;

        let mut diagnostics =
            source::check_themed(relative, &text, themed.contains(&config.package));

        if let Some(findings) = deep.as_mut() {
            diagnostics.extend(findings.remove(&path).unwrap_or_default());
        }

        let diagnostics = filter_diagnostics_with_deep(
            relative,
            &text,
            &config.levels,
            diagnostics,
            false,
            deep.is_some(),
        );

        if let Some(diagnostic) = diagnostics.iter().find(|d| d.edit.is_some()) {
            report
                .original_sources
                .insert(diagnostic.file.clone(), text);
        }

        report.diagnostics.extend(diagnostics);
        report.files_checked += 1;
    }

    // Empty packages and virtual workspace release profiles must also be checked.
    for (directory, config) in manifests {
        let path = directory.join("Cargo.toml");
        let text = fs::read_to_string(&path)?;
        let relative = path.strip_prefix(root)?;
        let diagnostics = manifest::check(relative, &text)?;

        let diagnostics = filter_diagnostics_with_deep(
            relative,
            &text,
            &config.levels,
            diagnostics,
            true,
            deep.is_some(),
        );

        if let Some(diagnostic) = diagnostics.iter().find(|d| d.edit.is_some()) {
            report
                .original_sources
                .insert(diagnostic.file.clone(), text);
        }

        report.diagnostics.extend(diagnostics);
    }

    finish_report(&mut report);

    Ok(report)
}

fn scan_directory(
    directory: &Path,
    inherited: &Config,
    boundary: Option<&Path>,
    visit: &mut impl FnMut(&Path, &Config) -> Result<()>,
) -> Result<()> {
    if boundary.is_some_and(|root| root != directory) && directory.join("Cargo.toml").is_file() {
        return Ok(());
    }

    let mut local;

    let config = if directory.join("Cargo.toml").is_file() {
        local = load_config(directory)?;
        local.exclude.extend_from_slice(&inherited.exclude);

        &local
    } else {
        inherited
    };

    if directory.join("Cargo.toml").is_file() {
        visit(&directory.join("Cargo.toml"), config)?;
    }

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
                scan_directory(&path, config, boundary, visit)?;
            }
        } else if kind.is_file()
            && path.extension().is_some_and(|extension| extension == "rs")
            && !path.with_extension("stderr").is_file()
        {
            visit(&path, config)?;
        }
    }

    Ok(())
}

#[cfg(test)]
fn check_source(file: &Path, text: &str, levels: &Levels) -> Vec<Diagnostic> {
    filter_diagnostics(file, text, levels, source::check(file, text), false)
}

fn filter_diagnostics(
    file: &Path,
    text: &str,
    levels: &Levels,
    diagnostics: Vec<Diagnostic>,
    manifest: bool,
) -> Vec<Diagnostic> {
    filter_diagnostics_with_deep(file, text, levels, diagnostics, manifest, false)
}

fn filter_diagnostics_with_deep(
    file: &Path,
    text: &str,
    levels: &Levels,
    mut diagnostics: Vec<Diagnostic>,
    manifest: bool,
    deep: bool,
) -> Vec<Diagnostic> {
    let mut suppressions = if manifest {
        suppression::parse_manifest(file, text, &mut diagnostics)
    } else {
        suppression::parse(file, text, &mut diagnostics)
    };

    let enabled = |diagnostic: &mut Diagnostic| {
        if let Some(level) = levels.get(diagnostic.rule_id) {
            diagnostic.level = *level;
        }

        diagnostic.level != Level::Allow
    };

    diagnostics.retain_mut(|diagnostic| {
        enabled(diagnostic) && !suppressions.iter_mut().any(|s| s.suppresses(diagnostic))
    });
    diagnostics.extend(suppressions.into_iter().filter_map(|suppression| {
        let mut diagnostic = suppression.unused(deep)?;

        enabled(&mut diagnostic).then_some(diagnostic)
    }));

    diagnostics
}

/// Reuse lint discovery, package excludes, and compile-fail fixture handling.
pub(crate) fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    scan_directory(root, &Config::default(), None, &mut |path, _| {
        if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path.to_owned());
        }

        Ok(())
    })?;

    Ok(files)
}

fn check_target(path: &Path) -> Result<(PathBuf, Report)> {
    let path = path.canonicalize()?;

    if path.is_dir() {
        return Ok((path.clone(), check_project(&path)?));
    }

    if !path.is_file() {
        return Err("lint expects a regular file or project directory".into());
    }

    let rust = path.extension().is_some_and(|extension| extension == "rs");

    if !rust && path.file_name().is_none_or(|name| name != "Cargo.toml") {
        return Err("lint expects a project directory, Rust file or Cargo.toml".into());
    }

    let root = path
        .parent()
        .and_then(|parent| {
            parent
                .ancestors()
                .find(|directory| directory.join("Cargo.toml").is_file())
        })
        .ok_or("lint file must belong to a directory containing Cargo.toml")?;

    let config = load_config(root)?;
    let relative = path.strip_prefix(root)?;

    let mut report = Report {
        schema_version: 1,
        diagnostics: Vec::new(),
        files_checked: 0,
        denied: false,
        original_sources: BTreeMap::new(),
    };

    let ignored = relative.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some("target" | ".git" | "node_modules")
        )
    }) || config
        .exclude
        .iter()
        .any(|excluded| path.starts_with(excluded))
        || (rust && path.with_extension("stderr").is_file());

    if !ignored {
        let text = fs::read_to_string(&path)?;

        let diagnostics = if rust {
            let mut themed = source::installs_theme(&text);

            // Search only this package for theme installation, stopping reads as
            // soon as evidence is found; nested packages cannot supply evidence.
            if !themed {
                scan_directory(root, &Config::default(), Some(root), &mut |candidate, _| {
                    if !themed
                        && candidate != path
                        && candidate.extension().is_some_and(|ext| ext == "rs")
                    {
                        themed = source::installs_theme(&fs::read_to_string(candidate)?);
                    }

                    Ok(())
                })?;
            }

            report.files_checked = 1;

            source::check_themed(relative, &text, themed)
        } else {
            manifest::check(relative, &text)?
        };

        let diagnostics = filter_diagnostics(relative, &text, &config.levels, diagnostics, !rust);

        if let Some(diagnostic) = diagnostics.iter().find(|d| d.edit.is_some()) {
            report
                .original_sources
                .insert(diagnostic.file.clone(), text);
        }

        report.diagnostics = diagnostics;
    }

    finish_report(&mut report);

    Ok((root.to_owned(), report))
}

fn finish_report(report: &mut Report) {
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
}
