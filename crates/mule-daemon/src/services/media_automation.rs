// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::path::Path;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::info;

use crate::services::notifications::{
    NotificationCategory, NotificationEventInput, NotificationKind, NotificationSeverity,
};
use crate::services::{AutoGrabber, IndexerAggregator, MediaCleaner, MediaOrganizer, SubtitleService};
use crate::state::AppState;

/// Número máximo de títulos incluidos de forma explícita en el resumen de biblioteca lista.
const MAX_MEDIA_SUMMARY_TITLES: usize = 3;
/// Cadencia de comprobación periódica de AutoGrabber (autocuración + rellenado de cola): 20 minutos.
const AUTO_GRAB_TICK_SECS: u64 = 1200;

fn extract_organized_titles(org_res: &serde_json::Value) -> Vec<String> {
    org_res["organized"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("target").and_then(|v| v.as_str()))
                .filter_map(|target| Path::new(target).file_stem().and_then(|s| s.to_str()))
                .map(mule_format::fix_mojibake)
                .collect()
        })
        .unwrap_or_default()
}

async fn emit_media_ready_notification(
    state: &AppState,
    org_res: &serde_json::Value,
    sub_res: &serde_json::Value,
) {
    let titles = extract_organized_titles(org_res);
    if titles.is_empty() {
        return;
    }

    let sub_count = sub_res["downloaded"]
        .as_array()
        .map(|arr| arr.len())
        .unwrap_or(0);
    let sub_suffix = if sub_count > 0 {
        format!(" · {sub_count} subtítulo(s) ES descargado(s)")
    } else {
        " · Biblioteca actualizada".to_string()
    };

    let (title, body) = if titles.len() == 1 {
        ("Listo para ver".to_string(), format!("{}{sub_suffix}", titles[0]))
    } else {
        let preview: Vec<&str> = titles
            .iter()
            .take(MAX_MEDIA_SUMMARY_TITLES)
            .map(String::as_str)
            .collect();
        let extra = titles.len().saturating_sub(MAX_MEDIA_SUMMARY_TITLES);
        let list_text = if extra > 0 {
            format!("{} y {extra} más", preview.join(", "))
        } else {
            preview.join(", ")
        };
        (
            format!("{} títulos listos en tu biblioteca", titles.len()),
            format!("{list_text}{sub_suffix}"),
        )
    };

    let _ = state
        .notifications
        .notify(
            state,
            NotificationEventInput {
                kind: NotificationKind::MediaReady,
                severity: NotificationSeverity::Success,
                category: NotificationCategory::Media,
                title,
                body,
                metadata: Some(serde_json::json!({
                    "organizedCount": titles.len(),
                    "subtitlesCount": sub_count,
                })),
                dedupe_key: None,
                cooldown_secs: 0,
            },
        )
        .await;
}

async fn emit_media_grabbed_notification(state: &AppState, grab_res: &serde_json::Value) {
    let movies = grab_res["grabbedMovies"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let eps = grab_res["grabbedEpisodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    if movies == 0 && eps == 0 {
        return;
    }

    let mut parts = Vec::new();
    if movies > 0 {
        parts.push(format!("{movies} película(s)"));
    }
    if eps > 0 {
        parts.push(format!("{eps} episodio(s)"));
    }

    let _ = state
        .notifications
        .notify(
            state,
            NotificationEventInput {
                kind: NotificationKind::MediaGrabbed,
                severity: NotificationSeverity::Info,
                category: NotificationCategory::Media,
                title: "Auto-captura multimedia iniciada".to_string(),
                body: format!("Se ha iniciado la descarga de {}.", parts.join(" y ")),
                metadata: Some(serde_json::json!({
                    "movies": movies,
                    "episodes": eps,
                })),
                dedupe_key: None,
                cooldown_secs: 0,
            },
        )
        .await;
}

pub struct MediaAutomation;

impl MediaAutomation {
    /// Inicia el bucle autónomo en segundo plano para la gestión multimedia
    pub fn start(state: AppState) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("[MEDIA_AUTOMATION] Autonomous background media service started (cadence: 1 hour + reactive on download completion)");

            let mut event_rx = state.event_tx.subscribe();

            // Retardo inicial de 45 segundos tras el arranque para permitir que el enjambre, las descargas y el RPC se estabilicen
            tokio::time::sleep(Duration::from_secs(45)).await;

            let mut ticker = tokio::time::interval(Duration::from_secs(3600));
            let mut autograb_ticker = tokio::time::interval(Duration::from_secs(AUTO_GRAB_TICK_SECS));
            autograb_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut cycle_counter: u32 = 0;

            loop {
                tokio::select! {
                    _ = autograb_ticker.tick() => {
                        let aggregator = IndexerAggregator::new();
                        let grab_res = AutoGrabber::run_auto_grab_round(&state, &aggregator, false).await;
                        let healed = grab_res["healedDownloads"].as_array().map(|a| a.len()).unwrap_or(0);
                        let movies = grab_res["grabbedMovies"].as_array().map(|a| a.len()).unwrap_or(0);
                        let eps = grab_res["grabbedEpisodes"].as_array().map(|a| a.len()).unwrap_or(0);
                        if healed > 0 || movies > 0 || eps > 0 {
                            info!(
                                "[MEDIA_AUTOMATION] AutoGrab tick: healed={}, grabbedMovies={}, grabbedEpisodes={}",
                                healed, movies, eps
                            );
                            if movies > 0 || eps > 0 {
                                emit_media_grabbed_notification(&state, &grab_res).await;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        let is_deep_scan_cycle = cycle_counter.is_multiple_of(6);
                        cycle_counter = cycle_counter.wrapping_add(1);

                        info!(
                            "[MEDIA_AUTOMATION] Running autonomous media maintenance cycle (deep_scan={})...",
                            is_deep_scan_cycle
                        );

                        // 1. Organizar descargas completadas en la biblioteca (si no está en Modo Simple)
                        let org_res = if crate::services::organizer::is_organize_enabled(&state).await {
                            MediaOrganizer::sweep_and_organize(&state).await
                        } else {
                            serde_json::json!({ "organized": [], "skipped": "simple_mode" })
                        };
                        let organized_count = org_res["organized"].as_array().map(|a| a.len()).unwrap_or(0);
                        if organized_count > 0 {
                            info!("[MEDIA_AUTOMATION] Organized {} completed items", organized_count);
                        }

                        // 2. Escaneo profundo y limpieza del NAS solo cada 6 horas o al arrancar (ahorra E/S de red y RAM del NAS)
                        if is_deep_scan_cycle {
                            let scan_res = crate::services::DiskScanner::scan_all(&state).await;
                            info!("[MEDIA_AUTOMATION] Deep NAS disk scan pass completed: {}", scan_res);

                            let clean_res = MediaCleaner::clean_all(&state, false).await;
                            if let Some(summary) = clean_res["summary"].as_str() {
                                info!("[MEDIA_AUTOMATION] Deep cleanup cycle: {}", summary);
                            }
                        }

                        // 3. Obtener subtítulos faltantes mediante el motor nativo de OpenSubtitles si está habilitado
                        let sub_cfg = SubtitleService::load_runtime_config(&state).await;
                        let sub_res = if sub_cfg.enabled {
                            SubtitleService::auto_fetch_all_missing(&state, &sub_cfg.language).await
                        } else {
                            serde_json::json!({ "downloaded": [], "skipped": "disabled" })
                        };
                        if let Some(arr) = sub_res["downloaded"].as_array() {
                            if !arr.is_empty() {
                                info!("[MEDIA_AUTOMATION] Downloaded {} missing subtitles", arr.len());
                            }
                        }

                        if organized_count > 0 {
                            emit_media_ready_notification(&state, &org_res, &sub_res).await;
                        }

                        // 4. Capturar automáticamente películas deseadas y episodios emitidos
                        let aggregator = IndexerAggregator::new();
                        let grab_res = AutoGrabber::run_auto_grab_round(&state, &aggregator, false).await;
                        let movies = grab_res["grabbedMovies"].as_array().map(|a| a.len()).unwrap_or(0);
                        let eps = grab_res["grabbedEpisodes"].as_array().map(|a| a.len()).unwrap_or(0);
                        if movies > 0 || eps > 0 {
                            info!("[MEDIA_AUTOMATION] Auto-grabbed {} movies and {} episodes", movies, eps);
                            emit_media_grabbed_notification(&state, &grab_res).await;
                        }
                    }
                    Ok(event_str) = event_rx.recv() => {
                        // Comprobar si el evento corresponde a una descarga completada
                        if event_str.contains("\"DownloadCompleted\"") || event_str.contains("\"downloadCompleted\"") {
                            info!("[MEDIA_AUTOMATION] Reactive trigger: download completed, linking to library immediately...");
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            let org_res = MediaOrganizer::sweep_and_organize(&state).await;
                            if let Some(arr) = org_res["organized"].as_array() {
                                if !arr.is_empty() {
                                    info!("[MEDIA_AUTOMATION] Reactively organized {} completed items into library", arr.len());
                                }
                            }
                            // Limpiar duplicados y residuos de forma reactiva
                            let _ = MediaCleaner::clean_all(&state, false).await;
                            // Obtener automáticamente subtítulos faltantes para medios recién vinculados
                            let sub_cfg = SubtitleService::load_runtime_config(&state).await;
                            let sub_res = if sub_cfg.enabled {
                                SubtitleService::auto_fetch_all_missing(&state, &sub_cfg.language).await
                            } else {
                                serde_json::json!({ "downloaded": [], "skipped": "disabled" })
                            };
                            emit_media_ready_notification(&state, &org_res, &sub_res).await;

                            // Disparar inmediatamente una pasada de AutoGrabber en segundo plano para rellenar la cola
                            // con los siguientes episodios pendientes de la serie (flujo continuo sin esperar al temporizador).
                            let state_grab = state.clone();
                            tokio::spawn(async move {
                                let aggregator = IndexerAggregator::new();
                                let grab_res =
                                    AutoGrabber::run_auto_grab_round(&state_grab, &aggregator, false).await;
                                let movies = grab_res["grabbedMovies"]
                                    .as_array()
                                    .map(|a| a.len())
                                    .unwrap_or(0);
                                let eps = grab_res["grabbedEpisodes"]
                                    .as_array()
                                    .map(|a| a.len())
                                    .unwrap_or(0);
                                if movies > 0 || eps > 0 {
                                    info!(
                                        "[MEDIA_AUTOMATION] Reactive auto-grabbed {} movies and {} episodes after completion",
                                        movies, eps
                                    );
                                    emit_media_grabbed_notification(&state_grab, &grab_res).await;
                                }
                            });
                        }
                    }
                }
            }
        })
    }
}
