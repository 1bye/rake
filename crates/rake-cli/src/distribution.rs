//! Portable read-only distribution planning, independent of Unix runtime IPC.

use rake_dist::Platform;
use std::path::PathBuf;

pub(crate) fn handles(arguments: &[String]) -> bool {
    if arguments.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--config"
                | "--legacy-schema"
                | "--native"
                | "--plan"
                | "--schema"
                | "--platform"
                | "--target-triple"
                | "--format"
                | "--trusted"
                | "--notary-profile"
                | "--certificate-thumbprint"
        )
    }) {
        return true;
    }

    if std::path::Path::new("Bundle.toml").exists()
        && !arguments.iter().any(|arg| arg == "--manifest-path")
    {
        return true;
    }

    let value = |flag: &str| {
        arguments
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    };

    let platform = match std::env::consts::OS {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        _ => Platform::Linux,
    };

    let target = rake_dist::host_target().ok();

    rake_dist::plan_with(
        std::path::Path::new(value("--manifest-path").unwrap_or("Cargo.toml")),
        value("--package").or_else(|| value("-p")),
        platform,
        target.as_deref(),
        None,
    )
    .map_or(true, |plan| {
        !plan.is_legacy()
            || !plan.config().custom_formats.is_empty()
            || !plan.config().component_recipes.is_empty()
            || !plan.config().dependencies.is_empty()
            || !plan.config().components.is_empty()
            || !plan.config().runtimes.is_empty()
            || !plan.config().sidecars.is_empty()
            || !plan.config().resources.is_empty()
    })
}

pub(crate) fn run(arguments: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    if arguments
        .iter()
        .any(|arg| matches!(arg.as_str(), "--schema" | "--legacy-schema"))
    {
        if arguments.iter().any(|arg| {
            !matches!(
                arg.as_str(),
                "--schema" | "--legacy-schema" | "--plan" | "--json"
            )
        }) {
            return Err("schema output accepts only --plan, --schema and --json".into());
        }

        println!(
            "{}",
            serde_json::to_string_pretty(
                &if arguments.iter().any(|arg| arg == "--legacy-schema") {
                    rake_dist::configuration_schema()
                } else {
                    rake_dist::bundle_file_schema()
                }
            )?
        );

        return Ok(true);
    }

    let planning = arguments.iter().any(|arg| arg == "--plan");

    let explicit = arguments.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--native" | "--format" | "--platform" | "--target-triple"
        )
    });

    let mut output = PathBuf::from("dist");
    let mut output_selected = false;
    let mut compatibility = arguments.iter().any(|arg| arg == "--compat");
    let mut entitlements = None;
    let mut binary = None;
    let mut release = true;
    let mut offline_components = false;
    let mut locked_components = false;
    let mut trust = rake_dist::TrustOptions::default();
    let mut manifest = PathBuf::from("Cargo.toml");
    let mut manifest_selected = false;
    let mut config_path = None;
    let mut package = None;
    let mut platform = std::env::consts::OS;
    let mut target = None;
    let mut formats = Vec::new();
    let mut features = Vec::new();
    let mut args = arguments.iter();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--plan" | "--json" | "--native" | "--compat" => {}
            "--entitlements" => {
                entitlements = Some(std::path::absolute(
                    args.next().ok_or("--entitlements requires a value")?,
                )?)
            }
            "--release" => release = true,
            "--features" => features.extend(
                args.next()
                    .ok_or("--features requires a value")?
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            ),
            "--trusted" => trust.required = true,
            "--sign-identity" => {
                let identity = args.next().ok_or("--sign-identity requires a value")?;

                trust.macos_identity = (identity != "-").then(|| identity.clone());
            }
            "--notary-profile" => {
                trust.notary_profile = Some(
                    args.next()
                        .ok_or("--notary-profile requires a value")?
                        .clone(),
                )
            }
            "--certificate-thumbprint" => {
                trust.windows_thumbprint = Some(
                    args.next()
                        .ok_or("--certificate-thumbprint requires a value")?
                        .clone(),
                )
            }
            "--offline-components" => offline_components = true,
            "--locked-components" => locked_components = true,
            "--debug" => release = false,
            "--output" => {
                output = args.next().ok_or("--output requires a value")?.into();
                output_selected = true;
            }
            "--bin" => binary = Some(args.next().ok_or("--bin requires a value")?.clone()),
            "--config" => {
                config_path = Some(PathBuf::from(
                    args.next().ok_or("--config requires a value")?,
                ))
            }
            "--manifest-path" => {
                manifest_selected = true;

                manifest = args
                    .next()
                    .ok_or("--manifest-path requires a value")?
                    .into()
            }
            "--package" | "-p" => {
                package = Some(args.next().ok_or("--package requires a value")?.as_str())
            }
            "--platform" => platform = args.next().ok_or("--platform requires a value")?,
            "--target-triple" => {
                target = Some(
                    args.next()
                        .ok_or("--target-triple requires a value")?
                        .as_str(),
                )
            }
            "--format" => formats.push(serde_json::from_value::<rake_dist::Format>(
                serde_json::Value::String(args.next().ok_or("--format requires a value")?.clone()),
            )?),
            "--target" => {
                if args.next().is_none_or(|target| target != "desktop") {
                    return Err("native planning requires --target desktop".into());
                }
            }
            _ => {
                return Err(
                    format!("unsupported distribution planning argument: {argument}").into(),
                );
            }
        }
    }

    let mut platform = match platform {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        "linux" => Platform::Linux,
        _ => return Err(format!("unsupported platform: {platform}").into()),
    };

    let directory = std::env::current_dir()?;

    let source = rake_dist::select_bundle_file(
        &directory,
        config_path.as_deref(),
        manifest_selected.then_some(manifest.as_path()),
        package,
        binary.as_deref(),
    )?;

    if let Some(source) = source {
        if compatibility {
            return Err("--compat is only supported by the legacy metadata adapter".into());
        }

        if !arguments.iter().any(|arg| arg == "--platform")
            && let rake_dist::Program::Prebuilt {
                target: declared, ..
            } = &source.program
            && let Some(target) = target.or(declared.as_deref())
        {
            platform = Platform::from_target(target)?;
        }

        let mut request = rake_dist::SourceBundleRequest::new(source, platform);

        request.target = target.map(str::to_owned);
        request.formats = formats;
        request.features = features;
        request.release = release;
        request.offline_components = offline_components;
        request.locked_components = locked_components;
        request.trust = trust;
        request.entitlements = entitlements;

        if output_selected {
            request.output = std::path::absolute(&output)?;
        }

        let resolved = rake_dist::plan_source(&request)?;

        if planning {
            if arguments.iter().any(|arg| arg == "--json") {
                println!("{}", serde_json::to_string_pretty(&resolved)?);
            } else {
                println!(
                    "Source: {:?}\nRoot: {}\nLock: {}\nOutput: {}\nTarget: {}\nProgram: {:?}",
                    resolved.source.origin,
                    resolved.source.root.display(),
                    resolved.source.lock_path.display(),
                    resolved.output.display(),
                    resolved.target,
                    resolved.program
                );

                for path in &resolved.shadowed_metadata {
                    println!(
                        "Shadowed Cargo metadata: {} [package.metadata.rake.bundle]",
                        path.display()
                    );
                }

                println!("{}", serde_json::to_string_pretty(&resolved.plan)?);
                println!("Native validation: pending");
            }
        } else {
            for path in &resolved.shadowed_metadata {
                eprintln!(
                    "Bundle.toml shadows {} [package.metadata.rake.bundle]",
                    path.display()
                );
            }

            let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = cancelled.clone();

            ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::Release))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&rake_dist::bundle_source(
                    &request,
                    &std::env::current_exe()?,
                    &cancelled
                )?)?
            );
        }

        return Ok(true);
    }

    let native_platform = match std::env::consts::OS {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        _ => Platform::Linux,
    };

    let host = if platform == native_platform && target.is_none() {
        Some(rake_dist::host_target()?)
    } else {
        None
    };

    let target = target.or(host.as_deref());

    if compatibility && formats.is_empty() {
        formats.push(match platform {
            Platform::Macos | Platform::Windows => rake_dist::Format::Zip,
            Platform::Linux => rake_dist::Format::Tar,
            _ => return Err("legacy compatibility requires macOS, Windows or Linux".into()),
        });
    }

    let plan = rake_dist::plan_with(
        &manifest,
        package,
        platform,
        target,
        if formats.is_empty() {
            None
        } else {
            Some(&formats)
        },
    )?;

    compatibility |= plan.is_legacy() && !explicit && platform != Platform::Windows;

    if planning {
        let (root, workspace, name) = legacy_paths(&manifest, package)?;

        let planned_output = if compatibility && !output_selected {
            workspace.join("dist").join(name).join("desktop-bundle")
        } else {
            std::path::absolute(&output)?
        };

        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"plan": plan, "native_validation": "pending",
                    "source": {"type": "cargo_metadata", "manifest": std::path::absolute(&manifest)?, "root": root, "lock_path": root.join("rake-dist.lock")},
                    "output": planned_output, "target": target, "shadowed_metadata": []})
            )?
        );
    } else {
        if compatibility && !output_selected {
            let (_, workspace, name) = legacy_paths(&manifest, package)?;

            output = workspace.join("dist").join(name).join("desktop-bundle");
        }

        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cancelled.clone();

        ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::Release))?;
        let request = rake_dist::BundleRequest {
            manifest,
            package: package.map(str::to_owned),
            platform,
            target: target.map(str::to_owned),
            formats,
            output,
            binary,
            release,
            compatibility,
            entitlements,
            features,
            offline_components,
            locked_components,
            trust,
        };

        let result = rake_dist::bundle(&request, &std::env::current_exe()?, &cancelled)?;

        if compatibility {
            let output = std::path::absolute(&request.output)?;
            let manifest = output.join("bundle.json");
            let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest)?)?;

            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"artifact":output.join(value["artifact"].as_str().ok_or("legacy artifact missing")?),"archive":output.join(value["archive"]["path"].as_str().ok_or("legacy archive missing")?),"manifest":manifest,"sha256":value["archive"]["sha256"],"profile":value["profile"],"signing":value["signing"]})
                )?
            );
        } else {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(true)
}

pub(crate) fn worker(
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    rake_dist::dispatch_worker(&[
        "__distribution-worker".into(),
        path.as_os_str().to_owned(),
        workspace.as_os_str().to_owned(),
    ])
    .ok_or("worker dispatcher did not recognize invocation")??;

    Ok(())
}

fn legacy_paths(
    manifest: &std::path::Path,
    package: Option<&str>,
) -> Result<(PathBuf, PathBuf, String), Box<dyn std::error::Error>> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;

    if !output.status.success() {
        return Err("could not resolve legacy output root".into());
    }

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    let selected = metadata["packages"]
        .as_array()
        .ok_or("Cargo packages missing")?
        .iter()
        .find(|item| {
            metadata["workspace_members"]
                .as_array()
                .is_some_and(|members| members.contains(&item["id"]))
                && package.is_none_or(|name| item["name"] == name)
        })
        .ok_or("Cargo package missing")?;

    let root = std::fs::canonicalize(
        std::path::Path::new(
            selected["manifest_path"]
                .as_str()
                .ok_or("Cargo manifest missing")?,
        )
        .parent()
        .ok_or("manifest parent missing")?,
    )?;

    let workspace = PathBuf::from(
        metadata["workspace_root"]
            .as_str()
            .ok_or("Cargo root missing")?,
    );

    let name = selected["name"]
        .as_str()
        .ok_or("Cargo package name missing")?
        .to_owned();

    Ok((root, workspace, name))
}
