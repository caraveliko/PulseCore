// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas para la recuperación al inicio de archivos `.part.met` huérfanos
//! (paridad con la carga automática de CPartFile en eMule), el marcado de
//! conservación de archivos (keep-files) y el volcado explícito de `.part.met`.

use md4::{Digest, Md4};
use mule_core::AllocationPolicy;
use mule_daemon::services::downloads::DownloadService;
use tempfile::tempdir;

fn make_orphan(temp_dir: &std::path::Path, stem: &str, content: &[u8], name: &str) -> [u8; 16] {
    let file_hash: [u8; 16] = Md4::digest(content).into();
    std::fs::write(temp_dir.join(format!("{stem}.part")), content).unwrap();
    let met = mule_format::PartMetFile {
        version: mule_format::PARTFILE_VERSION,
        mod_date: 0,
        file_hash: mule_format::Ed2kHash(file_hash),
        part_hashes: vec![file_hash],
        file_name: Some(name.to_string()),
        file_size: Some(content.len() as u64),
        transferred: content.len() as u64,
        aich_hash: None,
        gaps: vec![],
        tags: vec![],
    };
    met.save(temp_dir.join(format!("{stem}.part.met"))).unwrap();
    file_hash
}

#[test]
fn orphan_part_met_is_recovered_and_adopted_in_place() {
    let tmp = tempdir().unwrap();
    let temp_dir = tmp.path().join("temp");
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut svc = DownloadService::in_memory(temp_dir.clone()).unwrap();

    let mut content = vec![0xABu8; 500];
    let mkv_hdr = mule_network_policy::content_policy::MINIMAL_VALID_MKV_HEADER;
    content[..mkv_hdr.len()].copy_from_slice(mkv_hdr);
    let file_hash = make_orphan(&temp_dir, "003", &content, "orphan.mkv");

    let recovered = svc.recover_orphan_part_mets(tmp.path());
    assert_eq!(recovered, 1);

    // Adopción: el par de origen se consume, el {uuid}.part preparado existe.
    assert!(!temp_dir.join("003.part.met").exists());
    assert!(!temp_dir.join("003.part").exists());

    let snaps = svc.list_downloads();
    assert_eq!(snaps.len(), 1);
    let snap = &snaps[0];
    assert_eq!(snap.status, "paused");
    assert_eq!(snap.file_name, "orphan.mkv");
    assert_eq!(snap.received_bytes, 500);
    assert_eq!(snap.ed2k_hash, hex::encode(file_hash));
    assert_eq!(snap.source_count, 0);

    let staged = temp_dir.join(format!("{}.part", snap.id));
    assert!(staged.exists());
    assert_eq!(std::fs::metadata(&staged).unwrap().len(), 500);

    // Idempotente: no queda nada por recuperar.
    assert_eq!(svc.recover_orphan_part_mets(tmp.path()), 0);
}

#[test]
fn tracked_part_met_is_not_reimported() {
    let tmp = tempdir().unwrap();
    let temp_dir = tmp.path().join("temp");
    let mut svc = DownloadService::in_memory(temp_dir.clone()).unwrap();

    let link = format!("ed2k://|file|known.mkv|500|{}|/", hex::encode([0x77u8; 16]));
    svc.add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    // El {uuid}.part.met escrito por add es propiedad de una descarga rastreada:
    // el escaneo de huérfanos debe omitirlo.
    assert_eq!(svc.recover_orphan_part_mets(tmp.path()), 0);
    assert_eq!(svc.list_downloads().len(), 1);
}

#[test]
fn malformed_orphan_met_is_skipped_without_panicking() {
    let tmp = tempdir().unwrap();
    let temp_dir = tmp.path().join("temp");
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut svc = DownloadService::in_memory(temp_dir.clone()).unwrap();

    std::fs::write(temp_dir.join("007.part.met"), b"not a met file").unwrap();
    std::fs::write(temp_dir.join("007.part"), b"junk").unwrap();

    assert_eq!(svc.recover_orphan_part_mets(tmp.path()), 0);
    assert!(svc.list_downloads().is_empty());
    // La entrada malformada se deja en disco para inspección manual.
    assert!(temp_dir.join("007.part.met").exists());
}

#[test]
fn removed_download_with_kept_files_does_not_resurrect() {
    let tmp = tempdir().unwrap();
    let temp_dir = tmp.path().join("temp");
    let mut svc = DownloadService::in_memory(temp_dir.clone()).unwrap();

    let mut content = vec![0x55u8; 400];
    let mkv_hdr = mule_network_policy::content_policy::MINIMAL_VALID_MKV_HEADER;
    content[..mkv_hdr.len()].copy_from_slice(mkv_hdr);
    let file_hash: [u8; 16] = Md4::digest(&content).into();
    let link = format!("ed2k://|file|keep.mkv|400|{}|/", hex::encode(file_hash));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &content, None).unwrap();

    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let mut met_name = temp_path.as_os_str().to_os_string();
    met_name.push(".met");
    let met_path = std::path::PathBuf::from(&met_name);
    let mut keep_name = met_name.clone();
    keep_name.push(".keep");
    let keep_path = std::path::PathBuf::from(keep_name);
    assert!(met_path.exists());

    svc.remove_download(&snap.id, false).unwrap();

    // Datos preservados: .part permanece, el met pasa a lápida con .keep (ignorado por el escaneo).
    assert!(temp_path.exists());
    assert!(!met_path.exists());
    assert!(keep_path.exists());
    assert!(svc.list_downloads().is_empty());

    // El par conservado no debe resucitar en el siguiente escaneo de inicio.
    assert_eq!(svc.recover_orphan_part_mets(tmp.path()), 0);
    assert!(svc.list_downloads().is_empty());
}

#[test]
fn save_active_part_mets_flushes_all_in_progress() {
    let tmp = tempdir().unwrap();
    let temp_dir = tmp.path().join("temp");
    let mut svc = DownloadService::in_memory(temp_dir).unwrap();

    let link = format!("ed2k://|file|flush.mkv|300|{}|/", hex::encode([0x11u8; 16]));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &[0xAAu8; 100], None).unwrap();

    // Simular un met sucio: corromper el acompañante en disco y luego vaciar (flush).
    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let mut met_name = temp_path.as_os_str().to_os_string();
    met_name.push(".met");
    let met_path = std::path::PathBuf::from(met_name);
    std::fs::write(&met_path, b"stale").unwrap();

    svc.save_active_part_mets();

    let met = mule_format::PartMetFile::open(&met_path).unwrap();
    assert_eq!(met.gaps, vec![(100, 300)]);
    assert_eq!(met.transferred, 100);
}
