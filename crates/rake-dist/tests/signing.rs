//! Covers: public key generation/export, secure file access and non-overwriting enrollment.
//! Real: OS randomness, filesystem and Ed25519 public derivation.
//! Doubles: none. Requires: ordinary local filesystem access.
use rake_dist::{UpdateSigningKey, export_update_public_key, generate_update_key};
use std::fs;

#[test]
fn enrollment_and_export_preserve_an_existing_private_key() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private.key");
    let public = root.path().join("public.key");
    let reference = UpdateSigningKey::File {
        path: private.clone(),
    };
    generate_update_key(&reference, &public).unwrap();
    let original = fs::read(&private).unwrap();
    let original_public = fs::read(&public).unwrap();
    assert_eq!(original.len(), 45, "base64 seed and newline");
    // Native AppImage and Velopack clients accept hex or raw public keys.
    let public_text = std::str::from_utf8(&original_public).unwrap().trim();
    assert_eq!(
        public_text.len(),
        64,
        "export must be usable by native update clients"
    );
    let public_bytes: [u8; 32] = hex::decode(public_text).unwrap().try_into().unwrap();
    use base64::Engine as _;
    use ed25519_dalek::Signer as _;
    let seed: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(std::str::from_utf8(&original).unwrap().trim())
        .unwrap()
        .try_into()
        .unwrap();
    let signature = ed25519_dalek::SigningKey::from_bytes(&seed).sign(b"native update feed");
    ed25519_dalek::VerifyingKey::from_bytes(&public_bytes)
        .unwrap()
        .verify_strict(b"native update feed", &signature)
        .unwrap();
    assert!(generate_update_key(&reference, &root.path().join("other.pub")).is_err());
    assert!(
        generate_update_key(
            &UpdateSigningKey::File {
                path: root.path().join("other.key")
            },
            &public
        )
        .is_err()
    );
    assert_eq!(fs::read(&private).unwrap(), original);
    export_update_public_key(&reference, root.path(), &root.path().join("exported.pub")).unwrap();
    assert_eq!(
        fs::read(root.path().join("exported.pub")).unwrap(),
        original_public
    );
    assert!(export_update_public_key(&reference, root.path(), &public).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&private, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            export_update_public_key(&reference, root.path(), &root.path().join("unsafe.pub"))
                .is_err()
        );
    }
}
