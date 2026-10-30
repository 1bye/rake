//! Provider metadata signatures bind exact bytes before parsing or staging.
use crate::Error;
use ed25519_dalek::{Signature, VerifyingKey};
pub(crate) fn verify(key: &[u8], message: &[u8], signature: &str) -> Result<(), Error> {
    let key = if key.len() == 32 {
        key.to_vec()
    } else {
        hex::decode(
            std::str::from_utf8(key)
                .map_err(|error| Error::Policy(error.to_string()))?
                .trim(),
        )
        .map_err(|error| Error::Policy(error.to_string()))?
    };
    let key: [u8; 32] = key.try_into().map_err(|_| {
        Error::Policy(
            "public verification key requires 32 raw bytes or 64 hexadecimal characters".into(),
        )
    })?;
    let key = VerifyingKey::from_bytes(&key).map_err(|error| Error::Policy(error.to_string()))?;
    let signature =
        hex::decode(signature.trim()).map_err(|error| Error::Provider(error.to_string()))?;
    let signature =
        Signature::from_slice(&signature).map_err(|error| Error::Provider(error.to_string()))?;
    key.verify_strict(message, &signature)
        .map_err(|error| Error::Provider(format!("untrusted update metadata: {error}")))
}
