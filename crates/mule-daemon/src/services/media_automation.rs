// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::info;

use crate::services::{AutoGrabber, IndexerAggregator, MediaCleaner, MediaOrganizer, SubtitleService};
use crate::state::AppState;

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
            let mut cycle_counter: u32 = 0;

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let is_deep_scan_cycle = cycle_counter.is_multiple_of(6);
                        cycle_counter = cycle_counter.wrapping_add(1);

                        info!(
                            "[MEDIA_AUTOMATION] Running autonomous media maintenance cycle (deep_scan={})...",
                            is_deep_scan_cycle
                        );

                        // 1. Organizar descargas completadas en la biblioteca (enlaces duros / carpetas de Jellyfin)
                        let org_res = MediaOrganizer::sweep_and_organize(&state).await;
                        if let Some(arr) = org_res["organized"].as_array() {
                            if !arr.is_empty() {
                                info!("[MEDIA_AUTOMATION] Organized {} completed items", arr.len());
                            }
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

                        // 3. Obtener subtítulos en español faltantes mediante el motor Bazarr / OpenSubtitles
                        let sub_res = SubtitleService::auto_fetch_all_missing(&state, "spa").await;
                        if let Some(arr) = sub_res["downloaded"].as_array() {
                            if !arr.is_empty() {
                                info!("[MEDIA_AUTOMATION] Downloaded {} missing subtitles", arr.len());
                            }
                        }

                        // 4. Capturar automáticamente películas deseadas y episodios emitidos
                        let aggregator = IndexerAggregator::new();
                        let grab_res = AutoGrabber::run_auto_grab_round(&state, &aggregator, false).await;
                        let movies = grab_res["grabbedMovies"].as_array().map(|a| a.len()).unwrap_or(0);
                        let eps = grab_res["grabbedEpisodes"].as_array().map(|a| a.len()).unwrap_or(0);
                        if movies > 0 || eps > 0 {
                            info!("[MEDIA_AUTOMATION] Auto-grabbed {} movies and {} episodes", movies, eps);
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
                            // Obtener automáticamente subtítulos en español faltantes para medios recién vinculados
                            let _ = SubtitleService::auto_fetch_all_missing(&state, "spa").await;
                        }
                    }
                }
            }
        })
    }
}
