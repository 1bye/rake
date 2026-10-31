//! Native version conversion preserves prerelease ordering or requires a mapping.
use crate::{Error, Format};

/// Native version and optional package-release component.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PackageVersion {
    /// Format-specific version string.
    pub version: String,
    /// Native package release/revision when separate from upstream version.
    pub release: Option<String>,
}

pub(crate) fn convert(
    version: &semver::Version,
    format: Format,
    explicit: Option<&str>,
    build: Option<&str>,
) -> Result<PackageVersion, Error> {
    let numeric = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let pre = !version.pre.is_empty();
    let mapped = explicit.unwrap_or(&numeric);
    let mut release = None;

    let native = match format {
        Format::Msi | Format::Msix => {
            if pre && explicit.is_none() {
                return Err(Error::Field(
                    "prerelease MSI/MSIX requires explicit package_version for monotonic upgrades"
                        .into(),
                ));
            }

            let parts: Vec<u64> = mapped
                .split('.')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .map_err(|_| Error::Field("package_version must be numeric".into()))?;

            let valid = if format == Format::Msi {
                parts.len() == 3 && parts[0] <= 255 && parts[1] <= 255 && parts[2] <= 65535
            } else {
                (3..=4).contains(&parts.len()) && parts.iter().all(|n| *n <= 65535)
            };

            if !valid {
                return Err(Error::Field(
                    "package_version exceeds native Windows limits".into(),
                ));
            }

            if format == Format::Msix && parts.len() == 3 {
                format!("{mapped}.0")
            } else {
                mapped.into()
            }
        }
        Format::Deb | Format::Rpm => {
            release = Some("1".into());

            explicit.map(str::to_owned).unwrap_or_else(|| {
                if pre {
                    format!("{numeric}~{}", version.pre.as_str().replace('-', "."))
                } else {
                    numeric
                }
            })
        }
        Format::Arch => {
            if pre && explicit.is_none() {
                return Err(Error::Field(
                    "Arch prerelease requires explicit package_version validated with vercmp"
                        .into(),
                ));
            }

            release = Some("1".into());

            mapped.into()
        }
        Format::App | Format::Dmg => {
            if pre && build.is_none() {
                return Err(Error::Field(
                    "macOS prerelease requires a monotonic build_number".into(),
                ));
            }

            let parts: Vec<_> = mapped.split('.').collect();

            if parts.len() != 3 || parts.iter().any(|part| part.parse::<u64>().is_err()) {
                return Err(Error::Field(
                    "macOS package_version requires three numeric fields".into(),
                ));
            }

            mapped.into()
        }
        Format::Velopack | Format::Appimage => {
            let mapped = explicit
                .map(str::to_owned)
                .unwrap_or_else(|| version.to_string());

            semver::Version::parse(&mapped)
                .map_err(|_| Error::Field("updater package_version must use SemVer".into()))?;

            mapped
        }
        _ => explicit
            .map(str::to_owned)
            .unwrap_or_else(|| version.to_string()),
    };

    if native.is_empty()
        || native.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | '\'' | '"' | '$' | '`')
        })
    {
        return Err(Error::Field("invalid package_version".into()));
    }

    if format == Format::Arch && native.contains([':', '-']) {
        return Err(Error::Field("invalid Arch package_version".into()));
    }

    if format == Format::Rpm && native.contains('-') {
        return Err(Error::Field("invalid RPM package_version".into()));
    }

    if let Some(build) = build {
        let parts: Vec<_> = build.split('.').collect();

        if parts.is_empty()
            || parts.len() > 3
            || parts
                .iter()
                .any(|p| p.is_empty() || p.parse::<u32>().is_err())
        {
            return Err(Error::Field(
                "build_number requires one to three numeric components".into(),
            ));
        }
    }

    Ok(PackageVersion {
        version: native,
        release,
    })
}
