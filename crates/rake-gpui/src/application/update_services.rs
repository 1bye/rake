//! Startup composition for the shared native update service and controller.

#![cfg(all(feature = "updates", not(target_family = "wasm")))]

use super::AppError;
use crate::Resources;
use crate::services::ServiceRegistry;
use rake_updates::{UpdateBinding, UpdateController, UpdateService};

/// Validates explicit update services, then installs the shared service/controller.
///
/// A compiled policy is authoritative when present. Manual services/controllers
/// remain usable, but only when their complete immutable bindings agree with it.
/// Installed metadata is still checked for packaged policies even when both values
/// were supplied manually; an unbundled build simply skips automatic installation.
pub(super) fn install(
    mut registry: ServiceRegistry,
    distribution: Option<(&'static str, &'static str)>,
) -> Result<ServiceRegistry, AppError> {
    let compiled = distribution
        .map(|(policy, _)| rake_updates::parse_policy(policy))
        .transpose()
        .map_err(AppError::Updates)?;

    let manual_service = registry.get::<UpdateService>().ok();
    let manual_controller = registry.get::<UpdateController>().ok();

    // A packaged policy must still authorize the current installation descriptor,
    // even when callers supplied both service values themselves.
    let installed = match distribution {
        Some((policy, name)) => match Resources::discover(name) {
            Ok(resources) => Some(UpdateService::new(policy, &resources)?),
            Err(crate::resources::Error::Unbundled) => None,
            Err(error) => {
                return Err(AppError::Updates(rake_updates::Error::Policy(
                    error.to_string(),
                )));
            }
        },
        None => None,
    };

    validate_bindings(
        compiled.as_ref(),
        manual_service.as_deref(),
        manual_controller.as_deref(),
        installed.as_ref(),
    )?;

    if registry.get::<UpdateService>().is_err() {
        let service = manual_controller
            .as_ref()
            .map(|controller| controller.service())
            .or(installed);

        if let Some(service) = service {
            registry = registry.with(service).map_err(|error| {
                AppError::Updates(rake_updates::Error::Policy(error.to_string()))
            })?;
        }
    }

    if registry.get::<UpdateController>().is_err()
        && let Ok(service) = registry.get::<UpdateService>()
    {
        registry = registry
            .with(UpdateController::new((*service).clone()))
            .map_err(|error| AppError::Updates(rake_updates::Error::Policy(error.to_string())))?;
    }

    Ok(registry)
}

fn validate_bindings(
    compiled: Option<&UpdateBinding>,
    manual_service: Option<&UpdateService>,
    manual_controller: Option<&UpdateController>,
    installed: Option<&UpdateService>,
) -> Result<(), AppError> {
    if let Some(compiled) = compiled {
        if let Some(service) = manual_service {
            require_match(
                "manual UpdateService and compiled update policy",
                service.binding(),
                compiled,
            )?;
        }

        if let Some(controller) = manual_controller {
            require_match(
                "manual UpdateController and compiled update policy",
                controller.binding(),
                compiled,
            )?;
        }

        if let Some(installed) = installed {
            require_match(
                "installed UpdateService and compiled update policy",
                installed.binding(),
                compiled,
            )?;
        }
    }

    if let (Some(service), Some(controller)) = (manual_service, manual_controller) {
        require_match(
            "registered UpdateController and UpdateService",
            controller.binding(),
            service.binding(),
        )?;
    }

    Ok(())
}

fn require_match(
    subject: &str,
    actual: &UpdateBinding,
    expected: &UpdateBinding,
) -> Result<(), AppError> {
    if actual == expected {
        Ok(())
    } else {
        Err(AppError::Updates(rake_updates::Error::Policy(format!(
            "update binding conflict: {subject}"
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn binding() -> UpdateBinding {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "identifier": "dev.rake.application",
            "version": "1.0.0",
            "package_version": "1.0.0",
            "target": "x86_64-unknown-linux-gnu",
            "format": "deb",
            "owner": "disabled",
            "channel": "stable",
            "base_url": null,
            "public_keys": {}
        }))
        .unwrap()
    }

    fn policy(identifier: &str) -> String {
        format!(
            r#"{{"schema_version":1,"identifier":"{identifier}","version":"1.0.0","package_version":"1.0.0","target":"x86_64-unknown-linux-gnu","format":"deb","owner":"disabled","channel":"stable","base_url":null,"public_keys":{{}}}}"#
        )
    }

    fn service(policy: &str) -> UpdateService {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rake-update-services-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        std::fs::create_dir(&root).unwrap();
        let binding = rake_updates::parse_policy(policy).unwrap();

        let installation = serde_json::json!({
            "schema_version": 1,
            "identifier": binding.identifier,
            "version": binding.version,
            "package_version": binding.package_version,
            "target": binding.target,
            "platform": "linux",
            "format": "deb",
            "update_owner": "disabled",
            "updates": {},
            "helpers": {},
            "runtimes": {},
            "frameworks": {},
            "native_groups": {}
        });

        std::fs::write(
            root.join("rake-installation.json"),
            serde_json::to_vec(&installation).unwrap(),
        )
        .unwrap();
        let resources = Resources::load(&root).unwrap();
        let service = UpdateService::new(policy, &resources).unwrap();

        std::fs::remove_dir_all(root).unwrap();

        service
    }

    #[test]
    fn exact_binding_equality_includes_public_keys() {
        let expected = binding();
        let mut different = expected.clone();

        different.public_keys.insert("appimage".into(), vec![1]);
        assert!(
            require_match(
                "manual UpdateService and compiled update policy",
                &expected,
                &expected
            )
            .is_ok()
        );
        assert!(
            require_match(
                "manual UpdateService and compiled update policy",
                &different,
                &expected
            )
            .is_err()
        );
    }

    #[test]
    fn conflicting_diagnostics_do_not_include_policy_contents() {
        let expected = binding();
        let mut different = expected.clone();

        different.base_url = Some("https://private.example.invalid/secret-channel".into());
        let error = require_match(
            "manual UpdateService and compiled update policy",
            &different,
            &expected,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("manual UpdateService"));
        assert!(!error.contains("private.example.invalid"));
    }

    #[test]
    fn manual_services_and_controllers_share_the_compiled_fixture_binding() {
        let fixture_policy = policy("dev.rake.fixture");
        let fixture_service = service(&fixture_policy);
        let controller = UpdateController::new(fixture_service.clone());
        let compiled = rake_updates::parse_policy(&fixture_policy).unwrap();

        validate_bindings(
            Some(&compiled),
            Some(&fixture_service),
            Some(&controller),
            Some(&fixture_service),
        )
        .unwrap();

        let other = service(&policy("dev.rake.other"));

        assert!(
            validate_bindings(Some(&compiled), Some(&other), None, Some(&fixture_service)).is_err()
        );
        let other_controller = UpdateController::new(other);

        assert!(
            validate_bindings(None, Some(&fixture_service), Some(&other_controller), None,)
                .is_err()
        );
    }
}
