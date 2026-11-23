// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_core::AllocationPolicy;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::JsonRpcError;
use crate::state::AppState;

fn torrent_info_hash_to_uuid(hash: &str) -> Uuid {
    if let Ok(bytes) = hex::decode(hash) {
        if bytes.len() >= 16 {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&bytes[..16]);
            return Uuid::from_bytes(arr);
        }
    }
    Uuid::nil()
}

pub async fn handle_downloads_list(state: &AppState) -> Result<Value, JsonRpcError> {
    let seq = state.sequence.load(std::sync::atomic::Ordering::SeqCst);
    let mut list = state.downloads.lock().await.list_download_details();
    let active_nas_id = state.current_nas_transfer().await;
    // Adjuntar rangos de bloques en vuelo (segmentos amarillos "pendientes" de eMule) y bandera de cola del NAS.
    let pending_map = state.peer_session.pending_ranges_by_file().await;
    for detail in &mut list {
        if let Some(ranges) = pending_map.get(&detail.snapshot.ed2k_hash) {
            detail.snapshot.pending_ranges = ranges.clone();
        }
        state.enrich_completion_metadata(&mut detail.snapshot, active_nas_id);
    }

    // Adjuntar descargas activas de BitTorrent desde el motor mule-torrent
    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let u = torrent_info_hash_to_uuid(&t.info_hash);
            list.push(crate::services::DownloadDetail {
                snapshot: crate::services::DownloadSnapshot {
                    id: u,
                    file_name: t.name.clone(),
                    total_bytes: t.total_bytes,
                    received_bytes: t.downloaded_bytes,
                    verified_bytes: t.downloaded_bytes,
                    progress: t.progress,
                    status: t.status,
                    priority: "auto".to_string(),
                    source_count: t.peer_count as u32,
                    download_rate_bps: t.download_rate_bps,
                    uploaded_bytes: 0,
                    created_at_unix_ms: 0,
                    updated_at_unix_ms: 0,
                    category_id: t.category,
                    eta_seconds: None,
                    ed2k_hash: t.info_hash.clone(),
                    parts: vec![],
                    pending_ranges: vec![],
                    max_sources: None,
                    nas_copy_active: None,
                    completion_stage: None,
                    copy_progress: None,
                    protocol: Some("torrent".to_string()),
                },
                ed2k_hash: t.info_hash.clone(),
                aich_root: None,
                ed2k_link: format!("magnet:?xt=urn:btih:{}", t.info_hash),
                temp_path: String::new(),
                target_path: t.output_dir,
                remaining_bytes: t.total_bytes.saturating_sub(t.downloaded_bytes),
                average_rate_bps: t.download_rate_bps,
                eta_seconds: None,
                connection_attempts: t.peer_count as u32,
                last_error: None,
                last_status_change_unix_ms: 0,
                parts: vec![],
            });
        }
    }
    Ok(json!({
        "sequence": seq,
        "downloads": list
    }))
}

#[tracing::instrument(skip_all, fields(download_id = tracing::field::Empty))]
pub async fn handle_downloads_get(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    tracing::Span::current().record("download_id", tracing::field::display(id));

    let mut dl = state
        .downloads
        .lock()
        .await
        .get_download_detail(&id)
        .ok_or_else(|| JsonRpcError::business(format!("Download {id} not found")))?;

    dl.snapshot.pending_ranges = state
        .peer_session
        .pending_ranges_for_file(&dl.ed2k_hash)
        .await;

    let active_nas_id = state.current_nas_transfer().await;
    state.enrich_completion_metadata(&mut dl.snapshot, active_nas_id);

    Ok(crate::rpc::handlers::json_value(&dl))
}

#[tracing::instrument(
    skip_all,
    fields(download_id = tracing::field::Empty, ed2k_hash = tracing::field::Empty)
)]
pub async fn handle_add_ed2k_link(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let link = params["link"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'link' parameter"))?;
    let target_dir_opt = params["targetDirectory"]
        .as_str()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != ".");

    let category_id_opt = params["categoryId"]
        .as_str()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let category_dest = if let Some(ref cid) = category_id_opt {
        state
            .downloads
            .lock()
            .await
            .get_category(cid)
            .ok()
            .flatten()
            .and_then(|c| c.default_destination)
    } else {
        None
    };

    let global_db_dir = {
        state
            .downloads
            .lock()
            .await
            .get_setting("default_download_directory")
            .ok()
            .flatten()
            .map(std::path::PathBuf::from)
    };

    let fallback_bundle_dir = state.config.default_download_dir.clone();
    let effective_base_dir = global_db_dir
        .clone()
        .unwrap_or_else(|| fallback_bundle_dir.clone());

    // Resolución en 4 niveles:
    // Nivel 1: targetDirectory explícito
    // Nivel 2: destino predeterminado de la categoría
    // Nivel 3: ajuste global de SQLite default_download_directory
    // Nivel 4: directorio de descarga predeterminado de la raíz del paquete
    let target_dir = if let Some(p) = target_dir_opt {
        let path = std::path::PathBuf::from(p);
        if path.is_absolute() {
            path
        } else {
            effective_base_dir.join(path)
        }
    } else if let Some(cat_dest) = category_dest {
        let path = std::path::PathBuf::from(&cat_dest);
        if path.is_absolute() {
            path
        } else {
            effective_base_dir.join(path)
        }
    } else if let Some(global_dir) = global_db_dir {
        global_dir
    } else {
        fallback_bundle_dir
    };
    let _ = std::fs::create_dir_all(&target_dir);

    let policy_str = params["allocationPolicy"]
        .as_str()
        .or_else(|| params["policy"].as_str());
    let policy = match policy_str {
        Some("preallocate") => AllocationPolicy::Preallocate,
        Some("none") => AllocationPolicy::None,
        _ => AllocationPolicy::SparseIfSupported,
    };

    if link.starts_with("magnet:") || link.contains("xt=urn:btih:") {
        let engine = state
            .torrent_engine()
            .await
            .map_err(JsonRpcError::business)?;
        let snap = engine
            .add_url(link, Some(target_dir), category_id_opt)
            .await
            .map_err(|e| JsonRpcError::business(e.to_string()))?;
        let u = torrent_info_hash_to_uuid(&snap.info_hash);
        return Ok(json!({
            "downloadId": u.to_string(),
            "status": snap.status,
            "isTorrent": true,
            "name": snap.name,
        }));
    }

    let mut snapshot =
        state
            .downloads
            .lock()
            .await
            .add_ed2k_link(link, &target_dir, policy, category_id_opt)?;

    if let Some(prio) = params["priority"].as_str() {
        if let Ok(updated) = state.downloads.lock().await.set_priority(&snapshot.id, prio) {
            snapshot = updated;
        }
    }

    tracing::Span::current().record("download_id", tracing::field::display(snapshot.id));

    state.broadcast_download_update(snapshot.clone());

    // Desencadenar consulta inmediata de fuentes al servidor y a Kad
    let file_info = {
        state
            .downloads
            .lock()
            .await
            .get_download_file_info(&snapshot.id)
    };
    if let Some((_, _, hash, size)) = file_info {
        let hash_hex = hex::encode(hash.0);
        // Hash truncado en el span (8 caracteres hex bastan para correlacionar; el enlace
        // completo y el nombre de archivo nunca se registran).
        tracing::Span::current().record("ed2k_hash", &hash_hex[..8.min(hash_hex.len())]);
        tracing::info!(
            "Download added: requesting immediate sources from server and Kad for file {}",
            hash_hex
        );
        if state.server_session.snapshot().await.status
            == crate::services::server_session::ServerConnectionStatus::Connected
        {
            let req = mule_protocol::ed2k::GetSourcesRequest::with_obfuscation(
                hash.0,
                size,
                state.effective_obfuscation().await.0,
            );
            let _ = state.server_session.get_sources(req).await;
        }
        if state.network_policy.profile().allows_udp_kad() {
            let kad = state.kad.clone();
            tokio::spawn(async move {
                let _ = kad.search_sources(&hash_hex, size).await;
            });
        }
    }

    Ok(json!({
        "downloadId": snapshot.id.to_string(),
        "status": snapshot.status
    }))
}

#[tracing::instrument(skip_all, fields(download_id = tracing::field::Empty))]
pub async fn handle_pause(state: &AppState, params: Option<Value>) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    tracing::Span::current().record("download_id", tracing::field::display(id));

    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let u = torrent_info_hash_to_uuid(&t.info_hash);
            if u == id {
                let _ = engine.pause(t.id).await;
                return Ok(json!({ "id": id.to_string(), "status": "paused" }));
            }
        }
    }

    let snapshot = state.downloads.lock().await.pause_download(&id)?;
    state.broadcast_download_update(snapshot.clone());

    Ok(crate::rpc::handlers::json_value(&snapshot))
}

#[tracing::instrument(skip_all, fields(download_id = tracing::field::Empty))]
pub async fn handle_resume(state: &AppState, params: Option<Value>) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    tracing::Span::current().record("download_id", tracing::field::display(id));

    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let u = torrent_info_hash_to_uuid(&t.info_hash);
            if u == id {
                let _ = engine.resume(t.id).await;
                return Ok(json!({ "id": id.to_string(), "status": "downloading" }));
            }
        }
    }

    // `SafeConnect` de eMule: rechaza reanudar si ni el servidor eD2K
    // ni Kad están accesibles — las fuentes solo podrían ser entradas de caché inactivas.
    if state.setting_bool("safe_connect", false).await {
        let server_up = state.server_session.snapshot().await.status
            == crate::services::server_session::ServerConnectionStatus::Connected;
        let kad_ready = state.kad.get_status().await.total_contacts > 0;
        if !server_up && !kad_ready {
            return Err(JsonRpcError::business(
                "Safe connect: waiting for an eD2K server or Kad before resuming downloads",
            ));
        }
    }

    let snapshot = state.downloads.lock().await.resume_download(&id)?;
    state.broadcast_download_update(snapshot.clone());

    // Desencadenar consulta inmediata de fuentes al reanudar
    let file_info = {
        state
            .downloads
            .lock()
            .await
            .get_download_file_info(&snapshot.id)
    };
    if let Some((_, _, hash, size)) = file_info {
        let hash_hex = hex::encode(hash.0);
        if state.server_session.snapshot().await.status
            == crate::services::server_session::ServerConnectionStatus::Connected
        {
            let req = mule_protocol::ed2k::GetSourcesRequest::with_obfuscation(
                hash.0,
                size,
                state.effective_obfuscation().await.0,
            );
            let _ = state.server_session.get_sources(req).await;
        }
        if state.network_policy.profile().allows_udp_kad() {
            let kad = state.kad.clone();
            tokio::spawn(async move {
                let _ = kad.search_sources(&hash_hex, size).await;
            });
        }
    }

    Ok(crate::rpc::handlers::json_value(&snapshot))
}

#[tracing::instrument(skip_all, fields(download_id = tracing::field::Empty))]
pub async fn handle_remove(state: &AppState, params: Option<Value>) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    tracing::Span::current().record("download_id", tracing::field::display(id));
    let delete_files = params["deleteFiles"].as_bool().unwrap_or(false);

    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let u = torrent_info_hash_to_uuid(&t.info_hash);
            if u == id {
                let _ = engine.delete(t.id, delete_files).await;
                return Ok(json!({ "success": true, "id": id.to_string() }));
            }
        }
    }

    state
        .downloads
        .lock()
        .await
        .remove_download(&id, delete_files)?;

    Ok(json!({ "success": true }))
}

pub async fn handle_set_destination(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    let target_dir_str = params["targetDirectory"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'targetDirectory' parameter"))?;

    let target_dir = std::path::PathBuf::from(target_dir_str);
    let detail = state
        .downloads
        .lock()
        .await
        .set_destination(&id, &target_dir)?;

    state.broadcast_download_update(detail.snapshot.clone());

    Ok(crate::rpc::handlers::json_value(&detail))
}

/// `downloads.rename` — renombra el archivo (`{id, newName}`). Para una
/// descarga completada, renombra también el archivo en disco y la entrada de
/// compartición asociada.
pub async fn handle_rename(state: &AppState, params: Option<Value>) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    let new_name = params["newName"]
        .as_str()
        .or_else(|| params["new_name"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'newName' parameter"))?;

    let snapshot = state
        .downloads
        .lock()
        .await
        .rename_download(&id, new_name)?;
    state.broadcast_download_update(snapshot.clone());

    Ok(crate::rpc::handlers::json_value(&snapshot))
}

pub async fn handle_set_priority(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    let priority = params["priority"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'priority' parameter"))?;

    let snapshot = state.downloads.lock().await.set_priority(&id, priority)?;
    state.broadcast_download_update(snapshot.clone());

    Ok(crate::rpc::handlers::json_value(&snapshot))
}

/// `downloads.setMaxSources` — límite de fuentes por archivo (FT_MAXSOURCES de eMule).
/// Parámetros: `{id, maxSources}` — `maxSources` nulo u omitido restablece el valor global.
pub async fn handle_set_max_sources(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    let max_sources = params
        .get("maxSources")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let snapshot = state
        .downloads
        .lock()
        .await
        .set_max_sources(&id, max_sources)?;
    state.broadcast_download_update(snapshot.clone());

    Ok(crate::rpc::handlers::json_value(&snapshot))
}

pub async fn handle_inspect_part_met(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let met_path_str = params["metPath"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'metPath' parameter"))?;
    let part_path_str = params["partPath"].as_str().filter(|s| !s.trim().is_empty());

    let met_path = std::path::Path::new(met_path_str);
    let part_path = part_path_str.map(std::path::Path::new);

    let inspection = state
        .downloads
        .lock()
        .await
        .inspect_part_met(met_path, part_path)?;

    Ok(crate::rpc::handlers::json_value(&inspection))
}

pub async fn handle_import_part_met(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let met_path_str = params["metPath"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'metPath' parameter"))?;
    let part_path_str = params["partPath"].as_str().filter(|s| !s.trim().is_empty());

    let target_dir_opt = params["targetDirectory"]
        .as_str()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != ".");

    let category_id_opt = params["categoryId"]
        .as_str()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let category_dest = if let Some(ref cid) = category_id_opt {
        state
            .downloads
            .lock()
            .await
            .get_category(cid)
            .ok()
            .flatten()
            .and_then(|c| c.default_destination)
    } else {
        None
    };

    let global_db_dir = {
        state
            .downloads
            .lock()
            .await
            .get_setting("default_download_directory")
            .ok()
            .flatten()
            .map(std::path::PathBuf::from)
    };

    let fallback_bundle_dir = state.config.default_download_dir.clone();
    let effective_base_dir = global_db_dir
        .clone()
        .unwrap_or_else(|| fallback_bundle_dir.clone());

    let target_dir = if let Some(p) = target_dir_opt {
        let path = std::path::PathBuf::from(p);
        if path.is_absolute() {
            path
        } else {
            effective_base_dir.join(path)
        }
    } else if let Some(cat_dest) = category_dest {
        let path = std::path::PathBuf::from(&cat_dest);
        if path.is_absolute() {
            path
        } else {
            effective_base_dir.join(path)
        }
    } else if let Some(global_dir) = global_db_dir {
        global_dir
    } else {
        fallback_bundle_dir
    };
    let _ = std::fs::create_dir_all(&target_dir);

    let met_path = std::path::Path::new(met_path_str);
    let part_path = part_path_str.map(std::path::Path::new);

    let result = state.downloads.lock().await.import_part_met(
        met_path,
        part_path,
        &target_dir,
        category_id_opt,
    )?;

    state.broadcast_download_update(result.download.clone());

    Ok(crate::rpc::handlers::json_value(&result))
}

/// `downloads.importCancelledMet` — `CKnownFileList::LoadCancelled`:
/// importa un archivo `cancelled.met` de eMule. Las entradas enmascaradas `0x0F` conservan
/// su semilla; los hashes reales legados `0x0E` se enmascaran bajo la nuestra.
/// Parámetros: `metPath` (obligatorio).
pub async fn handle_import_cancelled_met(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let met_path = params
        .as_ref()
        .and_then(|p| p["metPath"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'metPath'"))?;
    let report = state
        .downloads
        .lock()
        .await
        .import_cancelled_met(std::path::Path::new(met_path))?;
    Ok(crate::rpc::handlers::json_value(&report))
}

/// `downloads.isCancelled` — `IsCancelledFileByID`: informa si un
/// hash eD2K pertenece al conjunto cancelado (marcado en resultados de búsqueda; no es
/// un bloqueo de descarga, al igual que en eMule).
/// Parámetros: `fileHash` (hexadecimal, obligatorio).
pub async fn handle_is_cancelled(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let file_hash = params
        .as_ref()
        .and_then(|p| p["fileHash"].as_str())
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'fileHash'"))?;
    let cancelled = state.downloads.lock().await.is_cancelled_file(file_hash)?;
    let total = state.downloads.lock().await.cancelled_count()?;
    Ok(json!({ "cancelled": cancelled, "cancelledCount": total }))
}

#[tracing::instrument(skip_all, fields(download_id = tracing::field::Empty))]
pub async fn handle_retry_connection(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str).map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;
    tracing::Span::current().record("download_id", tracing::field::display(id));

    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let u = torrent_info_hash_to_uuid(&t.info_hash);
            if u == id {
                let _ = engine.pause(t.id).await;
                let _ = engine.resume(t.id).await;
                return Ok(json!({ "id": id.to_string(), "status": "downloading", "restarted": true }));
            }
        }
    }

    let detail = state.downloads.lock().await.retry_connection(&id)?;

    state.broadcast_download_update(detail.snapshot.clone());

    Ok(crate::rpc::handlers::json_value(&detail))
}

pub async fn handle_downloads_set_category(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id_str = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;
    let id = Uuid::parse_str(id_str)
        .map_err(|e| JsonRpcError::invalid_params(&format!("Invalid UUID: {e}")))?;

    let category_id = params["categoryId"].as_str().map(String::from);

    let detail = state
        .downloads
        .lock()
        .await
        .set_download_category(&id, category_id)?;

    state.broadcast_download_update(detail.snapshot.clone());

    serde_json::to_value(detail)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}
