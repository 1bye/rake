//! Debian packages use dpkg's native version and dependency grammar.
use crate::{ArchPackage, Error, run};
use std::{fs, path::PathBuf, process::Command};

/// Builds a Debian archive from a staged root using dpkg-deb.
/// # Errors
/// Returns invalid control fields, an existing DEBIAN directory or native tool errors.
pub fn build_deb(input: &ArchPackage<'_>) -> Result<PathBuf, Error> {
    let architecture = match input.target.split('-').next() {
        Some("x86_64") => "amd64",
        Some("aarch64") => "arm64",
        _ => return Err(Error::Metadata("unsupported Debian architecture".into())),
    };

    for field in [input.name, input.version, input.display_name]
        .into_iter()
        .chain(input.dependencies.iter().map(String::as_str))
    {
        if field.chars().any(char::is_control) {
            return Err(Error::Metadata("control character in Debian field".into()));
        }
    }

    run(Command::new("dpkg").args(["--validate-version", input.version]))?;
    let control = input.payload.join("DEBIAN");

    fs::create_dir(&control)?;
    let dependencies = if input.dependencies.is_empty() {
        String::new()
    } else {
        format!("Depends: {}\n", input.dependencies.join(", "))
    };

    fs::write(
        control.join("control"),
        format!(
            "Package: {}\nVersion: {}\nArchitecture: {architecture}\nMaintainer: Rake <noreply@rake.invalid>\nSection: utils\nPriority: optional\n{dependencies}Description: {}\n",
            input.name, input.version, input.display_name
        ),
    )?;
    fs::create_dir_all(input.output)?;
    let output = input.output.join(format!(
        "{}_{}_{architecture}.deb",
        input.name, input.version
    ));

    run(Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(input.payload)
        .arg(&output))?;

    Ok(output)
}
