// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Capa de emulación de Transmission RPC para Sonarr, Radarr y el ecosistema *arr.
//! Implementa la especificación `/transmission/rpc` con gestión de CSRF (`X-Transmission-Session-Id`),
//! soporte de protocolo dual (BitTorrent y eD2K) y seguimiento unificado del ciclo de vida.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use data_encoding::BASE64;
use mule_core::AllocationPolicy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::state::AppState;

const TRANSMISSION_RPC_VERSION: i32 = 17;
const TRANSMISSION_RPC_VERSION_MIN: i32 = 14;
const TRANSMISSION_VERSION: &str = "4.0.5";
const TRANSMISSION_SESSION_HEADER: &str = "x-transmission-session-id";
const TORRENT_ID_OFFSET: i64 = 100_000;
/// Tamaño máximo permitido para el cuerpo JSON de una petición Transmission RPC (8 MiB).
const MAX_TRANSMISSION_RPC_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct TransmissionRequest {
    pub method: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub tag: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct TransmissionResponse {
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<Value>,
}

impl TransmissionResponse {
    pub fn success(arguments: Value, tag: Option<Value>) -> Self {
        Self {
            result: "success".to_string(),
            arguments: Some(arguments),
            tag,
        }
    }

    pub fn error(msg: impl Into<String>, tag: Option<Value>) -> Self {
        Self {
            result: msg.into(),
            arguments: None,
            tag,
        }
    }
}

/// Convierte el UUID de una descarga en un identificador entero positivo de 31 bits determinista para los clientes Transmission.
pub fn uuid_to_transmission_id(id: &Uuid) -> i64 {
    let bytes = id.as_bytes();
    let n = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    (n & 0x7FFF_FFFF) as i64
}

/// Manejador principal de Axum para `/transmission/rpc`.
/// Gestiona tanto GET (para comprobaciones simples) como POST (JSON-RPC).
pub async fn handle_transmission_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 0. Validar credenciales opcionales (o requeridas cuando se vincula a interfaces distintas de loopback)
    let expected_token = state.config.auth_token.as_str();
    let is_loopback_bind = state.config.bind_addr.ip().is_loopback();
    if let Some(auth_val) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if !expected_token.is_empty() {
            let valid = if let Some(bearer) = auth_val.strip_prefix("Bearer ") {
                bearer.trim() == expected_token
            } else if let Some(basic_b64) = auth_val.strip_prefix("Basic ") {
                BASE64
                    .decode(basic_b64.trim().as_bytes())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .map(|creds| {
                        let (user, pass) = creds.split_once(':').unwrap_or((&creds, ""));
                        pass == expected_token || user == expected_token || (is_loopback_bind && pass.is_empty())
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            if !valid {
                return (
                    StatusCode::UNAUTHORIZED,
                    "<h1>401: Unauthorized</h1>",
                )
                    .into_response();
            }
        }
    } else if !is_loopback_bind && !expected_token.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            "<h1>401: Unauthorized</h1>",
        )
            .into_response();
    }

    // 1. Aplicar comprobación de cabecera CSRF (X-Transmission-Session-Id)
    // Sonarr/Radarr esperan HTTP 409 Conflict con la cabecera del token de sesión activo en la negociación inicial.
    let expected_session = state.instance_id.to_string();
    let client_session = headers
        .get(TRANSMISSION_SESSION_HEADER)
        .and_then(|v| v.to_str().ok());

    match client_session {
        Some(s) if s == expected_session => {
            // El token coincide, continuar
        }
        _ => {
            // Discrepancia o ausente: devolver 409 Conflict con la cabecera para que el cliente la capture y reintente
            return (
                StatusCode::CONFLICT,
                [(TRANSMISSION_SESSION_HEADER, expected_session)],
                "<h1>409: Conflict</h1><p>Your request had an invalid session-id header.</p>",
            )
                .into_response();
        }
    }

    // 2. Analizar carga útil de la petición
    if body.is_empty() {
        // Cuerpo vacío o petición GET tras validar CSRF: devolver valor predeterminado de session-get
        let resp = handle_session_get(&state, None).await;
        return (
            StatusCode::OK,
            [(TRANSMISSION_SESSION_HEADER, state.instance_id.to_string())],
            Json(resp),
        )
            .into_response();
    }

    if body.len() > MAX_TRANSMISSION_RPC_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(TransmissionResponse::error("Payload too large", None)),
        )
            .into_response();
    }

    let Json(req) = match Json::<TransmissionRequest>::from_bytes(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(TransmissionResponse::error(
                    format!("Invalid JSON payload: {e}"),
                    None,
                )),
            )
                .into_response();
        }
    };

    let resp = match req.method.as_str() {
        "session-get" => handle_session_get(&state, req.tag).await,
        "session-set" => handle_session_set(&state, req.arguments, req.tag).await,
        "session-stats" => handle_session_stats(&state, req.tag).await,
        "free-space" => handle_free_space(&state, req.arguments, req.tag).await,
        "torrent-get" => handle_torrent_get(&state, req.arguments, req.tag).await,
        "torrent-add" => handle_torrent_add(&state, req.arguments, req.tag).await,
        "torrent-start" | "torrent-start-now" => {
            handle_torrent_start(&state, req.arguments, req.tag).await
        }
        "torrent-stop" => handle_torrent_stop(&state, req.arguments, req.tag).await,
        "torrent-remove" => handle_torrent_remove(&state, req.arguments, req.tag).await,
        unknown => {
            tracing::warn!("Unsupported Transmission RPC method requested: {unknown}");
            TransmissionResponse::error(format!("Method '{unknown}' not implemented"), req.tag)
        }
    };

    (
        StatusCode::OK,
        [(TRANSMISSION_SESSION_HEADER, state.instance_id.to_string())],
        Json(resp),
    )
        .into_response()
}

async fn handle_session_get(state: &AppState, tag: Option<Value>) -> TransmissionResponse {
    let dl_dir = state
        .config
        .default_download_dir
        .to_string_lossy()
        .to_string();
    let temp_dir = state
        .config
        .data_dir
        .join("temp")
        .to_string_lossy()
        .to_string();

    let args = json!({
        "version": TRANSMISSION_VERSION,
        "rpc-version": TRANSMISSION_RPC_VERSION,
        "rpc-version-minimum": TRANSMISSION_RPC_VERSION_MIN,
        "download-dir": dl_dir,
        "incomplete-dir": temp_dir,
        "incomplete-dir-enabled": true,
        "peer-port": state.ed2k_port(),
        "speed-limit-down": 0,
        "speed-limit-down-enabled": false,
        "speed-limit-up": 0,
        "speed-limit-up-enabled": false,
        "units": {
            "speed-units": ["kB/s", "MB/s", "GB/s", "TB/s"],
            "speed-bytes": 1024,
            "size-units": ["kB", "MB", "GB", "TB"],
            "size-bytes": 1024,
            "memory-units": ["kB", "MB", "GB", "TB"],
            "memory-bytes": 1024
        }
    });

    TransmissionResponse::success(args, tag)
}

/// Manejador de `session-set` de Transmission RPC.
///
/// Es intencionadamente un *no-op* de compatibilidad para clientes externos
/// (Radarr/Sonarr), ya que la configuración del daemon se gestiona únicamente
/// a través del JSON-RPC interno.
async fn handle_session_set(
    _state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    if let Some(obj) = args.as_object() {
        if !obj.is_empty() {
            let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
            tracing::debug!(
                ?keys,
                "Transmission RPC session-set ignorado intencionadamente (ajustes gestionados vía JSON-RPC interno)"
            );
        }
    }
    TransmissionResponse::success(json!({}), tag)
}

async fn handle_session_stats(state: &AppState, tag: Option<Value>) -> TransmissionResponse {
    let list = state.downloads.lock().await.list_download_details();
    let mut total_download_speed: u64 = 0;
    let mut total_upload_speed: u64 = 0;
    let mut active_count: u32 = 0;
    let mut paused_count: u32 = 0;
    let mut total_torrents = list.len();

    for item in &list {
        if item.snapshot.status.eq_ignore_ascii_case("downloading") {
            active_count += 1;
            total_download_speed += item.snapshot.download_rate_bps;
        } else if item.snapshot.status.eq_ignore_ascii_case("paused") {
            paused_count += 1;
        }
    }

    if let Some(engine) = state.torrent.get() {
        let torrents = engine.list();
        total_torrents += torrents.len();
        for t in torrents {
            if t.status == "downloading" {
                active_count += 1;
                total_download_speed += t.download_rate_bps;
                total_upload_speed += t.upload_rate_bps;
            } else if t.status == "paused" {
                paused_count += 1;
            }
        }
    }

    let args = json!({
        "activeTorrentCount": active_count,
        "downloadSpeed": total_download_speed,
        "uploadSpeed": total_upload_speed,
        "pausedTorrentCount": paused_count,
        "torrentCount": total_torrents,
    });

    TransmissionResponse::success(args, tag)
}

async fn handle_free_space(
    state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    let req_path = args["path"].as_str().map(Path::new);
    let target = req_path.unwrap_or(&state.config.default_download_dir);

    let free_bytes = fs4::available_space(target).unwrap_or(100 * 1024 * 1024 * 1024);

    let out = json!({
        "path": target.to_string_lossy(),
        "size-bytes": free_bytes
    });

    TransmissionResponse::success(out, tag)
}

fn map_status_to_transmission(status: &str) -> i64 {
    match status.to_lowercase().as_str() {
        "completed" => 6,
        "downloading" => 4,
        "waiting" | "allocating" | "verifying" => 3,
        "paused" | "error" | "cancelled" => 0,
        _ => 0,
    }
}

async fn handle_torrent_get(
    state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    let details = state.downloads.lock().await.list_download_details();
    let ids_filter = args["ids"].as_array();

    let mut torrents = Vec::new();
    let default_dl_dir = state
        .config
        .default_download_dir
        .to_string_lossy()
        .to_string();

    // 1. Procesar descargas eD2k desde mule-core
    for d in details {
        let numeric_id = uuid_to_transmission_id(&d.snapshot.id);
        let hash_lower = d.snapshot.ed2k_hash.to_lowercase();

        if let Some(ids) = ids_filter {
            let matched = ids.iter().any(|id_val| {
                if let Some(n) = id_val.as_i64() {
                    n == numeric_id
                } else if let Some(s) = id_val.as_str() {
                    s.eq_ignore_ascii_case(&hash_lower)
                        || s.eq_ignore_ascii_case(&d.snapshot.id.to_string())
                } else {
                    false
                }
            });
            if !matched {
                continue;
            }
        }

        let is_completed = d.snapshot.status.eq_ignore_ascii_case("completed");
        let percent_done = if is_completed {
            1.0
        } else if d.snapshot.total_bytes > 0 {
            (d.snapshot.received_bytes as f64 / d.snapshot.total_bytes as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let left_until_done = if is_completed {
            0
        } else {
            d.snapshot
                .total_bytes
                .saturating_sub(d.snapshot.received_bytes)
        };

        let status_code = map_status_to_transmission(&d.snapshot.status);

        let download_dir = if !d.target_path.is_empty() {
            Path::new(&d.target_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| default_dl_dir.clone())
        } else {
            default_dl_dir.clone()
        };

        let labels = if let Some(ref cat) = d.snapshot.category_id {
            vec![cat.clone()]
        } else {
            vec![]
        };

        let files = json!([
            {
                "bytesCompleted": if is_completed { d.snapshot.total_bytes } else { d.snapshot.received_bytes },
                "length": d.snapshot.total_bytes,
                "name": d.snapshot.file_name,
            }
        ]);

        let torrent_obj = json!({
            "id": numeric_id,
            "hashString": hash_lower,
            "name": d.snapshot.file_name,
            "totalSize": d.snapshot.total_bytes,
            "leftUntilDone": left_until_done,
            "percentDone": percent_done,
            "rateDownload": d.snapshot.download_rate_bps,
            "rateUpload": 0,
            "uploadRatio": 0.0,
            "status": status_code,
            "isFinished": is_completed,
            "downloadDir": download_dir,
            "error": 0,
            "errorString": "",
            "eta": d.snapshot.eta_seconds.unwrap_or(0),
            "labels": labels,
            "addedDate": d.snapshot.created_at_unix_ms / 1000,
            "doneDate": if is_completed { d.snapshot.updated_at_unix_ms / 1000 } else { 0 },
            "files": files,
            "peersConnected": d.snapshot.source_count,
            "peersGettingFromUs": 0,
            "peersSendingToUs": d.snapshot.source_count,
            "secondsDownloading": 0,
            "secondsSeeding": 0,
        });

        torrents.push(torrent_obj);
    }

    // 2. Procesar descargas BitTorrent desde mule-torrent
    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let numeric_id = (t.id as i64) + TORRENT_ID_OFFSET;
            let hash_lower = t.info_hash.to_lowercase();

            if let Some(ids) = ids_filter {
                let matched = ids.iter().any(|id_val| {
                    if let Some(n) = id_val.as_i64() {
                        n == numeric_id
                    } else if let Some(s) = id_val.as_str() {
                        s.eq_ignore_ascii_case(&hash_lower)
                    } else {
                        false
                    }
                });
                if !matched {
                    continue;
                }
            }

            let is_completed = t.status == "completed";
            let left_until_done = t.total_bytes.saturating_sub(t.downloaded_bytes);
            let status_code = map_status_to_transmission(&t.status);

            let labels = if let Some(ref cat) = t.category {
                vec![cat.clone()]
            } else {
                vec![]
            };

            let files = json!([
                {
                    "bytesCompleted": t.downloaded_bytes,
                    "length": t.total_bytes,
                    "name": t.name,
                }
            ]);

            let torrent_obj = json!({
                "id": numeric_id,
                "hashString": hash_lower,
                "name": t.name,
                "totalSize": t.total_bytes,
                "leftUntilDone": left_until_done,
                "percentDone": t.progress,
                "rateDownload": t.download_rate_bps,
                "rateUpload": t.upload_rate_bps,
                "uploadRatio": 0.0,
                "status": status_code,
                "isFinished": is_completed,
                "downloadDir": t.output_dir,
                "error": 0,
                "errorString": "",
                "eta": 0,
                "labels": labels,
                "addedDate": 0,
                "doneDate": 0,
                "files": files,
                "peersConnected": t.peer_count,
                "peersGettingFromUs": 0,
                "peersSendingToUs": t.peer_count,
                "secondsDownloading": 0,
                "secondsSeeding": 0,
            });

            torrents.push(torrent_obj);
        }
    }

    TransmissionResponse::success(json!({ "torrents": torrents }), tag)
}

fn url_decode(s: &str) -> String {
    let mut bytes = Vec::new();
    let s_bytes = s.as_bytes();
    let mut i = 0;
    while i < s_bytes.len() {
        match s_bytes[i] {
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < s_bytes.len() => {
                if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&s_bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    bytes.push(b);
                    i += 3;
                } else {
                    bytes.push(b'%');
                    i += 1;
                }
            }
            c => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    let decoded = String::from_utf8_lossy(&bytes).to_string();
    mule_format::fix_mojibake(&decoded)
}

fn parse_padded_magnet_to_ed2k(magnet: &str) -> Option<String> {
    if !magnet.starts_with("magnet:?") {
        return None;
    }
    let query = &magnet["magnet:?".len()..];
    let mut btih = None;
    let mut name = None;
    let mut size: Option<u64> = None;

    for param in query.split('&') {
        let mut parts = param.splitn(2, '=');
        let key = parts.next()?;
        let val = parts.next().unwrap_or("");
        match key {
            "xt" => {
                if let Some(h) = val.strip_prefix("urn:btih:") {
                    btih = Some(h);
                }
            }
            "dn" => {
                name = Some(url_decode(val));
            }
            "xl" => {
                if let Ok(s) = val.parse::<u64>() {
                    size = Some(s);
                }
            }
            _ => {}
        }
    }

    if let (Some(hash_40), Some(file_name), Some(file_size)) = (btih, name, size) {
        if hash_40.len() == 40 && hash_40.ends_with("00000000") {
            let ed2k_hash = &hash_40[..32];
            if ed2k_hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(format!("ed2k://|file|{file_name}|{file_size}|{ed2k_hash}|/"));
            }
        }
    }
    None
}

async fn handle_torrent_add(
    state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    let download_dir = args["download-dir"].as_str();
    let category = args["labels"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str());

    // 1. Comprobar si hay carga útil de .torrent codificada en Base64 (metainfo)
    if let Some(metainfo_b64) = args["metainfo"].as_str() {
        if let Ok(bytes) = BASE64.decode(metainfo_b64.as_bytes()) {
            let engine = match state.torrent_engine().await {
                Ok(eng) => eng,
                Err(e) => {
                    return TransmissionResponse::error(
                        format!("Failed to start torrent engine: {e}"),
                        tag,
                    )
                }
            };
            let target_dest = download_dir.map(PathBuf::from);
            match engine
                .add_bytes(bytes, target_dest, category.map(String::from))
                .await
            {
                Ok(snap) => {
                    let torrent_info = json!({
                        "id": (snap.id as i64) + TORRENT_ID_OFFSET,
                        "name": snap.name,
                        "hashString": snap.info_hash.to_lowercase()
                    });
                    return TransmissionResponse::success(
                        json!({ "torrent-added": torrent_info }),
                        tag,
                    );
                }
                Err(e) => {
                    return TransmissionResponse::error(
                        format!("Torrent rejected or invalid .torrent metainfo: {e}"),
                        tag,
                    );
                }
            }
        }
    }

    let filename = match args["filename"].as_str() {
        Some(f) => f.trim(),
        None => {
            return TransmissionResponse::error(
                "Missing 'filename' or 'metainfo' parameter in torrent-add",
                tag,
            )
        }
    };

    // 2. Comprobar URI magnet eD2k con relleno (compatibilidad con amutorrent / Torznab)
    let link_str = if let Some(ed2k_link) = parse_padded_magnet_to_ed2k(filename) {
        ed2k_link
    } else if filename.starts_with("magnet:") || filename.contains("xt=urn:btih:") || filename.ends_with(".torrent") {
        let engine = match state.torrent_engine().await {
            Ok(eng) => eng,
            Err(e) => {
                return TransmissionResponse::error(
                    format!("Failed to start torrent engine: {e}"),
                    tag,
                )
            }
        };
        let target_dest = download_dir.map(PathBuf::from);
        match engine
            .add_url(filename, target_dest, category.map(String::from))
            .await
        {
            Ok(snap) => {
                let torrent_info = json!({
                    "id": (snap.id as i64) + TORRENT_ID_OFFSET,
                    "name": snap.name,
                    "hashString": snap.info_hash.to_lowercase()
                });
                return TransmissionResponse::success(
                    json!({ "torrent-added": torrent_info }),
                    tag,
                );
            }
            Err(e) => {
                return TransmissionResponse::error(
                    format!("Failed to add magnet/torrent: {e}"),
                    tag,
                );
            }
        }
    } else if filename.starts_with("ed2k://") {
        filename.to_string()
    } else if filename.starts_with("http://") || filename.starts_with("https://") {
        match reqwest::get(filename).await {
            Ok(resp) => {
                let text = resp.text().await.unwrap_or_default();
                if let Some(pos) = text.find("ed2k://|file|") {
                    #[allow(clippy::manual_pattern_char_comparison)]
let end = text[pos..]
                        .find(|c| matches!(c, '"' | '\\' | '\n' | '\r' | '<'))
                        .unwrap_or(text.len() - pos);
                    text[pos..pos + end].to_string()
                } else if text.starts_with("magnet:") {
                    // ¡Redirigido a un enlace magnet!
                    let engine = match state.torrent_engine().await {
                        Ok(eng) => eng,
                        Err(e) => {
                            return TransmissionResponse::error(
                                format!("Failed to start torrent engine: {e}"),
                                tag,
                            )
                        }
                    };
                    let target_dest = download_dir.map(PathBuf::from);
                    match engine
                        .add_url(&text, target_dest, category.map(String::from))
                        .await
                    {
                        Ok(snap) => {
                            let torrent_info = json!({
                                "id": (snap.id as i64) + TORRENT_ID_OFFSET,
                                "name": snap.name,
                                "hashString": snap.info_hash.to_lowercase()
                            });
                            return TransmissionResponse::success(
                                json!({ "torrent-added": torrent_info }),
                                tag,
                            );
                        }
                        Err(e) => return TransmissionResponse::error(e.to_string(), tag),
                    }
                } else {
                    return TransmissionResponse::error(
                        "URL does not contain a valid ed2k:// or magnet: link",
                        tag,
                    );
                }
            }
            Err(e) => {
                return TransmissionResponse::error(
                    format!("Failed to fetch URL {filename}: {e}"),
                    tag,
                )
            }
        }
    } else {
        return TransmissionResponse::error(
            "Expected ed2k://, magnet:, or .torrent URL",
            tag,
        );
    };

    let target_dest = download_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.config.default_download_dir.clone());

    // Encolar descarga en DownloadService mediante add_ed2k_link
    let mut dl = state.downloads.lock().await;
    match dl.add_ed2k_link(
        &link_str,
        &target_dest,
        AllocationPolicy::SparseIfSupported,
        category.map(String::from),
    ) {
        Ok(snapshot) => {
            let numeric_id = uuid_to_transmission_id(&snapshot.id);
            let torrent_info = json!({
                "id": numeric_id,
                "name": snapshot.file_name,
                "hashString": snapshot.ed2k_hash.to_lowercase()
            });
            TransmissionResponse::success(json!({ "torrent-added": torrent_info }), tag)
        }
        Err(e) => {
            let err_msg = e.to_string();
            let existing = dl
                .list_download_details()
                .into_iter()
                .find(|d| link_str.contains(&d.snapshot.ed2k_hash));
            if let Some(d) = existing {
                let numeric_id = uuid_to_transmission_id(&d.snapshot.id);
                let torrent_info = json!({
                    "id": numeric_id,
                    "name": d.snapshot.file_name,
                    "hashString": d.snapshot.ed2k_hash.to_lowercase()
                });
                TransmissionResponse::success(
                    json!({ "torrent-duplicate": torrent_info }),
                    tag,
                )
            } else {
                TransmissionResponse::error(err_msg, tag)
            }
        }
    }
}

async fn handle_torrent_start(
    state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    let ids_val = &args["ids"];

    // 1. Reanudar descargas eD2k
    let mut dl = state.downloads.lock().await;
    let all = dl.list_download_details();
    for d in all {
        if match_torrent_id(&d, ids_val) {
            let _ = dl.resume_download(&d.snapshot.id);
        }
    }

    // 2. Reanudar descargas BitTorrent
    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let numeric_id = (t.id as i64) + TORRENT_ID_OFFSET;
            if match_torrent_generic(numeric_id, &t.info_hash, ids_val) {
                let _ = engine.resume(t.id).await;
            }
        }
    }

    TransmissionResponse::success(json!({}), tag)
}

async fn handle_torrent_stop(
    state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    let ids_val = &args["ids"];

    // 1. Pausar descargas eD2k
    let mut dl = state.downloads.lock().await;
    let all = dl.list_download_details();
    for d in all {
        if match_torrent_id(&d, ids_val) {
            let _ = dl.pause_download(&d.snapshot.id);
        }
    }

    // 2. Pausar descargas BitTorrent
    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let numeric_id = (t.id as i64) + TORRENT_ID_OFFSET;
            if match_torrent_generic(numeric_id, &t.info_hash, ids_val) {
                let _ = engine.pause(t.id).await;
            }
        }
    }

    TransmissionResponse::success(json!({}), tag)
}

async fn handle_torrent_remove(
    state: &AppState,
    args: Value,
    tag: Option<Value>,
) -> TransmissionResponse {
    let ids_val = &args["ids"];
    let delete_local_data = args["delete-local-data"].as_bool().unwrap_or(false);

    // 1. Eliminar descargas eD2k
    let mut dl = state.downloads.lock().await;
    let all = dl.list_download_details();
    for d in all {
        if match_torrent_id(&d, ids_val) {
            let _ = dl.remove_download(&d.snapshot.id, delete_local_data);
        }
    }

    // 2. Eliminar descargas BitTorrent
    if let Some(engine) = state.torrent.get() {
        for t in engine.list() {
            let numeric_id = (t.id as i64) + TORRENT_ID_OFFSET;
            if match_torrent_generic(numeric_id, &t.info_hash, ids_val) {
                let _ = engine.delete(t.id, delete_local_data).await;
            }
        }
    }

    TransmissionResponse::success(json!({}), tag)
}

fn match_torrent_generic(id: i64, hash: &str, ids_val: &Value) -> bool {
    let hash_lower = hash.to_lowercase();
    if let Some(arr) = ids_val.as_array() {
        arr.iter().any(|v| {
            if let Some(n) = v.as_i64() {
                n == id
            } else if let Some(s) = v.as_str() {
                s.eq_ignore_ascii_case(&hash_lower)
            } else {
                false
            }
        })
    } else if let Some(n) = ids_val.as_i64() {
        n == id
    } else if let Some(s) = ids_val.as_str() {
        s.eq_ignore_ascii_case(&hash_lower)
    } else {
        true
    }
}

fn match_torrent_id(
    d: &crate::services::downloads::DownloadDetail,
    ids_val: &Value,
) -> bool {
    let numeric_id = uuid_to_transmission_id(&d.snapshot.id);
    let hash_lower = d.snapshot.ed2k_hash.to_lowercase();
    let uuid_str = d.snapshot.id.to_string();

    if let Some(arr) = ids_val.as_array() {
        arr.iter().any(|v| {
            if let Some(n) = v.as_i64() {
                n == numeric_id
            } else if let Some(s) = v.as_str() {
                s.eq_ignore_ascii_case(&hash_lower) || s.eq_ignore_ascii_case(&uuid_str)
            } else {
                false
            }
        })
    } else if let Some(n) = ids_val.as_i64() {
        n == numeric_id
    } else if let Some(s) = ids_val.as_str() {
        s.eq_ignore_ascii_case(&hash_lower) || s.eq_ignore_ascii_case(&uuid_str)
    } else {
        // Si se omite ids, Transmission opera sobre todos los torrents
        true
    }
}