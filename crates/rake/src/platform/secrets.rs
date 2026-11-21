//! App-namespaced credentials; values never enter Rake's storage or inspection registry.
#[cfg(feature = "gpui")]
use super::Request;
use super::{Error, Platform};
use zeroize::Zeroizing;

/// Owned secret bytes. Debug is redacted; serialization and Display are deliberately absent.
/// Rake-owned bytes are zeroized on drop; GPUI and OS copies are outside that guarantee.
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    /// Takes ownership of bytes without making another copy.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Explicitly exposes bytes for authentication. Never log or inspect this slice.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(token: String) -> Self {
        Self::new(token.into_bytes())
    }
}

impl From<Vec<u8>> for Secret {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// A credential namespace. Use the same application id across restarts.
#[derive(Clone)]
pub struct Secrets {
    app_id: String,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("app_id", &self.app_id)
            .finish()
    }
}

impl Platform {
    /// Opens the configured application's credential namespace without accessing the keychain.
    ///
    /// # Errors
    /// Returns [`Error::MissingApplicationId`] when the platform has no configured
    /// application id, or [`Error::InvalidInput`] if the id is outside 1–128 UTF-8 bytes.
    /// Credential operations apply the same length limit to names.
    pub fn secrets(&self) -> Result<Secrets, Error> {
        let app_id = self
            .application_id
            .as_deref()
            .ok_or(Error::MissingApplicationId)?;

        validate(app_id)?;

        Ok(Secrets {
            app_id: app_id.into(),
        })
    }
}

fn validate(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > 128 {
        return Err(Error::InvalidInput(
            "secret id/name must contain 1..=128 bytes".into(),
        ));
    }

    Ok(())
}

impl Secrets {
    #[cfg(any(feature = "gpui", test))]
    fn key(&self, name: &str) -> Result<String, Error> {
        validate(name)?;
        // Hex is injective, preserves case and cannot introduce URL delimiters or NULs.
        fn hex(value: &str) -> String {
            value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        Ok(format!(
            "rake-secret://v1/{}/{}",
            hex(&self.app_id),
            hex(name)
        ))
    }

    /// Reads bytes, returning `None` for an absent name. Native work runs on GPUI's
    /// background executor; Web explicitly fails without touching browser storage.
    #[cfg(feature = "gpui")]
    pub fn read(&self, name: &str, cx: &gpui::App) -> Request<Option<Secret>> {
        let key = self.key(name);

        #[cfg(not(target_family = "wasm"))]
        {
            let task = key.map(|key| cx.read_credentials(&key));

            Box::pin(async move {
                Ok(task?
                    .await
                    .map_err(native_error)?
                    .map(|(_, bytes)| Secret::new(bytes)))
            })
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = (key, cx);

            Box::pin(async { Err(Error::SecretsUnavailable) })
        }
    }

    /// Replaces a token. Await the request to observe completion; dropping it cannot
    /// undo an OS operation. Windows rejects values larger than 2560 bytes.
    #[cfg(feature = "gpui")]
    pub fn write(&self, name: &str, value: Secret, cx: &gpui::App) -> Request<()> {
        let key = self.key(name);

        #[cfg(not(target_family = "wasm"))]
        {
            if cfg!(target_os = "windows") && value.expose().len() > 2560 {
                return Box::pin(async { Err(Error::SecretSizeLimit) });
            }

            let task = key.map(|key| cx.write_credentials(&key, "rake-secret-v1", value.expose()));

            Box::pin(async move { task?.await.map_err(native_error) })
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = (key, value, cx);

            Box::pin(async { Err(Error::SecretsUnavailable) })
        }
    }

    /// Deletes a name; an absent entry is already deleted.
    #[cfg(feature = "gpui")]
    pub fn delete(&self, name: &str, cx: &gpui::App) -> Request<()> {
        let key = self.key(name);

        #[cfg(not(target_family = "wasm"))]
        {
            let task = key.map(|key| cx.delete_credentials(&key));

            Box::pin(async move {
                match task?.await {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        #[cfg(target_os = "macos")]
                        if macos_status(&error) == Some(-25300) {
                            return Ok(());
                        }

                        #[cfg(target_os = "windows")]
                        if error
                            .downcast_ref::<windows::core::Error>()
                            .is_some_and(|e| e.code().0 as u32 == 0x80070490)
                        {
                            return Ok(());
                        }

                        Err(native_error(error))
                    }
                }
            })
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = (key, cx);

            Box::pin(async { Err(Error::SecretsUnavailable) })
        }
    }
}

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
fn native_error(error: anyhow::Error) -> Error {
    #[cfg(target_os = "macos")]
    match macos_status(&error) {
        Some(-128 | -25293) => return Error::SecretAccessDenied,
        Some(-25308) => return Error::KeychainLocked,
        _ => {}
    }

    #[cfg(target_os = "windows")]
    if error
        .downcast_ref::<windows::core::Error>()
        .is_some_and(|e| e.code().0 as u32 == 0x80070005)
    {
        return Error::SecretAccessDenied;
    }

    #[cfg(target_os = "linux")]
    if let Some(oo7::Error::DBus(error)) = error.downcast_ref::<oo7::Error>() {
        use oo7::dbus::{Error as DbusError, ServiceError};

        match error {
            DbusError::Dismissed => return Error::SecretAccessDenied,
            DbusError::Service(ServiceError::IsLocked(_)) => return Error::KeychainLocked,
            DbusError::ZBus(zbus::Error::MethodError(name, _, _)) => match name.as_str() {
                "org.freedesktop.DBus.Error.ServiceUnknown" => return Error::NoSecretService,
                "org.freedesktop.DBus.Error.AccessDenied" => return Error::SecretAccessDenied,
                _ => {}
            },
            DbusError::ZBus(zbus::Error::FDO(error)) => match error.as_ref() {
                zbus::fdo::Error::ServiceUnknown(_) => return Error::NoSecretService,
                zbus::fdo::Error::AccessDenied(_) => return Error::SecretAccessDenied,
                _ => {}
            },
            _ => {}
        }
    }

    // Never forward backend messages: they may contain application content.
    let _ = error;

    Error::SecretsUnavailable
}

#[cfg(all(feature = "gpui", target_os = "macos"))]
fn macos_status(error: &anyhow::Error) -> Option<i32> {
    use gpui_pre_apple::keychain::KeychainError;

    if let Some(error) = error.downcast_ref::<KeychainError>() {
        return match error {
            KeychainError::Security { status, .. } => Some(*status),
            KeychainError::UserCanceled { .. } => Some(-128),
            _ => None,
        };
    }

    #[cfg(feature = "gpui-fast")]
    {
        use gpui_fast_apple::keychain::KeychainError;

        if let Some(error) = error.downcast_ref::<KeychainError>() {
            return match error {
                KeychainError::Security { status, .. } => Some(*status),
                KeychainError::UserCanceled { .. } => Some(-128),
                _ => None,
            };
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    #[cfg(all(feature = "gpui", not(target_family = "wasm")))]
    #[test]
    fn backend_error_messages_are_discarded() {
        let error = native_error(anyhow::anyhow!("DO-NOT-EXPOSE-backend-token"));

        assert!(!format!("{error:?}: {error}").contains("DO-NOT-EXPOSE"));
    }

    #[cfg(all(feature = "gpui", target_os = "macos"))]
    #[test]
    fn keychain_statuses_remain_typed() {
        use gpui_pre_apple::keychain::{KeychainError, KeychainOperation};
        let error = |status| {
            anyhow::Error::new(KeychainError::Security {
                operation: KeychainOperation::Read,
                status,
            })
        };

        assert!(matches!(native_error(error(-25308)), Error::KeychainLocked));
        assert!(matches!(
            native_error(error(-25293)),
            Error::SecretAccessDenied
        ));
    }

    #[test]
    fn default_platform_has_no_secret_namespace() {
        assert!(matches!(
            Platform::default().secrets(),
            Err(Error::MissingApplicationId)
        ));
    }

    #[cfg(feature = "gpui")]
    #[test]
    fn app_services_use_the_configured_namespace() {
        let services = crate::App::new("example.app").services().unwrap();
        let secrets = services.platform().secrets().unwrap();

        assert_eq!(
            secrets.key("token").unwrap(),
            "rake-secret://v1/6578616d706c652e617070/746f6b656e"
        );
    }

    #[test]
    fn names_do_not_collide() {
        let first = Platform::for_application("a/b").secrets().unwrap();
        let second = Platform::for_application("a").secrets().unwrap();

        assert_ne!(first.key("c").unwrap(), second.key("b/c").unwrap());
        assert_ne!(first.key("A").unwrap(), first.key("a").unwrap());
        assert!(first.key("").is_err());
    }

    #[test]
    fn conversions_reuse_owned_buffers() {
        let token = String::from("tökén");
        let token_ptr = token.as_ptr();
        let secret = Secret::from(token);

        assert_eq!(secret.expose(), "tökén".as_bytes());
        assert_eq!(secret.expose().as_ptr(), token_ptr);

        let bytes = vec![0, 255, 42];
        let bytes_ptr = bytes.as_ptr();
        let secret = Secret::from(bytes);

        assert_eq!(secret.expose(), &[0, 255, 42]);
        assert_eq!(secret.expose().as_ptr(), bytes_ptr);
    }

    #[test]
    fn diagnostics_and_debug_inspection_redact_values() {
        let value = Secret::new(b"DO-NOT-EXPOSE-token".to_vec());
        let collector = rake_diagnostics::Collector::new(16);
        let subscriber = tracing_subscriber::registry().with(collector.layer());

        tracing::subscriber::with_default(subscriber, || tracing::info!(?value, "credential"));
        let output = serde_json::to_string(&collector.snapshot_after(0, None)).unwrap();

        assert!(!output.contains("DO-NOT-EXPOSE"));
        assert!(output.contains("REDACTED"));
        let logs = serde_json::to_string(&collector.logs_after(0)).unwrap();

        assert!(!logs.contains("DO-NOT-EXPOSE"));
        assert!(logs.contains("REDACTED"));
        assert_eq!(format!("{value:?}"), "Secret([REDACTED])");
        let inspection = crate::inspection::Store::default();
        let snapshot = serde_json::to_string(&inspection.snapshot()).unwrap();

        assert!(!snapshot.contains("DO-NOT-EXPOSE"));
        #[cfg(feature = "devtools")]
        assert!(
            crate::storage::devtools::Storage::new()
                .catalog()
                .is_empty()
        );
    }
}
