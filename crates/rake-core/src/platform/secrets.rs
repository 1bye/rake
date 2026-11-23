//! App-namespaced credentials; values never enter Rake's storage or inspection registry.
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
    #[doc(hidden)]
    pub fn key(&self, name: &str) -> Result<String, Error> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_platform_has_no_secret_namespace() {
        assert!(matches!(
            Platform::default().secrets(),
            Err(Error::MissingApplicationId)
        ));
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
}
