// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Módulo de gestión y aplicación de actualizaciones seguras para eMuleRust.

pub mod constants;
pub mod error;
pub mod installer;
pub mod manifest;
pub mod service;
pub mod verifier;

pub use constants::*;
pub use error::UpdaterError;
pub use installer::*;
pub use manifest::*;
pub use service::*;
pub use verifier::*;

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use tempfile::tempdir;

    #[test]
    fn test_manifest_parse_and_target_resolution() {
        let json_manifest = r#"{
            "version": "0.3.1",
            "releaseDate": "2026-11-20T12:00:00Z",
            "channel": "stable",
            "targets": {
                "x86_64-pc-windows-msvc": {
                    "url": "https://example.com/mule-windows.zip",
                    "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                    "size": 1024,
                    "signature": "112233"
                },
                "x86_64-unknown-linux-gnu": {
                    "url": "https://example.com/mule-linux.tar.gz",
                    "sha256": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "size": 2048,
                    "signature": "445566"
                }
            }
        }"#;

        let manifest = UpdateManifest::parse_str(json_manifest).expect("Manifiesto JSON válido");
        assert_eq!(manifest.version, "0.3.1");
        assert_eq!(manifest.channel, "stable");

        let win_target = manifest.resolve_target("x86_64-pc-windows-msvc").expect("Target Windows presente");
        assert_eq!(win_target.size, 1024);
        assert_eq!(win_target.url, "https://example.com/mule-windows.zip");

        let linux_target = manifest.resolve_target("x86_64-unknown-linux-gnu").expect("Target Linux presente");
        assert_eq!(linux_target.size, 2048);

        assert!(manifest.resolve_target("arm-unknown-linux-gnueabi").is_none());
    }

    #[test]
    fn test_version_comparison_is_newer_than() {
        let make_manifest = |v: &str| UpdateManifest {
            version: v.to_string(),
            min_supported_version: None,
            release_date: "2026-11-20T12:00:00Z".to_string(),
            channel: "stable".to_string(),
            changelog_url: None,
            summary: None,
            targets: std::collections::HashMap::new(),
        };

        // Versión remota mayor
        assert!(make_manifest("0.3.1").is_newer_than("0.3.0"));
        assert!(make_manifest("1.0.0").is_newer_than("0.9.9"));
        assert!(make_manifest("0.4.0").is_newer_than("0.3.9"));

        // Misma versión numérica pero local es beta / prerelease y remota es estable
        assert!(make_manifest("0.3.0").is_newer_than("0.3.0-beta.2"));

        // Versión local igual o más reciente
        assert!(!make_manifest("0.3.0").is_newer_than("0.3.0"));
        assert!(!make_manifest("0.2.9").is_newer_than("0.3.0"));
        assert!(!make_manifest("0.3.0-beta.1").is_newer_than("0.3.0"));
    }

    #[test]
    fn test_ed25519_verification_offline() {
        let rng = SystemRandom::new();
        let pkcs8_doc = Ed25519KeyPair::generate_pkcs8(&rng).expect("Generación PKCS8 de prueba");
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8_doc.as_ref()).expect("Par de claves Ed25519");

        let public_key_bytes = key_pair.public_key().as_ref();
        let pubkey_hex = hex::encode(public_key_bytes);

        let payload = b"Prueba binaria simulada de actualizacion de eMuleRust";
        let sig = key_pair.sign(payload);
        let sig_hex = hex::encode(sig.as_ref());
        let expected_sha256 = compute_sha256_hex(payload);

        // Caso 1: Todo correcto
        let res = verify_payload(payload, &expected_sha256, &sig_hex, &pubkey_hex);
        assert!(res.is_ok(), "La verificación criptográfica debió ser satisfactoria");

        // Caso 2: Hash SHA-256 manipulado
        let bad_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
        let res_bad_hash = verify_payload(payload, bad_sha256, &sig_hex, &pubkey_hex);
        assert!(matches!(res_bad_hash, Err(UpdaterError::ChecksumMismatch { .. })));

        // Caso 3: Firma alterada
        let mut tampered_sig = sig.as_ref().to_vec();
        tampered_sig[0] ^= 0xFF;
        let tampered_sig_hex = hex::encode(tampered_sig);
        let res_bad_sig = verify_payload(payload, &expected_sha256, &tampered_sig_hex, &pubkey_hex);
        assert!(matches!(res_bad_sig, Err(UpdaterError::CryptographicSignatureInvalid)));
    }

    #[test]
    fn test_staging_and_replacement_simulation() {
        let dir = tempdir().expect("Directorio temporal");
        let fake_exe = dir.path().join("mule-daemon.bin");
        let fake_staging = get_staging_executable_path(&fake_exe);

        // Crear ejecutable original
        std::fs::write(&fake_exe, b"version_antigua").expect("Escritura ejecutable original");

        // Escribir archivo en staging
        let new_bytes = b"version_nueva_actualizada";
        write_staging_binary(&fake_staging, new_bytes).expect("Escritura en staging");

        assert!(fake_staging.exists());

        // Aplicar reemplazo
        replace_current_executable(&fake_staging, &fake_exe).expect("Reemplazo atómico");

        let content_after = std::fs::read(&fake_exe).expect("Lectura tras sustitución");
        assert_eq!(content_after, new_bytes);

        // Limpieza de respaldo si existiese
        cleanup_old_backup_if_present(&fake_exe);
    }
}
