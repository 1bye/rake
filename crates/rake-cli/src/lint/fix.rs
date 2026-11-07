//! Validate all edits before writing; format only the Rust files with safe fixes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{Diagnostic, Report, Result};

pub(super) fn apply(root: &Path, report: &Report) -> Result<()> {
    let mut files: BTreeMap<&str, Vec<&Diagnostic>> = BTreeMap::new();

    for diagnostic in &report.diagnostics {
        if diagnostic.edit.is_some() {
            files.entry(&diagnostic.file).or_default().push(diagnostic);
        }
    }

    let mut outputs = Vec::new();

    for (file, mut diagnostics) in files {
        if report
            .diagnostics
            .iter()
            .any(|d| d.file == file && d.rule_id == super::catalogue::SYNTAX)
        {
            continue;
        }

        let path = root.join(file);
        let original = fs::read_to_string(&path)?;

        if report.original_sources.get(file) != Some(&original) {
            return Err(format!("{file}: stale lint fixes; file left unchanged").into());
        }

        diagnostics.sort_by_key(|d| d.edit.as_ref().expect("selected edit").range.start);
        let mut output = original.clone();
        let mut end = 0;

        for diagnostic in &diagnostics {
            let edit = diagnostic.edit.as_ref().expect("selected edit");

            if edit.range.start < end || original.get(edit.range.clone()) != Some(&edit.original) {
                return Err(format!(
                    "{file}: overlapping or stale lint fixes; file left unchanged"
                )
                .into());
            }

            end = edit.range.end;
        }

        for diagnostic in diagnostics.into_iter().rev() {
            let edit = diagnostic.edit.as_ref().expect("selected edit");

            output.replace_range(edit.range.clone(), &edit.replacement);
        }

        if path.extension().is_some_and(|extension| extension == "rs") {
            output = crate::format::fixed_source(&path, &output)?;
        } else {
            let _: toml::Value = toml::from_str(&output)?;
        }

        outputs.push((path, original, output));
    }

    // A formatter/configuration failure above must never leave a partially edited project.
    for (path, original, _) in &outputs {
        if fs::read_to_string(path)? != *original {
            return Err(format!("{}: source changed during lint --fix", path.display()).into());
        }
    }

    for (path, _, output) in outputs {
        fs::write(path, output)?;
    }

    Ok(())
}
