// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Suite de integración sin conexión para la Política de Contenido «Solo Vídeo» (Opción E1 / ADR 0006).
//! Verifica las cuatro puertas defensivas: admisión, inspección en vuelo (MD4 parte 0),
//! prefinalización en disco, renombrado/compartidos, búsqueda RPC y Transmission RPC.

use md4::{Digest, Md4};
use mule_core::{AllocationPolicy, HistoryFilter};
use mule_daemon::rpc::methods::handle_request;
use mule_daemon::services::downloads::DownloadService;
use mule_daemon::{AppState, DaemonConfig, WorkDirMode};
use mule_format::ed2k::ED2K_PART_SIZE;
use mule_format::{met::tags::*, PARTFILE_VERSION};
use mule_network_policy::content_policy::MINIMAL_VALID_MKV_HEADER;
use mule_test_support::{make_req, spawn_test_daemon_with_mode};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

/// Construye un archivo `.part.met` mínimo en disco para pruebas sin conexión.
fn write_part_met_fixture(
    path: &std::path::Path,
    filename: &str,
    file_size: u64,
    transferred: u64,
    file_hash: [u8; 16],
    part_hashes: &[[u8; 16]],
) {
    let mut bytes = Vec::new();
    bytes.push(PARTFILE_VERSION);
    bytes.extend_from_slice(&1_700_000_000u32.to_le_bytes());
    bytes.extend_from_slice(&file_hash);
    bytes.extend_from_slice(&(part_hashes.len() as u16).to_le_bytes());
    for h in part_hashes {
        bytes.extend_from_slice(h);
    }

    let tag_count = 3u32;
    bytes.extend_from_slice(&tag_count.to_le_bytes());

    bytes.push(0x82);
    bytes.push(FT_FILENAME);
    let fn_bytes = filename.as_bytes();
    bytes.extend_from_slice(&(fn_bytes.len() as u16).to_le_bytes());
    bytes.extend_from_slice(fn_bytes);

    bytes.push(0x83);
    bytes.push(FT_FILESIZE);
    bytes.extend_from_slice(&(file_size as u32).to_le_bytes());

    bytes.push(0x83);
    bytes.push(FT_TRANSFERRED);
    bytes.extend_from_slice(&(transferred as u32).to_le_bytes());

    let mut file = File::create(path).unwrap();
    file.write_all(&bytes).unwrap();
}

/// Construye los bytes bencode de un torrent mínimo de un solo archivo y los codifica en base64.
fn single_file_torrent_base64(file_name: &str, length: usize) -> String {
    let mut buf = Vec::new();
    let header = format!(
        "d8:announce25:http://127.0.0.1/announce4:infod6:lengthi{length}e4:name{}:{file_name}12:piece lengthi16384e6:pieces20:",
        file_name.len()
    );
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(&[0x41u8; 20]);
    buf.extend_from_slice(b"ee");
    data_encoding::BASE64.encode(&buf)
}

#[test]
fn admission_ed2k_link_rejects_non_video_and_double_extensions() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    for bad_name in [
        "instalador.exe",
        "paquete.zip",
        "disco.iso",
        "cancion.mp3",
        "manual.pdf",
        "archivo_sin_extension",
        "pelicula.exe.mkv",
        "temporada1.zip.mp4",
    ] {
        let link = format!("ed2k://|file|{bad_name}|1048576|0123456789abcdef0123456789abcdef|/");
        let err = svc
            .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
            .expect_err("el enlace no multimedia debe rechazarse en admisión");
        assert!(
            !err.to_string().is_empty(),
            "debe existir motivo legible para '{bad_name}'"
        );
    }

    for (idx, good_name) in [
        "pelicula.mkv",
        "serie.s01e01.mp4",
        "clasico.avi",
        "captura.ts",
        "trailer.webm",
    ]
    .into_iter()
    .enumerate()
    {
        let hash = hex::encode([(idx as u8) + 0x10; 16]);
        let link = format!("ed2k://|file|{good_name}|1048576|{hash}|/");
        svc.add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
            .expect("los contenedores de vídeo permitidos deben admitirse");
    }
}

#[test]
fn part_met_import_and_orphan_recovery_enforce_filename_and_header() {
    let tmp = tempdir().unwrap();
    let external_dir = tempdir().unwrap();
    let orphan_dir = tmp.path().join("temp_orphan");
    std::fs::create_dir_all(&orphan_dir).unwrap();

    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    // 1. `.part.met` con nombre no vídeo -> rechazado antes de copiar/adoptar.
    let mut valid_payload = vec![0x42u8; 512];
    valid_payload[..MINIMAL_VALID_MKV_HEADER.len()].copy_from_slice(MINIMAL_VALID_MKV_HEADER);
    let valid_hash: [u8; 16] = Md4::digest(&valid_payload).into();

    let bad_met = external_dir.path().join("001.part.met");
    let bad_part = external_dir.path().join("001.part");
    std::fs::write(&bad_part, &valid_payload).unwrap();
    write_part_met_fixture(&bad_met, "instalador.exe", 512, 512, valid_hash, &[valid_hash]);
    assert!(
        svc.import_part_met(&bad_met, None, tmp.path(), None).is_err(),
        "import_part_met debe rechazar un nombre no vídeo"
    );

    // 2. `.part.met` con nombre `.mkv` pero parte 0 verificada con cabecera PE/MZ -> rechazado y `.part` interno limpiado.
    let mut mz_payload = vec![0x90u8; 512];
    mz_payload[..2].copy_from_slice(b"MZ");
    let mz_hash: [u8; 16] = Md4::digest(&mz_payload).into();

    let spoof_met = external_dir.path().join("002.part.met");
    let spoof_part = external_dir.path().join("002.part");
    std::fs::write(&spoof_part, &mz_payload).unwrap();
    write_part_met_fixture(&spoof_met, "falso_video.mkv", 512, 512, mz_hash, &[mz_hash]);
    assert!(
        svc.import_part_met(&spoof_met, None, tmp.path(), None)
            .is_err(),
        "import_part_met debe rechazar una parte 0 verificada con cabecera MZ"
    );
    let leftover_internal: Vec<_> = std::fs::read_dir(tmp.path().join("temp"))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        leftover_internal.is_empty(),
        "el .part temporal interno debe eliminarse cuando falla la inspección de cabecera"
    );

    // 3. Recuperación de huérfanos con archivo no vídeo y con falso `.mkv` MZ -> ninguno adoptado.
    let mut orphan_svc = DownloadService::in_memory(orphan_dir.clone()).unwrap();
    let orphan_exe_part = orphan_dir.join("003.part");
    let orphan_exe_met = orphan_dir.join("003.part.met");
    std::fs::write(&orphan_exe_part, &valid_payload).unwrap();
    write_part_met_fixture(&orphan_exe_met, "virus.exe", 512, 512, valid_hash, &[valid_hash]);

    let orphan_spoof_part = orphan_dir.join("004.part");
    let orphan_spoof_met = orphan_dir.join("004.part.met");
    std::fs::write(&orphan_spoof_part, &mz_payload).unwrap();
    write_part_met_fixture(
        &orphan_spoof_met,
        "fake_movie.mkv",
        512,
        512,
        mz_hash,
        &[mz_hash],
    );

    let recovered = orphan_svc.recover_orphan_part_mets(tmp.path());
    assert_eq!(recovered, 0);
    assert!(orphan_svc.list_downloads().is_empty());
}

#[test]
fn in_flight_part0_inspection_rejects_verified_fake_mkv_without_harming_corrupt_peer_case() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("mule.db");
    let temp_dir = tmp.path().join("temp");
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::new(&db_path, temp_dir).unwrap();

    // A) Carga maliciosa `MZ` cuyo MD4 coincide exactamente con la parte 0 declarada:
    // debe marcar la descarga como failed, borrar .part/.part.met, registrar en cancelled_files y en historial.
    let mut mz_payload = vec![0x90u8; 512];
    mz_payload[..2].copy_from_slice(b"MZ");
    let mz_hash: [u8; 16] = Md4::digest(&mz_payload).into();
    let mz_hex = hex::encode(mz_hash);
    let fake_link = format!("ed2k://|file|pelicula_falsa.mkv|512|{mz_hex}|/");
    let fake_snap = svc
        .add_ed2k_link(&fake_link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.store_part_hashes_by_hash(&mz_hex, &[mz_hash]).unwrap();

    let (fake_part_path, _, _, _) = svc.get_download_file_info(&fake_snap.id).unwrap();
    let mut fake_met_name = fake_part_path.as_os_str().to_os_string();
    fake_met_name.push(".met");
    let fake_met_path = std::path::PathBuf::from(fake_met_name);

    let after_snap = svc
        .inject_range(&fake_snap.id, 0, &mz_payload, Some("127.0.0.1:4662"))
        .unwrap();
    assert_eq!(after_snap.status.to_lowercase(), "failed");
    assert!(!fake_part_path.exists(), "el .part falso debe eliminarse");
    assert!(!fake_met_path.exists(), "el .part.met falso debe eliminarse");
    assert!(
        svc.is_cancelled_file(&mz_hex).unwrap(),
        "el hash rechazado por contenido debe registrarse en cancelled_files"
    );

    let history = svc
        .query_history(&HistoryFilter {
            limit: Some(10),
            ..Default::default()
        })
        .unwrap();
    assert!(
        history
            .iter()
            .any(|h| h.download_id == fake_snap.id.to_string()
                && h.event_type == "failed"
                && h.verification_result == "content_policy_rejected"),
        "debe quedar constancia en historial con content_policy_rejected: {history:?}"
    );

    // B) Par corrupto enviando `MZ` cuando el MD4 esperado corresponde a un MKV legítimo:
    // NO debe disparar la política de contenido ni añadir el hash a cancelled_files.
    let mut real_mkv = vec![0x44u8; 512];
    real_mkv[..MINIMAL_VALID_MKV_HEADER.len()].copy_from_slice(MINIMAL_VALID_MKV_HEADER);
    let real_hash: [u8; 16] = Md4::digest(&real_mkv).into();
    let real_hex = hex::encode(real_hash);
    let real_link = format!("ed2k://|file|pelicula_real.mkv|512|{real_hex}|/");
    let real_snap = svc
        .add_ed2k_link(&real_link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.store_part_hashes_by_hash(&real_hex, &[real_hash])
        .unwrap();

    let corrupt_snap = svc
        .inject_range(&real_snap.id, 0, &mz_payload, Some("127.0.0.2:4662"))
        .unwrap();
    assert_ne!(corrupt_snap.status.to_lowercase(), "failed");
    assert!(!svc.is_cancelled_file(&real_hex).unwrap());
    assert_eq!(svc.get_covered_len(&real_snap.id), Some(0));

    // Tras expulsar el bloque corrupto, un par honesto puede completar el vídeo legítimo.
    let verified_snap = svc
        .inject_range(&real_snap.id, 0, &real_mkv, Some("127.0.0.3:4662"))
        .unwrap();
    assert_eq!(verified_snap.status.to_lowercase(), "verifying");
    let done = svc.finalize_download(&real_snap.id).unwrap();
    assert_eq!(done.status.to_lowercase(), "completed");
}

#[test]
fn prepare_finalize_rejects_invalid_header_before_moving_to_incoming() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("mule.db");
    let temp_dir = tmp.path().join("temp");
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::new(&db_path, temp_dir).unwrap();

    // Tamaño exacto de 1 parte (9_728_000 bytes) sin hashset: no verifica parte 0 por MD4 en vuelo,
    // por lo que la puerta de prefinalización (`prepare_finalize`) debe interceptarlo en disco.
    let mut zip_payload = vec![0x00u8; ED2K_PART_SIZE];
    zip_payload[..4].copy_from_slice(b"PK\x03\x04");
    let file_hash = [0x99u8; 16];
    let hash_hex = hex::encode(file_hash);
    let link = format!(
        "ed2k://|file|sin_hashset.mkv|{}|{hash_hex}|/",
        ED2K_PART_SIZE
    );
    let snap = svc
        .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
        .unwrap();

    let (part_path, dest_path, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    svc.inject_range(&snap.id, 0, &zip_payload, None).unwrap();

    let err = svc
        .finalize_download(&snap.id)
        .expect_err("finalize_download debe bloquear un archivo ZIP disfrazado de .mkv");
    assert!(
        err.to_string().contains("cabecera") || err.to_string().contains("ZIP"),
        "error inesperado en prefinalización: {err}"
    );
    assert!(!dest_path.exists(), "no debe moverse al directorio destino");
    assert!(!part_path.exists(), "el archivo temporal debe haberse borrado");
    assert_eq!(
        svc.get_download(&snap.id).unwrap().status.to_lowercase(),
        "failed"
    );
    assert!(svc.is_cancelled_file(&hash_hex).unwrap());
}

#[tokio::test]
async fn renames_shares_and_search_enforce_video_only_policy() {
    let dir = tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    std::fs::write(download_dir.join("payload.exe"), b"MZ").unwrap();
    std::fs::write(download_dir.join("archive.zip"), b"PK").unwrap();
    std::fs::write(download_dir.join("audio.mp3"), b"ID3").unwrap();
    std::fs::write(download_dir.join("notas.txt"), b"texto").unwrap();
    std::fs::write(
        download_dir.join("pelicula_buena.mkv"),
        MINIMAL_VALID_MKV_HEADER,
    )
    .unwrap();

    let token = "content_policy_rpc_token".to_string();
    let config = DaemonConfig::new(dir.path().to_path_buf(), &token)
        .with_workdir_mode(WorkDirMode::Portable)
        .with_default_download_dir(download_dir.clone());
    let mut svc = DownloadService::in_memory(dir.path().join("temp")).unwrap();

    // Pruebas directas de renombrado en DownloadService.
    let dl_hash = hex::encode([0x51u8; 16]);
    let dl_snap = svc
        .add_ed2k_link(
            &format!("ed2k://|file|original.mkv|2048|{dl_hash}|/"),
            &download_dir,
            AllocationPolicy::None,
            None,
        )
        .unwrap();
    assert!(svc.rename_download(&dl_snap.id, "malicioso.exe").is_err());
    assert!(svc.rename_download(&dl_snap.id, "doble.exe.mkv").is_err());
    assert_eq!(
        svc.rename_download(&dl_snap.id, "renombrado.mp4")
            .unwrap()
            .file_name,
        "renombrado.mp4"
    );

    let state = AppState::new(config, svc);
    let mut authenticated = true;

    // 1. `shares.rescan`: únicamente `pelicula_buena.mkv` se comparte.
    let rescan = handle_request(
        &state,
        make_req(1, "shares.rescan", None),
        &mut authenticated,
    )
    .await
    .unwrap();
    assert!(rescan.error.is_none());
    assert_eq!(rescan.result.as_ref().unwrap()["added"], 1);

    let shares_list = handle_request(&state, make_req(2, "shares.list", None), &mut authenticated)
        .await
        .unwrap();
    let shared_files = shares_list.result.as_ref().unwrap()["sharedFiles"]
        .as_array()
        .unwrap();
    let good_share = shared_files
        .iter()
        .find(|s| s["fileName"] == "pelicula_buena.mkv")
        .expect("pelicula_buena.mkv debe haberse compartido");
    let good_hash = good_share["fileHash"].as_str().unwrap();

    // `rename_shared_file` rechaza extensiones prohibidas.
    assert!(state
        .downloads
        .lock()
        .await
        .rename_shared_file(good_hash, "troyano.zip")
        .is_err());

    // 2. `shares.add`: rechaza no vídeo y dobles extensiones.
    for bad in ["documento.pdf", "script.ps1.mkv"] {
        let resp = handle_request(
            &state,
            make_req(
                3,
                "shares.add",
                Some(json!({
                    "fileHash": "0123456789abcdef0123456789abcdef",
                    "fileName": bad,
                    "fileSize": 1024,
                    "filePath": download_dir.join(bad).to_string_lossy()
                })),
            ),
            &mut authenticated,
        )
        .await
        .unwrap();
        assert!(
            resp.error.is_some(),
            "shares.add debe rechazar '{bad}'"
        );
    }

    // 3. `servers.search`: rechaza extensiones no vídeo y admite `mkv`.
    for bad_ext in ["exe", "zip"] {
        let resp = handle_request(
            &state,
            make_req(
                4,
                "servers.search",
                Some(json!({ "term": "linux", "extension": bad_ext })),
            ),
            &mut authenticated,
        )
        .await
        .unwrap();
        let err_msg = resp
            .error
            .as_ref()
            .expect("servers.search debe rechazar extensiones no vídeo")
            .message
            .to_string();
        assert!(
            err_msg.contains("no permitida"),
            "mensaje inesperado para '{bad_ext}': {err_msg}"
        );
    }

    let ok_ext_resp = handle_request(
        &state,
        make_req(
            5,
            "servers.search",
            Some(json!({ "term": "documental", "extension": "mkv" })),
        ),
        &mut authenticated,
    )
    .await
    .unwrap();
    if let Some(err) = ok_ext_resp.error {
        assert!(
            !err.message.contains("no permitida"),
            "la extensión 'mkv' no debe rechazarse por política de contenido: {:?}",
            err
        );
    }
}

#[tokio::test]
async fn transmission_rpc_enforces_video_only_for_ed2k_and_torrents() {
    let (addr, _token, _dir) =
        spawn_test_daemon_with_mode("cp_transmission_token", WorkDirMode::Portable).await;
    let rpc_url = format!("http://{addr}/transmission/rpc");
    let client = reqwest::Client::new();

    let res = client
        .post(&rpc_url)
        .header("content-type", "application/json")
        .body(json!({ "method": "session-get" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let session_header = res
        .headers()
        .get("x-transmission-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // 1. Enlace eD2K con `.exe` -> rechazado.
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-add",
                "arguments": {
                    "filename": "ed2k://|file|malware.exe|1024|0123456789abcdef0123456789abcdef|/"
                },
                "tag": 1
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    let val: Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    assert_ne!(val["result"], "success");

    // 2. Torrent base64 con único fichero `setup.exe` -> rechazado.
    let bad_b64 = single_file_torrent_base64("setup.exe", 2048);
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-add",
                "arguments": { "metainfo": bad_b64 },
                "tag": 2
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    let val: Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    assert_ne!(val["result"], "success");

    // 3. Torrent base64 con `pelicula.mkv` -> aceptado.
    let good_b64 = single_file_torrent_base64("pelicula.mkv", 2048);
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-add",
                "arguments": { "metainfo": good_b64 },
                "tag": 3
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    let val: Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    assert_eq!(val["result"], "success", "respuesta inesperada: {val}");
    assert_eq!(val["arguments"]["torrent-added"]["name"], "pelicula.mkv");
}
