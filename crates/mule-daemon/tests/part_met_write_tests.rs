// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas para la emisión de archivos complementarios `.part.met` (paridad con SavePartFile de eMule).

use md4::{Digest, Md4};
use mule_core::AllocationPolicy;
use mule_daemon::services::downloads::DownloadService;
use mule_daemon::{AppState, DaemonConfig};
use tempfile::tempdir;

#[test]
fn part_met_is_written_on_add_and_updated_on_progress() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let content = vec![0x55u8; 1000];
    let file_hash: [u8; 16] = Md4::digest(&content).into();
    let link = format!("ed2k://|file|sample.bin|1000|{}|/", hex::encode(file_hash));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let mut met_name = temp_path.as_os_str().to_os_string();
    met_name.push(".met");
    let met_path = std::path::PathBuf::from(met_name);

    assert!(met_path.exists(), ".part.met must be created on add");

    let met = mule_format::PartMetFile::open(&met_path).unwrap();
    assert_eq!(met.file_hash.0, file_hash);
    assert_eq!(met.file_name.as_deref(), Some("sample.bin"));
    assert_eq!(met.file_size, Some(1000));
    assert_eq!(met.gaps, vec![(0, 1000)]);
    assert_eq!(
        met.compute_completed_ranges().unwrap(),
        Vec::<(u64, u64)>::new()
    );

    // Tras inyectar datos, el met refleja los rangos cubiertos (sin intervalos).
    svc.inject_range(&snap.id, 0, &content, None).unwrap();
    let met = mule_format::PartMetFile::open(&met_path).unwrap();
    assert_eq!(met.transferred, 1000);
    assert!(met.gaps.is_empty());
    assert_eq!(
        met.compute_completed_ranges().unwrap(),
        vec![(0u64, 1000u64)]
    );
}

#[test]
fn part_met_is_deleted_on_remove_with_files() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!("ed2k://|file|gone.bin|500|{}|/", hex::encode([0x77u8; 16]));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let mut met_name = temp_path.as_os_str().to_os_string();
    met_name.push(".met");
    let met_path = std::path::PathBuf::from(met_name);
    assert!(met_path.exists());

    svc.remove_download(&snap.id, true).unwrap();
    assert!(
        !met_path.exists(),
        ".part.met must be removed with delete_files"
    );
    assert!(!temp_path.exists());
}

#[test]
fn completed_file_is_deleted_and_unshared_on_remove_with_files() {
    let tmp = tempdir().unwrap();
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    // El hash del enlace debe corresponder al contenido inyectado (archivo
    // de una sola parte: verificado al completarse).
    let content = vec![0x42u8; 5000];
    let mut hasher = mule_format::ed2k::Ed2kHasher::new();
    hasher.update(&content);
    let file_hash = hasher.finalize().unwrap().root_hash.to_hex();
    let link = format!("ed2k://|file|done.bin|5000|{file_hash}|/");
    let snap = svc
        .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &content, None).unwrap();
    svc.finalize_download(&snap.id).unwrap();

    let target_path = target_dir.join("done.bin");
    assert!(target_path.exists(), "finalized file must exist");
    assert!(
        !svc.list_shared_files().unwrap().is_empty(),
        "finalized file must be registered as shared"
    );

    svc.remove_download(&snap.id, true).unwrap();
    assert!(
        !target_path.exists(),
        "completed file must be deleted with delete_files"
    );
    assert!(
        svc.list_shared_files().unwrap().is_empty(),
        "shared entry must be removed with the file"
    );
}

#[test]
fn completed_file_is_kept_on_remove_without_files() {
    let tmp = tempdir().unwrap();
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let content = vec![0x24u8; 5000];
    let mut hasher = mule_format::ed2k::Ed2kHasher::new();
    hasher.update(&content);
    let file_hash = hasher.finalize().unwrap().root_hash.to_hex();
    let link = format!("ed2k://|file|kept.bin|5000|{file_hash}|/");
    let snap = svc
        .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &content, None).unwrap();
    svc.finalize_download(&snap.id).unwrap();

    let target_path = target_dir.join("kept.bin");
    svc.remove_download(&snap.id, false).unwrap();
    assert!(
        target_path.exists(),
        "completed file must be kept without delete_files"
    );
    assert!(
        !svc.list_shared_files().unwrap().is_empty(),
        "kept file stays shared"
    );
}

#[test]
fn session_downloaded_bytes_counts_only_bytes_received_this_session() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!("ed2k://|file|sess.bin|1000|{}|/", hex::encode([0x11u8; 16]));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    assert_eq!(svc.session_downloaded_bytes(), 0);
    svc.inject_range(&snap.id, 0, &[0xAAu8; 400], None).unwrap();
    assert_eq!(svc.session_downloaded_bytes(), 400);
    svc.inject_range(&snap.id, 400, &[0xBBu8; 200], None).unwrap();
    assert_eq!(svc.session_downloaded_bytes(), 600);
}

#[test]
fn shared_file_delete_removes_disk_file_and_entry() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    // Archivo compartido «fuera de descargas» (nuevo escaneo de Incoming / shares.add).
    let dir = tmp.path().join("incoming");
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join("loose.bin");
    std::fs::write(&file_path, b"data").unwrap();
    let hash = "aabbccddeeff00112233445566778899";
    svc.add_shared_file(&mule_core::model::SharedFile::new(
        hash,
        "loose.bin",
        4,
        file_path.to_string_lossy().as_ref(),
    ))
    .unwrap();

    assert!(svc.remove_shared_file(hash, true).unwrap());
    assert!(!file_path.exists(), "delete_file must remove the disk file");
    assert!(svc.get_shared_file(hash).unwrap().is_none());
}

#[test]
fn shared_file_delete_keeps_disk_file_without_flag() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let dir = tmp.path().join("incoming");
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join("kept_share.bin");
    std::fs::write(&file_path, b"data").unwrap();
    let hash = "11223344556677889900aabbccddeeff";
    svc.add_shared_file(&mule_core::model::SharedFile::new(
        hash,
        "kept_share.bin",
        4,
        file_path.to_string_lossy().as_ref(),
    ))
    .unwrap();

    assert!(svc.remove_shared_file(hash, false).unwrap());
    assert!(file_path.exists(), "unshare must keep the disk file");
    assert!(svc.get_shared_file(hash).unwrap().is_none());
}

#[test]
fn shared_file_delete_rejects_partial_share() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!(
        "ed2k://|file|partial.bin|500|{}|/",
        hex::encode([0x77u8; 16])
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    let hash = snap.ed2k_hash.clone();

    // El compartido parcial se resuelve a partir de la descarga activa.
    let sf = svc.get_shared_file(&hash).unwrap().unwrap();
    assert!(sf.is_partial);

    assert!(svc.remove_shared_file(&hash, true).is_err());
    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    assert!(temp_path.exists(), "partial .part file must be untouched");
    assert!(svc.get_download(&snap.id).is_some());
}

#[test]
fn shared_file_delete_completed_routes_through_remove_download() {
    let tmp = tempdir().unwrap();
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let content = vec![0x5Au8; 5000];
    let mut hasher = mule_format::ed2k::Ed2kHasher::new();
    hasher.update(&content);
    let file_hash = hasher.finalize().unwrap().root_hash.to_hex();
    let link = format!("ed2k://|file|shared_done.bin|5000|{file_hash}|/");
    let snap = svc
        .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &content, None).unwrap();
    svc.finalize_download(&snap.id).unwrap();

    let target_path = target_dir.join("shared_done.bin");
    assert!(target_path.exists());

    // La eliminación a través del compartido retira también la entrada de descarga
    // completada (de lo contrario volvería a apuntar a un archivo inexistente).
    assert!(svc.remove_shared_file(&file_hash, true).unwrap());
    assert!(!target_path.exists());
    assert!(svc.get_shared_file(&file_hash).unwrap().is_none());
    assert!(svc.get_download(&snap.id).is_none());
}

/// `downloads.rename` en una descarga en curso: actualiza el nombre
/// mostrado y la ruta de destino sin tocar el `.part` (nombrado por UUID).
#[test]
fn rename_in_progress_updates_name_and_target_not_part_file() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!(
        "ed2k://|file|old_name.bin|500|{}|/",
        hex::encode([0x44u8; 16])
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let part_path = temp_path.clone();

    let renamed = svc.rename_download(&snap.id, "new_name.bin").unwrap();
    assert_eq!(renamed.file_name, "new_name.bin");
    assert_eq!(
        svc.get_download_file_info(&snap.id).unwrap().1,
        tmp.path().join("new_name.bin")
    );
    // El .part por UUID no se mueve.
    assert!(part_path.exists());
    assert_eq!(
        svc.get_download(&snap.id).unwrap().file_name,
        "new_name.bin"
    );
}

/// Descarga terminada: el renombramiento mueve el archivo en disco y
/// actualiza la entrada `shared_files` (nombre + ruta).
#[test]
fn rename_completed_renames_disk_file_and_share_entry() {
    let tmp = tempdir().unwrap();
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let content = vec![0x66u8; 500];
    let file_hash: [u8; 16] = Md4::digest(&content).into();
    let hash_hex = hex::encode(file_hash);
    let link = format!("ed2k://|file|before.iso|500|{hash_hex}|/");
    let snap = svc
        .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &content, None).unwrap();
    svc.finalize_download(&snap.id).unwrap();

    let old_path = target_dir.join("before.iso");
    let new_path = target_dir.join("after.iso");
    assert!(old_path.exists());

    svc.rename_download(&snap.id, "after.iso").unwrap();
    assert!(!old_path.exists());
    assert!(new_path.exists());

    let shared = svc.get_shared_file(&hash_hex).unwrap().unwrap();
    assert_eq!(shared.file_name, "after.iso");
    assert_eq!(shared.file_path, new_path.to_string_lossy());
}

/// `shares.rename` delega en `rename_download` cuando el hash respalda una
/// descarga completada — ambas vistas se mantienen coherentes.
#[test]
fn rename_shared_file_by_hash_delegates_to_completed_download() {
    let tmp = tempdir().unwrap();
    let target_dir = tmp.path().join("incoming");
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let content = vec![0x77u8; 500];
    let file_hash: [u8; 16] = Md4::digest(&content).into();
    let hash_hex = hex::encode(file_hash);
    let link = format!("ed2k://|file|dl.bin|500|{hash_hex}|/");
    let snap = svc
        .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
        .unwrap();
    svc.inject_range(&snap.id, 0, &content, None).unwrap();
    svc.finalize_download(&snap.id).unwrap();

    svc.rename_shared_file(&hash_hex, "dl_renamed.bin").unwrap();
    assert!(target_dir.join("dl_renamed.bin").exists());
    assert_eq!(
        svc.get_download(&snap.id).unwrap().file_name,
        "dl_renamed.bin"
    );
    assert_eq!(
        svc.get_shared_file(&hash_hex).unwrap().unwrap().file_name,
        "dl_renamed.bin"
    );
}

/// Los nombres no válidos (vacíos, separadores, caracteres prohibidos en Windows)
/// se rechazan sin tocar la descarga.
#[test]
fn rename_rejects_invalid_file_names() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!("ed2k://|file|keep.bin|500|{}|/", hex::encode([0x55u8; 16]));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    for bad in [
        "", "   ", "a/b.bin", "a\\b.bin", "x*.bin", "nul?.bin", "..", "name.",
    ] {
        assert!(
            svc.rename_download(&snap.id, bad).is_err(),
            "name {bad:?} must be rejected"
        );
    }
    assert_eq!(svc.get_download(&snap.id).unwrap().file_name, "keep.bin");
    assert!(svc.rename_shared_file("deadbeef", "a|b.bin").is_err());
}

#[test]
fn part_met_persists_partial_gaps() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!(
        "ed2k://|file|sparse.bin|300|{}|/",
        hex::encode([0x33u8; 16])
    );
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();

    // Los datos parciales no activan la finalización de parte; forzar un guardado mediante pausa.
    svc.inject_range(&snap.id, 0, &[0xAAu8; 100], None).unwrap();
    svc.pause_download(&snap.id).unwrap();

    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let mut met_name = temp_path.as_os_str().to_os_string();
    met_name.push(".met");
    let met = mule_format::PartMetFile::open(std::path::PathBuf::from(met_name)).unwrap();
    assert_eq!(met.gaps, vec![(100, 300)]);
    assert_eq!(
        met.compute_completed_ranges().unwrap(),
        vec![(0u64, 100u64)]
    );
    assert_eq!(met.transferred, 100);
}

/// `stats.resetLifetimeTotals` restablece a cero los contadores acumulados en
/// memoria Y persistidos (ajustes en SQLite); los valores recargados en
/// el arranque parten correctamente desde cero.
#[tokio::test]
async fn reset_lifetime_totals_zeroes_memory_and_persisted_settings() {
    use std::sync::atomic::Ordering;

    let tmp = tempdir().unwrap();
    let mut downloads = DownloadService::in_memory(tmp.path().join("temp")).unwrap();
    downloads
        .set_setting("lifetime_downloaded_bytes", "12345")
        .unwrap();
    downloads
        .set_setting("lifetime_uploaded_bytes", "6789")
        .unwrap();

    let config = DaemonConfig::new(tmp.path().to_path_buf(), "t");
    let state = AppState::new(config, downloads);

    // Los contadores se hidratan a partir de los ajustes persistidos.
    assert_eq!(
        state.lifetime_downloaded_bytes.load(Ordering::SeqCst),
        12345
    );
    assert_eq!(state.lifetime_uploaded_bytes.load(Ordering::SeqCst), 6789);

    mule_daemon::rpc::handlers::settings::handle_stats_reset_lifetime_totals(&state)
        .await
        .unwrap();

    assert_eq!(state.lifetime_downloaded_bytes.load(Ordering::SeqCst), 0);
    assert_eq!(state.lifetime_uploaded_bytes.load(Ordering::SeqCst), 0);

    let dl = state.downloads.lock().await;
    assert_eq!(
        dl.get_setting("lifetime_downloaded_bytes")
            .unwrap()
            .as_deref(),
        Some("0")
    );
    assert_eq!(
        dl.get_setting("lifetime_uploaded_bytes")
            .unwrap()
            .as_deref(),
        Some("0")
    );
}

/// `shares.list` enriquece cada archivo con la tasa medida agregada en sus
/// ranuras activas y el total subido acumulado (hidratado desde
/// `share_upload_stats`) — unión que no distingue entre mayúsculas y minúsculas en el hash.
#[tokio::test]
async fn shares_list_enriches_per_file_upload_stats() {
    let tmp = tempdir().unwrap();
    let mut downloads = DownloadService::in_memory(tmp.path().join("temp")).unwrap();
    // Total persistido de un «arranque anterior».
    downloads.add_share_uploaded_bytes("HAA", 7000).unwrap();
    let config = DaemonConfig::new(tmp.path().to_path_buf(), "t");
    let state = AppState::new(config, downloads);

    // Los compartidos apuntan a archivos reales: `list_all_shared_files`
    // ahora filtra las entradas cuya carga útil haya desaparecido del disco.
    let f_path = tmp.path().join("f.iso");
    let g_path = tmp.path().join("g.iso");
    std::fs::write(&f_path, b"f").unwrap();
    std::fs::write(&g_path, b"g").unwrap();
    {
        let mut dl = state.downloads.lock().await;
        dl.add_shared_file(&mule_core::model::SharedFile::new(
            "haa",
            "f.iso",
            100,
            f_path.to_string_lossy().as_ref(),
        ))
        .unwrap();
        dl.add_shared_file(&mule_core::model::SharedFile::new(
            "hbb",
            "g.iso",
            100,
            g_path.to_string_lossy().as_ref(),
        ))
        .unwrap();
    }
    let um = state.inbound_listener.upload_manager();
    // Hidratación en el arranque: el total persistido debe ser visible.
    assert_eq!(um.per_file_uploaded_bytes()["haa"], 7000);

    // Dos ranuras activas en el mismo archivo (distinto tamaño de mayúsculas/minúsculas en el hash).
    um.request_upload_slot("c1", "c1", "HAA", "f.iso", 1.0, "normal", false, None)
        .await
        .unwrap();
    um.request_upload_slot("c2", "c2", "haa", "f.iso", 1.0, "normal", false, None)
        .await
        .unwrap();
    um.update_progress("c1", 3000).await;
    um.update_progress("c2", 500).await;
    // La ventana de medición se abre en el primer envío — un segundo envío
    // tras la primera muestra (≥ 500 ms) produce una tasa medida.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    um.update_progress("c1", 1000).await;

    let out = mule_daemon::rpc::handlers::shares::handle_shares_list(&state)
        .await
        .unwrap();
    let files = out["sharedFiles"].as_array().unwrap();
    let f = files.iter().find(|f| f["fileHash"] == "haa").unwrap();
    assert_eq!(f["uploadedBytes"].as_u64().unwrap(), 7000 + 4500);
    assert!(f["uploadRateBps"].as_u64().unwrap() > 0);
    // El archivo sin actividad: tasa 0, total 0.
    let g = files.iter().find(|f| f["fileHash"] == "hbb").unwrap();
    assert_eq!(g["uploadedBytes"].as_u64().unwrap(), 0);
    assert_eq!(g["uploadRateBps"].as_u64().unwrap(), 0);
}

/// El archivo `.part` eliminado físicamente mientras el daemon está detenido:
/// al reiniciar, la entrada se retira de la lista (datos perdidos) y
/// el `.part.met` huérfano se limpia — en lugar de resucitar con
/// rangos fantasma que apuntan a un archivo vacío.
#[test]
fn missing_part_file_removes_download_at_startup() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("downloads.db");
    let temp_dir = tmp.path().join("temp");
    let target_dir = tmp.path().join("incoming");

    let (temp_path, met_path) = {
        let mut svc = DownloadService::new(&db_path, temp_dir.clone()).unwrap();
        let link = format!(
            "ed2k://|file|gone.bin|500|{}|/",
            hex::encode([0x66u8; 16])
        );
        let snap = svc
            .add_ed2k_link(&link, &target_dir, AllocationPolicy::None, None)
            .unwrap();
        let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
        let mut met_name = temp_path.as_os_str().to_os_string();
        met_name.push(".met");
        (temp_path, std::path::PathBuf::from(met_name))
    };
    assert!(temp_path.exists());
    assert!(met_path.exists());

    // Eliminación física del .part mientras el daemon está detenido.
    std::fs::remove_file(&temp_path).unwrap();

    // Reinicio: la entrada se retira y el .met huérfano se limpia.
    let svc = DownloadService::new(&db_path, temp_dir).unwrap();
    assert!(svc.list_downloads().is_empty());
    assert!(!met_path.exists());
}

/// `DAPPref` de eMule (por defecto `true`): una nueva descarga comienza con
/// prioridad «auto»; `SavePartFile` escribe `FT_DLPRIORITY = PR_AUTO` (5);
/// una prioridad manual escribe el valor `PR_*` correspondiente y sale del
/// modo auto; la reimportación del `.part.met` restaura la bandera.
#[test]
fn auto_down_priority_defaults_on_and_roundtrips_via_part_met() {
    let tmp = tempdir().unwrap();
    let mut svc = DownloadService::in_memory(tmp.path().join("temp")).unwrap();

    let link = format!("ed2k://|file|prio.bin|1000|{}|/", hex::encode([0x44u8; 16]));
    let snap = svc
        .add_ed2k_link(&link, tmp.path(), AllocationPolicy::None, None)
        .unwrap();
    assert_eq!(snap.priority, "auto", "DAPPref on → new download is auto");

    let (temp_path, _, _, _) = svc.get_download_file_info(&snap.id).unwrap();
    let mut met_name = temp_path.as_os_str().to_os_string();
    met_name.push(".met");
    let met_path = std::path::PathBuf::from(met_name);

    let dl_prio = |path: &std::path::Path| {
        mule_format::PartMetFile::open(path)
            .unwrap()
            .tags
            .iter()
            .find(|t| t.name_id == mule_format::met::tags::FT_DLPRIORITY)
            .and_then(|t| t.as_u64())
    };
    assert_eq!(dl_prio(&met_path), Some(5), "PR_AUTO tag in .part.met");

    // Prioridad manual → desactiva el modo auto (SetDownPriority(PR_LOW) en eMule).
    let snap = svc.set_priority(&snap.id, "low").unwrap();
    assert_eq!(snap.priority, "low");
    svc.save_active_part_mets();
    assert_eq!(dl_prio(&met_path), Some(0), "PR_LOW tag in .part.met");

    // Retorno a auto → bandera restaurada.
    let snap = svc.set_priority(&snap.id, "auto").unwrap();
    assert_eq!(snap.priority, "auto");
    svc.save_active_part_mets();
    assert_eq!(dl_prio(&met_path), Some(5));

    // Prioridad no válida → rechazada, el estado no se modifica.
    assert!(svc.set_priority(&snap.id, "urgent").is_err());
    assert_eq!(svc.get_download(&snap.id).unwrap().priority, "auto");

    // Reimportación del met en modo «auto» → bandera restaurada del lado de la descarga. Copia bajo
    // un nombre neutro: `remove_download` renombra el acompañante a `.keep`
    // para evitar la resurrección en el arranque — el par sigue siendo importable.
    let import_dir = tmp.path().join("import");
    std::fs::create_dir_all(&import_dir).unwrap();
    let import_met = import_dir.join("foreign.part.met");
    std::fs::copy(&met_path, &import_met).unwrap();
    svc.remove_download(&snap.id, false).unwrap();
    let imported = svc
        .import_part_met(&import_met, Some(&temp_path), tmp.path(), None)
        .unwrap();
    assert_eq!(imported.download.priority, "auto");
}
