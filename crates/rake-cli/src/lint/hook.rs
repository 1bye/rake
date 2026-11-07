//! Advisory PostToolUse input/output; failures become context and never a CLI error.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::{Diagnostic, check_target};

const FINDING_LIMIT: usize = 20;

#[derive(Deserialize)]
struct Event {
    hook_event_name: String,
    tool_name: String,
    cwd: Option<PathBuf>,
    tool_input: Value,
}

pub(super) fn run(arguments: &[String]) {
    let context = if arguments.len() != 1 {
        Some(failure("received unsupported hook arguments"))
    } else {
        let mut input = String::new();
        match io::stdin().read_to_string(&mut input) {
            Ok(_) => match collect(&input) {
                Ok(context) => context,
                Err(reason) => Some(failure(reason)),
            },
            Err(_) => Some(failure("could not read the hook payload")),
        }
    };
    if let Some(context) = context {
        let output = serde_json::json!({"hookSpecificOutput": {
            "hookEventName": "PostToolUse", "additionalContext": context,
        }});
        // A closed hook consumer must not turn advisory feedback into a failing exit.
        let mut stdout = io::stdout().lock();
        let _ = serde_json::to_writer(&mut stdout, &output);
        let _ = stdout.write_all(b"\n");
    }
}

fn failure(reason: &str) -> String {
    format!("rake lint {reason}; run rake lint manually.")
}

fn paths(event: &Event) -> Result<Vec<PathBuf>, &'static str> {
    let invalid = "received an unsupported hook payload";
    if event.hook_event_name != "PostToolUse" {
        return Err(invalid);
    }
    match event.tool_name.as_str() {
        "apply_patch" => {
            let patch = event
                .tool_input
                .get("command")
                .and_then(Value::as_str)
                .ok_or(invalid)?;
            if patch.lines().next() != Some("*** Begin Patch")
                || patch.lines().last() != Some("*** End Patch")
            {
                return Err(invalid);
            }
            let mut paths = Vec::new();
            let mut deletion = false;
            for line in patch.lines() {
                if let Some(path) = ["*** Add File: ", "*** Update File: ", "*** Move to: "]
                    .iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
                {
                    if path.trim().is_empty() {
                        return Err(invalid);
                    }
                    paths.push(PathBuf::from(path));
                }
                deletion |= line.starts_with("*** Delete File: ");
            }
            if paths.is_empty() && !deletion {
                return Err(invalid);
            }
            Ok(paths)
        }
        _ => {
            let path = event
                .tool_input
                .get("file_path")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty());
            Ok(path.into_iter().map(PathBuf::from).collect())
        }
    }
}

fn collect(input: &str) -> Result<Option<String>, &'static str> {
    let event: Event =
        serde_json::from_str(input).map_err(|_| "received an unsupported hook payload")?;
    let candidates = paths(&event)?;
    if candidates.is_empty() {
        return Ok(None);
    }
    let cwd = match event.cwd {
        Some(path) if path.is_absolute() => path,
        path => std::env::current_dir()
            .map_err(|_| "could not determine the current directory")?
            .join(path.unwrap_or_default()),
    };
    let cwd = cwd
        .canonicalize()
        .map_err(|_| "could not resolve the hook directory")?;
    if !cwd.is_dir() {
        return Err("could not resolve the hook directory");
    }
    let mut selected = BTreeSet::new();
    let mut failed = false;
    for name in candidates {
        let path = cwd.join(name);
        if path.extension().is_none_or(|ext| ext != "rs")
            && path.file_name().is_none_or(|name| name != "Cargo.toml")
        {
            continue;
        }
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => match path.canonicalize() {
                Ok(path) => {
                    selected.insert(path);
                }
                Err(_) => failed = true,
            },
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => failed = true,
        }
    }
    let mut lines = Vec::new();
    let mut total = 0;
    for path in selected {
        match check_target(&path) {
            Ok((_, report)) => {
                total += report.diagnostics.len();
                for diagnostic in report.diagnostics {
                    if lines.len() < FINDING_LIMIT {
                        let display = path.strip_prefix(&cwd).unwrap_or(&path).to_string_lossy();
                        lines.push(finding(&display, &diagnostic));
                    }
                }
            }
            Err(_) => failed = true,
        }
    }
    if total == 0 && !failed {
        return Ok(None);
    }
    let mut context = if total > 0 {
        let noun = if total == 1 { "issue" } else { "issues" };
        format!(
            "rake lint found {total} {noun} in the edited files:\n{}",
            lines.join("\n")
        )
    } else {
        String::new()
    };
    if total > FINDING_LIMIT {
        context.push_str(&format!("\n{} more; run rake lint", total - FINDING_LIMIT));
    }
    if failed {
        if !context.is_empty() {
            context.push('\n');
        }
        context.push_str(&failure("could not check an edited file"));
    }
    Ok(Some(context))
}

fn finding(path: &str, diagnostic: &Diagnostic) -> String {
    let path = compact(path, 140);
    let message = compact(diagnostic.message.trim_end_matches('.'), 140);
    let fix = compact(diagnostic.fix, 180);
    format!(
        "{path}:{}:{} {} ({}): {message}. Fix: {fix}",
        diagnostic.span.start.line,
        diagnostic.span.start.column,
        diagnostic.rule_id,
        diagnostic.level.name()
    )
}

fn compact(text: &str, limit: usize) -> String {
    let mut chars = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch });
    let mut line: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        line.push_str("...");
    }
    line
}
