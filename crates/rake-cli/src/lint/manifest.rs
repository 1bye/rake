//! Explicit development controls in release profiles, without guessing feature selection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use super::catalogue::RELEASE;
use super::{Diagnostic, Result};

#[derive(Default, Deserialize)]
#[serde(default)]
struct Manifest {
    profile: BTreeMap<String, Profile>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct Profile {
    inherits: Option<String>,
    debug_assertions: Option<toml::Spanned<bool>>,
    package: BTreeMap<String, Profile>,
    build_override: Option<Box<Profile>>,
}

pub(super) fn check(file: &Path, text: &str) -> Result<Vec<Diagnostic>> {
    let manifest: Manifest = toml::from_str(text)?;
    let mut diagnostics = Vec::new();
    for (name, profile) in &manifest.profile {
        let mut current = name.as_str();
        let mut seen = BTreeSet::new();
        while current != "release" && seen.insert(current) {
            let Some(parent) = manifest
                .profile
                .get(current)
                .and_then(|p| p.inherits.as_deref())
            else {
                break;
            };
            current = parent;
        }
        if current == "release" {
            controls(file, text, profile, &mut diagnostics);
        }
    }
    Ok(diagnostics)
}

fn controls(file: &Path, text: &str, profile: &Profile, diagnostics: &mut Vec<Diagnostic>) {
    if let Some(value) = &profile.debug_assertions
        && *value.get_ref()
    {
        let mut diagnostic = Diagnostic::at_bytes(RELEASE, file, text, value.span());
        diagnostic.set_edit(text, value.span(), "false".into());
        diagnostics.push(diagnostic);
    }
    for package in profile.package.values() {
        controls(file, text, package, diagnostics);
    }
    if let Some(build) = &profile.build_override {
        controls(file, text, build, diagnostics);
    }
}
