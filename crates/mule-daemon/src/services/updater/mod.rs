// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Módulo de gestión y aplicación de actualizaciones seguras para eMuleRust.

pub mod constants;
pub mod error;
pub mod http_fetch;
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
        assert!(make_manifest("0.4.0").is_newer_than("0.4.0-beta.2"));

        // Progresión entre versiones de prelanzamiento SemVer
        assert!(make_manifest("0.4.0-beta.2").is_newer_than("0.4.0-beta.1"));
        assert!(make_manifest("0.4.0-beta.10").is_newer_than("0.4.0-beta.2"));
        assert!(make_manifest("0.4.0-rc.1").is_newer_than("0.4.0-beta.9"));
        assert!(!make_manifest("0.4.0-beta.1").is_newer_than("0.4.0-beta.2"));
        assert!(!make_manifest("0.4.0-beta.2").is_newer_than("0.4.0-beta.2"));

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

    /// Construye una cabecera ELF de 64 bits little-endian sintética con la máquina indicada.
    fn make_elf64_header(machine: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 64];
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = ELF_CLASS_64;
        buf[5] = ELF_DATA_LITTLE_ENDIAN;
        buf[6] = 1;
        buf[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN (PIE)
        buf[18..20].copy_from_slice(&machine.to_le_bytes());
        buf
    }

    #[test]
    fn test_validate_elf_architecture_matching_and_mismatch() {
        let elf_x86 = make_elf64_header(ELF_MACHINE_X86_64);
        let elf_arm = make_elf64_header(ELF_MACHINE_AARCH64);
        let mut pe_win = vec![0u8; 64];
        pe_win[0..2].copy_from_slice(&PE_MAGIC);

        // Coincidencia válida en cada arquitectura Linux
        assert!(validate_executable_for_triple(&elf_x86, "x86_64-unknown-linux-gnu").is_ok());
        assert!(validate_executable_for_triple(&elf_arm, "aarch64-unknown-linux-gnu").is_ok());
        assert!(validate_executable_for_triple(&pe_win, "x86_64-pc-windows-msvc").is_ok());

        // Rechazo cruzado ARM64 <-> x86_64
        let err_arm_on_x86 = validate_executable_for_triple(&elf_arm, "x86_64-unknown-linux-gnu")
            .expect_err("Debe rechazar ELF aarch64 en host x86_64");
        assert!(matches!(
            err_arm_on_x86,
            UpdaterError::ArchitectureMismatch { .. }
        ));

        let err_x86_on_arm = validate_executable_for_triple(&elf_x86, "aarch64-unknown-linux-gnu")
            .expect_err("Debe rechazar ELF x86_64 en Raspberry Pi aarch64");
        assert!(matches!(
            err_x86_on_arm,
            UpdaterError::ArchitectureMismatch { .. }
        ));

        // Rechazos de formato inválido y vacío
        assert!(matches!(
            validate_executable_for_triple(&[], "x86_64-unknown-linux-gnu"),
            Err(UpdaterError::EmptyPayload)
        ));
        assert!(matches!(
            validate_executable_for_triple(b"no-soy-un-ejecutable-valido-1234", "x86_64-unknown-linux-gnu"),
            Err(UpdaterError::InvalidExecutableFormat(_))
        ));
        assert!(matches!(
            validate_executable_for_triple(&pe_win, "x86_64-unknown-linux-gnu"),
            Err(UpdaterError::InvalidExecutableFormat(_))
        ));
        assert!(matches!(
            validate_executable_for_triple(&elf_x86, "x86_64-pc-windows-msvc"),
            Err(UpdaterError::InvalidExecutableFormat(_))
        ));
    }

    #[tokio::test]
    async fn test_stage_and_apply_local_binary_to_target_replaces_atomically() {
        let dir = tempdir().expect("Directorio temporal");
        let target_exe = dir.path().join("mule-daemon");
        std::fs::write(&target_exe, b"binario-anterior").expect("Escribir binario anterior");

        let mut payload = match UpdateManifest::current_platform_triple() {
            "aarch64-unknown-linux-gnu" => make_elf64_header(ELF_MACHINE_AARCH64),
            "x86_64-pc-windows-msvc" => {
                let mut pe = vec![0u8; 64];
                pe[0..2].copy_from_slice(&PE_MAGIC);
                pe
            }
            _ => make_elf64_header(ELF_MACHINE_X86_64),
        };
        payload.extend_from_slice(b"cuerpo-compilado-nuevo");
        let good_sha = compute_sha256_hex(&payload);

        let svc = UpdaterService::new("http://127.0.0.1/manifest.json".to_string(), "0.4.0".to_string());

        // Suma SHA-256 incorrecta debe abortar sin tocar el binario previo
        let bad_res = svc
            .stage_and_apply_local_binary_to_target(
                &payload,
                Some("0000000000000000000000000000000000000000000000000000000000000000"),
                Some("local-bad"),
                &target_exe,
                UpdateManifest::current_platform_triple(),
            )
            .await;
        assert!(matches!(bad_res, Err(UpdaterError::ChecksumMismatch { .. })));
        assert_eq!(std::fs::read(&target_exe).unwrap(), b"binario-anterior");

        // Suma SHA-256 correcta sustituye atómicamente el ejecutable
        let ok_res = svc
            .stage_and_apply_local_binary_to_target(
                &payload,
                Some(&good_sha),
                Some("compilacion-v0.4.1"),
                &target_exe,
                UpdateManifest::current_platform_triple(),
            )
            .await
            .expect("Instalación local debe tener éxito");

        assert_eq!(ok_res.sha256, good_sha);
        assert_eq!(ok_res.label, "compilacion-v0.4.1");
        assert_eq!(std::fs::read(&target_exe).unwrap(), payload);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target_exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "El binario debe conservar permisos ejecutables");
        }
    }

    #[test]
    fn test_is_executable_dir_writable_tempdir() {
        let dir = tempdir().expect("Directorio temporal");
        let fake_exe = dir.path().join("mule-daemon");
        assert!(is_executable_dir_writable(&fake_exe));
        assert!(!is_executable_dir_writable(&std::path::PathBuf::from(
            "/ruta/inexistente/pulse-core-no-existe/mule-daemon"
        )));
    }

    #[tokio::test]
    async fn test_linux_hardlink_backup_and_rollback_cycle() {
        let dir = tempdir().expect("Directorio temporal");
        let target_exe = dir.path().join("mule-daemon");

        let mut old_bin = make_elf64_header(ELF_MACHINE_X86_64);
        old_bin.extend_from_slice(b"v1-estable");
        std::fs::write(&target_exe, &old_bin).expect("Escribir v1");

        let mut new_bin = make_elf64_header(ELF_MACHINE_X86_64);
        new_bin.extend_from_slice(b"v2-experimental");

        let svc = UpdaterService::new("http://127.0.0.1/manifest.json".to_string(), "0.4.0".to_string());
        svc.stage_and_apply_local_binary_to_target(
            &new_bin,
            None,
            Some("v2"),
            &target_exe,
            "x86_64-unknown-linux-gnu",
        )
        .await
        .expect("Instalación v2");

        // Se ha creado el respaldo .old y coexiste tras limpiar posibles .update huérfanos
        assert!(has_rollback_backup(&target_exe));
        let orphan_staging = get_staging_executable_path(&target_exe);
        std::fs::write(&orphan_staging, b"huerfano").expect("Crear staging huérfano");
        cleanup_stale_staging_if_present(&target_exe);
        assert!(!orphan_staging.exists());
        assert!(has_rollback_backup(&target_exe));
        assert_eq!(std::fs::read(&target_exe).unwrap(), new_bin);

        // El rollback restaura exactamente v1 y consume .old
        svc.rollback_update_to_target(&target_exe, "x86_64-unknown-linux-gnu")
            .await
            .expect("Rollback debe restaurar v1");
        assert_eq!(std::fs::read(&target_exe).unwrap(), old_bin);
        assert!(!has_rollback_backup(&target_exe));

        // Un segundo rollback sin .old debe devolver NoRollbackAvailable
        let second = svc
            .rollback_update_to_target(&target_exe, "x86_64-unknown-linux-gnu")
            .await;
        assert!(matches!(second, Err(UpdaterError::NoRollbackAvailable)));
    }

    #[test]
    fn test_resolve_beta_manifest_url_from_github_releases_json() {
        let payload = r#"[
            {
                "tag_name": "v0.4.8-beta.1",
                "draft": true,
                "prerelease": true,
                "assets": [
                    { "name": "manifest.json", "browser_download_url": "https://example.invalid/draft/manifest.json" }
                ]
            },
            {
                "tag_name": "v0.4.7-beta.1",
                "draft": false,
                "prerelease": true,
                "assets": [
                    { "name": "PulseCore-v0.4.7-beta.1-linux-x64.tar.gz", "browser_download_url": "https://example.invalid/tar" },
                    { "name": "manifest.json", "browser_download_url": "https://github.com/caraveliko/PulseCore/releases/download/v0.4.7-beta.1/manifest.json" }
                ]
            }
        ]"#;
        let resolved = super::service::resolve_beta_manifest_url_from_releases_json(payload);
        assert_eq!(
            resolved.as_deref(),
            Some("https://github.com/caraveliko/PulseCore/releases/download/v0.4.7-beta.1/manifest.json")
        );
    }

    #[tokio::test]
    async fn test_updater_channel_switch_and_validation() {
        let svc = UpdaterService::new("http://127.0.0.1/manifest.json".to_string(), "0.4.7-beta.1".to_string());
        assert_eq!(svc.get_channel().await, "stable");

        let ch = svc.set_channel("beta").await.expect("cambio a beta");
        assert_eq!(ch, "beta");
        assert_eq!(svc.get_channel().await, "beta");

        let err = svc.set_channel("nightly").await.unwrap_err();
        assert!(matches!(err, UpdaterError::InvalidChannel(_)));
        assert_eq!(svc.get_channel().await, "beta");
    }
}
