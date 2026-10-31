//! RPM dependency expressions retain comparator semantics at the package boundary.
use crate::{ArchPackage, Error};

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Builds an RPM from a relocated payload, preserving symlinks and versioned requirements.
/// # Errors
/// Returns invalid dependency expressions or package writer/filesystem failures.
pub fn build_rpm(input: &ArchPackage<'_>) -> Result<PathBuf, Error> {
    let architecture = match input.target.split('-').next() {
        Some("x86_64") => "x86_64",
        Some("aarch64") => "aarch64",
        _ => return Err(Error::Metadata("unsupported RPM architecture".into())),
    };

    let mut builder = rpm::PackageBuilder::new(
        input.name,
        input.version,
        "",
        architecture,
        input.display_name,
    );

    builder.release("1");

    for expression in input.dependencies {
        builder.requires(dependency(expression)?);
    }

    add(&mut builder, input.payload, input.payload)?;
    fs::create_dir_all(input.output)?;
    let output = input.output.join(format!(
        "{}-{}-1.{architecture}.rpm",
        input.name, input.version
    ));

    builder
        .build()
        .and_then(|package| package.write_file(&output))
        .map_err(|error| Error::Metadata(error.to_string()))?;

    Ok(output)
}

fn dependency(text: &str) -> Result<rpm::Dependency, Error> {
    let fields = text.split_whitespace().collect::<Vec<_>>();

    Ok(match fields.as_slice() {
        [name] => rpm::Dependency::any(*name),
        [name, operator, version] => match *operator {
            ">=" => rpm::Dependency::greater_eq(*name, *version),
            ">" => rpm::Dependency::greater(*name, *version),
            "=" => rpm::Dependency::eq(*name, *version),
            "<" => rpm::Dependency::less(*name, *version),
            "<=" => rpm::Dependency::less_eq(*name, *version),
            _ => {
                return Err(Error::Metadata(format!(
                    "invalid RPM dependency comparator: {text}"
                )));
            }
        },
        _ => {
            return Err(Error::Metadata(format!(
                "RPM dependency must be a capability or 'name >= version': {text}"
            )));
        }
    })
}

fn add(builder: &mut rpm::PackageBuilder, root: &Path, directory: &Path) -> Result<(), Error> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;

    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;

        let destination = format!(
            "/{}",
            path.strip_prefix(root)
                .map_err(|_| Error::Metadata("RPM payload path".into()))?
                .to_string_lossy()
        );

        if kind.is_dir() {
            add(builder, root, &path)?;
        } else if kind.is_symlink() {
            let target = fs::read_link(&path)?;

            if !fs::canonicalize(&path)?.starts_with(fs::canonicalize(root)?) {
                return Err(Error::Metadata("RPM symlink escapes payload".into()));
            }

            builder
                .with_symlink(rpm::FileOptions::symlink(
                    destination,
                    target.to_string_lossy(),
                ))
                .map_err(|error| Error::Metadata(error.to_string()))?;
        } else if kind.is_file() {
            builder
                .with_file(path, rpm::FileOptions::new(destination))
                .map_err(|error| Error::Metadata(error.to_string()))?;
        } else {
            return Err(Error::Metadata("special file in RPM payload".into()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_requires_retain_versions_and_reject_ambiguous_syntax() {
        let value = dependency("gstreamer1 >= 1.22").unwrap();

        assert_eq!(value.name, "gstreamer1");
        assert_eq!(value.version, "1.22");
        assert!(dependency("gstreamer1 >= 1.22 extra").is_err());
        assert!(dependency("gstreamer1 ~~ 1.22").is_err());
    }

    #[test]
    fn package_round_trip_exposes_versioned_requirements() {
        let root = tempfile::tempdir().unwrap();
        let payload = root.path().join("payload");

        fs::create_dir_all(payload.join("usr/share/app")).unwrap();
        fs::write(payload.join("usr/share/app/data"), "hello").unwrap();
        let path = build_rpm(&ArchPackage {
            name: "rake-test",
            version: "1.2.3",
            target: "x86_64-unknown-linux-gnu",
            display_name: "Rake Test",
            dependencies: &["gstreamer1 >= 1.22".into()],
            payload: &payload,
            output: &root.path().join("result"),
        })
        .unwrap();

        let package = rpm::Package::open(path).unwrap();

        assert!(
            package
                .metadata
                .get_requires()
                .unwrap()
                .iter()
                .any(|value| value.name == "gstreamer1" && value.version == "1.22")
        );
    }
}
