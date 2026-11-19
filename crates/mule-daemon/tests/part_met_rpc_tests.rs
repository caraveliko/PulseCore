// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::Database;
use mule_daemon::services::downloads::DownloadService;
use mule_format::{met::tags::*, PARTFILE_VERSION};
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

fn create_part_met_file(
    path: &std::path::Path,
    filename: &str,
    file_size: u64,
    transferred: u64,
    file_hash: [u8; 16],
    part_hashes: &[[u8; 16]],
    gaps: &[(u64, u64)],
) {
    let mut bytes = Vec::new();
    // Encabezado
    bytes.push(PARTFILE_VERSION); // 0xe0
    bytes.extend_from_slice(&1_700_000_000u32.to_le_bytes()); // mod date
    bytes.extend_from_slice(&file_hash); // file hash

    // Cantidad de hashes de partes
    bytes.extend_from_slice(&(part_hashes.len() as u16).to_le_bytes());
    for h in part_hashes {
        bytes.extend_from_slice(h);
    }

    // Cantidad de etiquetas: 3 (filename, filesize, transferred) + filesize_hi opcional + intervalos (gaps) (2 etiquetas por intervalo)
    let has_hi = file_size > u32::MAX as u64;
    let tag_count = 3 + (if has_hi { 1 } else { 0 }) + (gaps.len() as u32 * 2);
    bytes.extend_from_slice(&tag_count.to_le_bytes());

    // Etiqueta: FT_FILENAME
    bytes.push(0x82); // string
    bytes.push(FT_FILENAME);
    let fn_bytes = filename.as_bytes();
    bytes.extend_from_slice(&(fn_bytes.len() as u16).to_le_bytes());
    bytes.extend_from_slice(fn_bytes);

    // Etiqueta: FT_FILESIZE
    bytes.push(0x83); // uint32
    bytes.push(FT_FILESIZE);
    bytes.extend_from_slice(&(file_size as u32).to_le_bytes());

    if has_hi {
        bytes.push(0x83); // uint32
        bytes.push(FT_FILESIZE_HI);
        bytes.extend_from_slice(&((file_size >> 32) as u32).to_le_bytes());
    }

    // Etiqueta: FT_TRANSFERRED
    bytes.push(0x83); // uint32
    bytes.push(FT_TRANSFERRED);
    bytes.extend_from_slice(&(transferred as u32).to_le_bytes());

    // Intervalos (gaps)
    for &(s, e) in gaps {
        // GAPSTART
        bytes.push(0x83);
        bytes.push(FT_GAPSTART);
        bytes.extend_from_slice(&(s as u32).to_le_bytes());

        // GAPEND
        bytes.push(0x83);
        bytes.push(FT_GAPEND);
        bytes.extend_from_slice(&(e as u32).to_le_bytes());
    }

    let mut f = File::create(path).unwrap();
    f.write_all(&bytes).unwrap();
}

#[test]
fn test_part_met_inspection_and_import_nominal() {
    let temp_dir = tempdir().unwrap();
    let external_dir = tempdir().unwrap();
    let download_target_dir = tempdir().unwrap();

    let db_path = temp_dir.path().join("mule.db");
    let mut service = DownloadService::new(&db_path, temp_dir.path().join("temp")).unwrap();

    // Crear datos de archivo de 1000 bytes con cabecera Matroska válida y su hash eD2K
    let mut part_data = vec![0x42u8; 1000];
    let mkv_hdr = mule_network_policy::content_policy::MINIMAL_VALID_MKV_HEADER;
    part_data[..mkv_hdr.len()].copy_from_slice(mkv_hdr);
    let mut hasher = mule_format::ed2k::hash::Ed2kHasher::new();
    hasher.update(&part_data);
    let part_hash = hasher.finalize().unwrap().root_hash.0;

    let met_path = external_dir.path().join("001.part.met");
    let part_path = external_dir.path().join("001.part");

    // Crear archivo .part
    std::fs::write(&part_path, &part_data).unwrap();

    // Crear .part.met sin intervalos (parte única 100 % completa)
    create_part_met_file(
        &met_path,
        "sample.mkv",
        1000,
        1000,
        part_hash,
        &[part_hash],
        &[], // no gaps
    );

    // 1. Probar inspect_part_met
    let inspection = service.inspect_part_met(&met_path, None).unwrap();
    assert_eq!(inspection.file_name, "sample.mkv");
    assert_eq!(inspection.total_size, 1000);
    assert_eq!(inspection.part_count, 1);
    assert!(inspection.part_file_exists);
    assert_eq!(inspection.part_file_size, Some(1000));
    assert_eq!(inspection.estimated_transferred_bytes, 1000);

    // 2. Probar import_part_met
    let result = service
        .import_part_met(&met_path, None, download_target_dir.path(), None)
        .unwrap();

    // Invariantes garantizados:
    // ¡El estado DEBE estar en pausa (paused)!
    assert_eq!(result.download.status, "paused");
    assert_eq!(result.total_parts, 1);
    assert_eq!(result.verified_parts, 1);
    assert_eq!(result.corrupted_parts, 0);
    assert_eq!(result.transferred_bytes, 1000);

    // Verificar que el archivo temporal exista en el directorio temporal interno
    let internal_part = temp_dir
        .path()
        .join("temp")
        .join(format!("{}.part", result.download.id));
    assert!(internal_part.exists());

    // Verificar que el archivo externo siga intacto
    assert!(part_path.exists());
    assert_eq!(std::fs::metadata(&part_path).unwrap().len(), 1000);

    // Verificar verified_parts en la BD
    let db = Database::open(&db_path).unwrap();
    let verified = db.get_verified_parts(&result.download.id).unwrap();
    assert_eq!(verified, vec![0]);

    // Verificar que el evento de historial "imported" haya sido registrado
    let history = service
        .query_history(&mule_core::HistoryFilter {
            limit: Some(10),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].event_type, "imported");
    assert_eq!(history[0].verification_result, "verified_ok");
}

#[test]
fn test_part_met_import_with_corrupted_part() {
    let temp_dir = tempdir().unwrap();
    let external_dir = tempdir().unwrap();
    let download_target_dir = tempdir().unwrap();

    let db_path = temp_dir.path().join("mule.db");
    let mut service = DownloadService::new(&db_path, temp_dir.path().join("temp")).unwrap();

    // Los datos de .part tienen bytes 0x99, pero .part.met tiene el hash para bytes 0x42
    let part_data = vec![0x99u8; 1000];
    let fake_expected_hash = [0x55u8; 16];

    let met_path = external_dir.path().join("002.part.met");
    let part_path = external_dir.path().join("002.part");

    std::fs::write(&part_path, &part_data).unwrap();

    create_part_met_file(
        &met_path,
        "corrupted.mkv",
        1000,
        1000,
        fake_expected_hash,
        &[fake_expected_hash],
        &[],
    );

    let result = service
        .import_part_met(&met_path, None, download_target_dir.path(), None)
        .unwrap();

    // ¡La parte corrupta fue detectada y expulsada de los rangos!
    assert_eq!(result.verified_parts, 0);
    assert_eq!(result.corrupted_parts, 1);
    assert_eq!(result.transferred_bytes, 0); // evicted from range set
    assert_eq!(result.download.status, "paused");
}

#[test]
fn test_part_met_import_oversized_physical_file_rejected() {
    let temp_dir = tempdir().unwrap();
    let external_dir = tempdir().unwrap();
    let download_target_dir = tempdir().unwrap();

    let db_path = temp_dir.path().join("mule.db");
    let mut service = DownloadService::new(&db_path, temp_dir.path().join("temp")).unwrap();

    let part_data = vec![0x11u8; 2000]; // 2000 bytes, but declared size is 1000
    let met_path = external_dir.path().join("003.part.met");
    let part_path = external_dir.path().join("003.part");

    std::fs::write(&part_path, &part_data).unwrap();

    create_part_met_file(
        &met_path,
        "oversized.mkv",
        1000, // declared: 1000 bytes
        1000,
        [0; 16],
        &[[0; 16]],
        &[],
    );

    let err = service
        .import_part_met(&met_path, None, download_target_dir.path(), None)
        .unwrap_err();

    assert!(err.to_string().contains("exceeds declared file size"));
}

#[test]
fn test_part_met_import_persists_across_daemon_restart() {
    let temp_dir = tempdir().unwrap();
    let external_dir = tempdir().unwrap();
    let download_target_dir = tempdir().unwrap();

    let db_path = temp_dir.path().join("mule.db");
    let temp_staging_dir = temp_dir.path().join("temp");

    let mut part_data = vec![0x42u8; 1000];
    let mkv_hdr = mule_network_policy::content_policy::MINIMAL_VALID_MKV_HEADER;
    part_data[..mkv_hdr.len()].copy_from_slice(mkv_hdr);
    let mut hasher = mule_format::ed2k::hash::Ed2kHasher::new();
    hasher.update(&part_data);
    let part_hash = hasher.finalize().unwrap().root_hash.0;

    let met_path = external_dir.path().join("004.part.met");
    let part_path = external_dir.path().join("004.part");
    std::fs::write(&part_path, &part_data).unwrap();

    create_part_met_file(
        &met_path,
        "reboot_preserved.mkv",
        1000,
        1000,
        part_hash,
        &[part_hash],
        &[],
    );

    let imported_id;
    {
        let mut service = DownloadService::new(&db_path, temp_staging_dir.clone()).unwrap();

        let result = service
            .import_part_met(
                &met_path,
                None,
                download_target_dir.path(),
                Some("cat-archives".to_string()),
            )
            .unwrap();

        assert_eq!(result.verified_parts, 1);
        assert_eq!(result.corrupted_parts, 0);
        assert_eq!(result.download.status, "paused");
        assert_eq!(result.download.category_id.as_deref(), Some("cat-archives"));

        let report = result
            .report
            .expect("Sanitized audit report must be present");
        assert!(report.contains("Format Version: 0xE0 (Standard eMule)"));
        assert!(report.contains("reboot_preserved.mkv"));
        assert!(report.contains("STRICTLY PAUSED"));
        assert!(report.contains("Verified Valid Parts: 1"));

        imported_id = result.download.id;
        // El servicio se descarta aquí, simulando el apagado del daemon
    }

    // Reiniciar el daemon recreando DownloadService sobre la misma base de datos persistente y directorio temporal
    {
        let fresh_service = DownloadService::new(&db_path, temp_staging_dir).unwrap();
        let downloads = fresh_service.list_downloads();
        let dl = downloads
            .iter()
            .find(|d| d.id == imported_id)
            .expect("Imported download must persist across daemon reboot");

        assert_eq!(dl.file_name, "reboot_preserved.mkv");
        assert_eq!(dl.status, "paused");
        assert_eq!(dl.category_id.as_deref(), Some("cat-archives"));
        assert_eq!(dl.received_bytes, 1000);
        assert_eq!(dl.source_count, 0);

        // Comprobar las partes verificadas en SQLite
        let db = Database::open(&db_path).unwrap();
        let verified = db.get_verified_parts(&imported_id).unwrap();
        assert_eq!(verified, vec![0]);

        // Comprobar la persistencia del historial
        let history = fresh_service
            .query_history(&mule_core::HistoryFilter {
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        let import_event = history
            .iter()
            .find(|h| h.download_id == imported_id.to_string())
            .expect("Import history entry must persist across daemon reboot");

        assert_eq!(import_event.event_type, "imported");
        assert_eq!(import_event.file_name, "reboot_preserved.mkv");
    }
}

#[test]
fn test_part_met_import_safety_limits_enforced() {
    let temp_dir = tempdir().unwrap();
    let external_dir = tempdir().unwrap();
    let download_target_dir = tempdir().unwrap();

    let db_path = temp_dir.path().join("mule.db");
    let mut service = DownloadService::new(&db_path, temp_dir.path().join("temp")).unwrap();

    let met_path = external_dir.path().join("005.part.met");
    let part_path = external_dir.path().join("005.part");
    std::fs::write(&part_path, b"dummy").unwrap();

    // Tamaño declarado de 2 TB (> límite de seguridad de 1 TB)
    create_part_met_file(
        &met_path,
        "too_large.mkv",
        2 * 1024 * 1024 * 1024 * 1024,
        0,
        [0; 16],
        &[],
        &[],
    );

    let err = service
        .import_part_met(&met_path, None, download_target_dir.path(), None)
        .unwrap_err();

    assert!(err.to_string().contains("maximum import safety limit"));
}
