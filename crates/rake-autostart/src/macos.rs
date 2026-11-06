//! macOS 13+ main-app login service; never writes launchd plists.
use crate::{Error, LaunchAtLogin, Operation, Status};
use objc2_foundation::{NSBundle, NSProcessInfo};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use std::path::Path;

fn status(value: SMAppServiceStatus) -> Status {
    match value {
        SMAppServiceStatus::NotRegistered => Status::Disabled,
        SMAppServiceStatus::Enabled => Status::Enabled,
        SMAppServiceStatus::RequiresApproval => Status::RequiresApproval,
        SMAppServiceStatus::NotFound => {
            Status::Unsupported("SMAppService could not find the signed main app bundle".into())
        }
        _ => Status::Unsupported(format!("unknown SMAppService status {}", value.0)),
    }
}

pub(crate) fn apply(login: &LaunchAtLogin, operation: Operation) -> Result<Status, Error> {
    // Runtime gate preserves rake-dist's older minimum_system_version; no deployment target bump.
    if NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        < 13
    {
        return Ok(Status::Unsupported(
            "SMAppService requires macOS 13 or newer".into(),
        ));
    }
    let bundle = NSBundle::mainBundle();
    let path = bundle.bundlePath().to_string();
    if Path::new(&path)
        .extension()
        .is_none_or(|extension| extension != "app")
        || bundle
            .bundleIdentifier()
            .is_none_or(|id| id.to_string() != login.app_id)
    {
        return Ok(Status::Unsupported("launch at login requires a signed .app bundle whose CFBundleIdentifier matches the app id; unbundled cargo run is unsupported".into()));
    }
    if !login.args.is_empty() {
        return Ok(Status::Unsupported(
            "SMAppService.mainAppService does not support custom login arguments".into(),
        ));
    }

    // SAFETY: mainAppService and its methods exist on macOS 13+, checked above.
    // NSBundle identifies this process's main app; objc2 owns returned objects.
    unsafe {
        let service = SMAppService::mainAppService();
        let current = status(service.status());
        match operation {
            Operation::Enable if current == Status::Disabled => {
                if let Err(error) = service.registerAndReturnError() {
                    let applied = status(service.status());
                    if applied == Status::RequiresApproval {
                        return Ok(applied);
                    }
                    return Err(Error::Service(error.to_string()));
                }
            }
            Operation::Disable if matches!(current, Status::Enabled | Status::RequiresApproval) => {
                service
                    .unregisterAndReturnError()
                    .map_err(|error| Error::Service(error.to_string()))?;
            }
            _ => {}
        }

        Ok(status(service.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_all_native_statuses_and_unknown_future_values() {
        assert_eq!(status(SMAppServiceStatus::NotRegistered), Status::Disabled);
        assert_eq!(status(SMAppServiceStatus::Enabled), Status::Enabled);
        assert_eq!(
            status(SMAppServiceStatus::RequiresApproval),
            Status::RequiresApproval
        );
        assert!(matches!(
            status(SMAppServiceStatus::NotFound),
            Status::Unsupported(_)
        ));
        assert!(matches!(
            status(SMAppServiceStatus(99)),
            Status::Unsupported(_)
        ));
    }
}
