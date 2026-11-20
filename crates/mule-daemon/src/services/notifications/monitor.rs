// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Monitor reactivo que traduce eventos del bus del demonio, finalizaciones torrent y salud de disco en notificaciones.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::state::AppState;

use super::constants::{
    BYTES_PER_GIB, DISK_CHECK_INTERVAL_SECS, DISK_INITIAL_DELAY_SECS, DISK_LOW_COOLDOWN_SECS,
    SERVER_DISCONNECT_COOLDOWN_SECS, TORRENT_POLL_INTERVAL_SECS,
};
use super::types::{
    format_bytes_human, NotificationCategory, NotificationEventInput, NotificationKind,
    NotificationSeverity,
};

/// Inicia la tarea en segundo plano encargada de generar notificaciones automáticas.
pub fn start_notification_monitor(state: AppState) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("[NOTIFICATIONS] Monitor reactivo de notificaciones iniciado");
        let mut event_rx = state.event_tx.subscribe();
        let mut was_connected = false;
        let mut last_server_label = "el servidor eD2K".to_string();

        let mut torrents_initialized = false;
        let mut seen_completed_torrents: HashSet<String> = HashSet::new();

        let mut torrent_ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(TORRENT_POLL_INTERVAL_SECS),
            Duration::from_secs(TORRENT_POLL_INTERVAL_SECS),
        );
        torrent_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut disk_ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(DISK_INITIAL_DELAY_SECS),
            Duration::from_secs(DISK_CHECK_INTERVAL_SECS),
        );
        disk_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                recv_res = event_rx.recv() => {
                    match recv_res {
                        Ok(raw) => {
                            handle_bus_frame(
                                &state,
                                &raw,
                                &mut was_connected,
                                &mut last_server_label,
                            ).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            debug!(skipped, "[NOTIFICATIONS] Eventos perdidos por retraso en el canal broadcast");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = torrent_ticker.tick() => {
                    check_completed_torrents(
                        &state,
                        &mut torrents_initialized,
                        &mut seen_completed_torrents,
                    ).await;
                }
                _ = disk_ticker.tick() => {
                    check_disk_space_alerts(&state).await;
                }
            }
        }
    })
}

async fn handle_bus_frame(
    state: &AppState,
    raw: &str,
    was_connected: &mut bool,
    last_server_label: &mut String,
) {
    if !raw.contains("\"event.downloadCompleted\"")
        && !raw.contains("\"event.downloadFailed\"")
        && !raw.contains("\"server.sessionUpdated\"")
    {
        return;
    }

    let Ok(frame) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    let Some(method) = frame.get("method").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(params) = frame.get("params") else {
        return;
    };

    match method {
        "event.downloadCompleted" => {
            let Some(dl) = params.get("download") else {
                return;
            };
            let raw_name = dl
                .get("fileName")
                .and_then(|v| v.as_str())
                .unwrap_or("Archivo")
                .trim();
            let clean_name = mule_format::fix_mojibake(raw_name);
            let total_bytes = dl.get("totalBytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let dl_id = dl.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let body = if total_bytes > 0 {
                format!("{clean_name} ({}) · eD2K", format_bytes_human(total_bytes))
            } else {
                format!("{clean_name} · eD2K")
            };

            let _ = state
                .notifications
                .notify(
                    state,
                    NotificationEventInput {
                        kind: NotificationKind::DownloadCompleted,
                        severity: NotificationSeverity::Success,
                        category: NotificationCategory::Downloads,
                        title: "Descarga completada".to_string(),
                        body,
                        metadata: Some(serde_json::json!({
                            "downloadId": dl_id,
                            "protocol": "ed2k",
                            "totalBytes": total_bytes,
                        })),
                        dedupe_key: (!dl_id.is_empty()).then(|| format!("dl_done:{dl_id}")),
                        cooldown_secs: DISK_LOW_COOLDOWN_SECS,
                    },
                )
                .await;
        }
        "event.downloadFailed" => {
            let Some(dl) = params.get("download") else {
                return;
            };
            let raw_name = dl
                .get("fileName")
                .and_then(|v| v.as_str())
                .unwrap_or("Archivo")
                .trim();
            let clean_name = mule_format::fix_mojibake(raw_name);
            let reason = params
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("error de verificación o transferencia");
            let dl_id = dl.get("id").and_then(|v| v.as_str()).unwrap_or("");

            let _ = state
                .notifications
                .notify(
                    state,
                    NotificationEventInput {
                        kind: NotificationKind::DownloadFailed,
                        severity: NotificationSeverity::Error,
                        category: NotificationCategory::Downloads,
                        title: "Error en descarga".to_string(),
                        body: format!("{clean_name}: {reason}"),
                        metadata: Some(serde_json::json!({
                            "downloadId": dl_id,
                            "protocol": "ed2k",
                            "reason": reason,
                        })),
                        dedupe_key: (!dl_id.is_empty()).then(|| format!("dl_fail:{dl_id}")),
                        cooldown_secs: SERVER_DISCONNECT_COOLDOWN_SECS,
                    },
                )
                .await;
        }
        "server.sessionUpdated" => {
            let Some(session) = params.get("session") else {
                return;
            };
            let Some(status) = session.get("status").and_then(|v| v.as_str()) else {
                return;
            };
            match status {
                "connected" => {
                    *was_connected = true;
                    let name = session
                        .get("serverName")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    let addr = session
                        .get("serverAddress")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    *last_server_label = name
                        .or(addr)
                        .unwrap_or("el servidor eD2K")
                        .to_string();
                }
                "disconnected" if *was_connected => {
                    *was_connected = false;
                    let _ = state
                        .notifications
                        .notify(
                            state,
                            NotificationEventInput {
                                kind: NotificationKind::ServerDisconnected,
                                severity: NotificationSeverity::Warning,
                                category: NotificationCategory::Network,
                                title: "Conexión eD2K perdida".to_string(),
                                body: format!("Se ha desconectado de {last_server_label}."),
                                metadata: Some(serde_json::json!({
                                    "server": last_server_label,
                                })),
                                dedupe_key: Some("server_disconnected".to_string()),
                                cooldown_secs: SERVER_DISCONNECT_COOLDOWN_SECS,
                            },
                        )
                        .await;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

async fn check_completed_torrents(
    state: &AppState,
    initialized: &mut bool,
    seen_hashes: &mut HashSet<String>,
) {
    let Some(engine) = state.torrent.get() else {
        return;
    };
    let list = engine.list();

    if !*initialized {
        for tor in list {
            if tor.status == "completed" {
                seen_hashes.insert(tor.info_hash);
            }
        }
        *initialized = true;
        return;
    }

    for tor in list {
        if tor.status == "completed" && seen_hashes.insert(tor.info_hash.clone()) {
            let clean_name = mule_format::fix_mojibake(&tor.name);
            let body = if tor.total_bytes > 0 {
                format!(
                    "{clean_name} ({}) · BitTorrent",
                    format_bytes_human(tor.total_bytes)
                )
            } else {
                format!("{clean_name} · BitTorrent")
            };

            if let Err(err) = state
                .notifications
                .notify(
                    state,
                    NotificationEventInput {
                        kind: NotificationKind::DownloadCompleted,
                        severity: NotificationSeverity::Success,
                        category: NotificationCategory::Downloads,
                        title: "Descarga completada".to_string(),
                        body,
                        metadata: Some(serde_json::json!({
                            "infoHash": tor.info_hash,
                            "protocol": "torrent",
                            "totalBytes": tor.total_bytes,
                        })),
                        dedupe_key: Some(format!("tor_done:{}", tor.info_hash)),
                        cooldown_secs: DISK_LOW_COOLDOWN_SECS,
                    },
                )
                .await
            {
                warn!("[NOTIFICATIONS] Error notificando torrent completado: {err}");
            }
        }
    }
}

async fn probe_available_space(path: PathBuf) -> Option<u64> {
    tokio::task::spawn_blocking(move || {
        if !path.exists() {
            return None;
        }
        crate::services::nfs_mount::verify_destination_mount_ready(&path, false).ok()?;
        fs4::available_space(&path).ok()
    })
    .await
    .ok()
    .flatten()
}

async fn check_disk_space_alerts(state: &AppState) {
    let threshold_gb = state.notifications.current_disk_low_threshold_gb().await;
    let threshold_bytes = threshold_gb.saturating_mul(BYTES_PER_GIB);
    let dl_dir = state.effective_download_dir().await;

    if let Some(free_bytes) = probe_available_space(dl_dir.clone()).await {
        if free_bytes < threshold_bytes {
            let _ = state
                .notifications
                .notify(
                    state,
                    NotificationEventInput {
                        kind: NotificationKind::DiskLow,
                        severity: NotificationSeverity::Warning,
                        category: NotificationCategory::System,
                        title: "Poco espacio en disco local".to_string(),
                        body: format!(
                            "Quedan {} libres en {} (umbral: {} GiB).",
                            format_bytes_human(free_bytes),
                            dl_dir.display(),
                            threshold_gb
                        ),
                        metadata: Some(serde_json::json!({
                            "volume": "incoming",
                            "path": dl_dir.to_string_lossy(),
                            "freeBytes": free_bytes,
                            "thresholdGb": threshold_gb,
                        })),
                        dedupe_key: Some("disk_low:incoming".to_string()),
                        cooldown_secs: DISK_LOW_COOLDOWN_SECS,
                    },
                )
                .await;
        }
    }

    let nas_dir = state.effective_nas_root_dir().await;
    if nas_dir != dl_dir && !nas_dir.starts_with(&dl_dir) {
        if let Some(nas_free) = probe_available_space(nas_dir.clone()).await {
            if nas_free < threshold_bytes {
                let _ = state
                    .notifications
                    .notify(
                        state,
                        NotificationEventInput {
                            kind: NotificationKind::DiskLow,
                            severity: NotificationSeverity::Warning,
                            category: NotificationCategory::System,
                            title: "Poco espacio en NAS".to_string(),
                            body: format!(
                                "Quedan {} libres en {} (umbral: {} GiB).",
                                format_bytes_human(nas_free),
                                nas_dir.display(),
                                threshold_gb
                            ),
                            metadata: Some(serde_json::json!({
                                "volume": "nas",
                                "path": nas_dir.to_string_lossy(),
                                "freeBytes": nas_free,
                                "thresholdGb": threshold_gb,
                            })),
                            dedupe_key: Some("disk_low:nas".to_string()),
                            cooldown_secs: DISK_LOW_COOLDOWN_SECS,
                        },
                    )
                    .await;
            }
        }
    }
}
