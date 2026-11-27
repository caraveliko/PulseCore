// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use futures_util::future::join_all;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::services::downloads::now_ms;
use crate::services::events::DaemonEvent;
use crate::services::indexer_aggregator::IndexerAggregator;
use crate::services::media_probe::probe_video_file;
use crate::services::organizer::detect_quality_tag;
use crate::services::scorer::{
    apply_strict_castilian_filter, castilian_confidence_level, evaluate_release,
    extract_tv_episode_numbers, is_single_video_file_ed2k, matches_release_with_alts,
    matches_release_with_options, matches_season_pack_release, spanish_title_search_variants,
    strip_trailing_year_suffix, ScoredRelease,
};
use crate::state::AppState;
use mule_core::model::{EpisodeItem, MediaItem};
use mule_core::storage::AllocationPolicy;

const AUTOGRABBER_STATE_KEY: &str = "autograbber_state_v1";

/// Bloqueo global para evitar que dos pasadas de AutoGrabber (planificador en segundo plano y RPC manual)
/// se ejecuten simultáneamente y sobrescriban `autograbber_state_v1`.
static AUTOGRAB_PASS_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn autograb_pass_mutex() -> &'static tokio::sync::Mutex<()> {
    AUTOGRAB_PASS_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Contador de peticiones manuales de AutoGrab esperando turno frente a la ronda en segundo plano.
static MANUAL_AUTOGRAB_WAITERS: AtomicUsize = AtomicUsize::new(0);

struct ManualWaiterGuard;

impl ManualWaiterGuard {
    fn new() -> Self {
        MANUAL_AUTOGRAB_WAITERS.fetch_add(1, Ordering::Release);
        Self
    }
}

impl Drop for ManualWaiterGuard {
    fn drop(&mut self) {
        MANUAL_AUTOGRAB_WAITERS.fetch_sub(1, Ordering::Release);
    }
}

/// Plazos de detección de bloqueo (detección de estancamiento / Stall Detection) por protocolo.
/// Ventana sin fuentes ni bytes en eD2K antes de considerar la descarga estancada (45 min).
pub(super) const STALL_ED2K_ZERO_SOURCES_MS: i64 = 45 * 60 * 1000;
/// Ventana máxima a 0 bytes en eD2K sin cola remota (`QR`) activa antes de cambiar de versión (4 h).
pub(super) const STALL_ED2K_ZERO_BYTES_MAX_MS: i64 = 4 * 3600 * 1000;
/// Ventana sin pares ni bytes en BitTorrent antes de considerar la descarga estancada (15 min).
pub(super) const STALL_BT_ZERO_SOURCES_MS: i64 = 15 * 60 * 1000;
/// Ventana máxima a 0 bytes en BitTorrent antes de cambiar de versión (45 min).
pub(super) const STALL_BT_ZERO_BYTES_MAX_MS: i64 = 45 * 60 * 1000;
/// Ventana sin avance ni fuentes para una descarga parcial antes de descartarla (12 h).
pub(super) const STALL_PARTIAL_NO_PROGRESS_MS: i64 = 12 * 3600 * 1000;
/// Tiempo de vida de las entradas temporales en lista negra por estancamiento/falta de fuentes (24 h).
pub(super) const STALL_BLACKLIST_TTL_MS: i64 = 24 * 3600 * 1000;

/// Intervalo entre dos comprobaciones de mejora de calidad/idioma (12 h).
pub(super) const UPGRADE_CHECK_INTERVAL_MS: i64 = 12 * 3600 * 1000;
/// Ganancia mínima de puntuación requerida para activar una mejora automática.
pub(super) const MIN_UPGRADE_SCORE_DELTA: i32 = 35;
/// Número mínimo de episodios faltantes en una misma temporada para buscar primero un paquete de temporada.
pub(super) const MIN_WANTED_EPS_FOR_SEASON_PACK: usize = 3;
/// Puntuación mínima requerida para aceptar un paquete de temporada.
pub(super) const MIN_SEASON_PACK_SCORE: i32 = 25;
/// Motivo normalizado registrado en lista negra cuando el audio no está en Castellano de España.
pub const WRONG_LANGUAGE_REASON: &str = "wrong_language_not_castilian";
/// Prefijo para entradas de lista negra basadas en título normalizado sin hash conocido.
pub(super) const TITLE_BLACKLIST_PREFIX: &str = "title:";
/// Puntuación mínima exigida para seleccionar automáticamente un reemplazo en Castellano estricto.
pub(super) const MIN_STRICT_REPLACEMENT_SCORE: i32 = 15;

/// Intervalos de retroceso exponencial (en ms) cuando no se encuentra ninguna versión válida.
pub(super) const BACKOFF_STEPS_MS: &[i64] = &[
    20 * 60 * 1000,      // 1.er fallo: 20 min
    60 * 60 * 1000,      // 2.º fallo: 1 h
    2 * 3600 * 1000,     // 3.er fallo: 2 h
    4 * 3600 * 1000,     // 4.º+ fallo: 4 h
];


mod dispatch;
mod failover;
mod movies;
mod series;
mod state;
#[cfg(test)]
mod tests;

pub use state::{ActiveGrabTracker, AutoGrabberState, BlacklistedEntry, GrabDiagnostic};

/// Motor de captura automática y recuperación de descargas multimedia.
pub struct AutoGrabber;

/// Tiempo máximo de espera para cargar metadatos extendidos de una película/serie durante el Auto-Grab (6 s).
pub(super) const AUTOGRAB_METADATA_TIMEOUT: Duration = Duration::from_secs(6);

/// Carga `MediaExtendedDetails` acotado por `AUTOGRAB_METADATA_TIMEOUT` para evitar bloqueos por APIs externas.
pub(super) async fn load_extended_details_bounded(
    state: &AppState,
    item: &MediaItem,
) -> crate::services::media_metadata::MediaExtendedDetails {
    match tokio::time::timeout(
        AUTOGRAB_METADATA_TIMEOUT,
        state.media_metadata.get_extended_details(item),
    )
    .await
    {
        Ok(details) => details,
        Err(_) => {
            tracing::debug!(
                media_id = %item.id,
                title = %item.title,
                "Tiempo agotado cargando metadatos extendidos en AutoGrabber; usando metadatos locales"
            );
            crate::services::media_metadata::MediaExtendedDetails::default()
        }
    }
}

/// Emite un evento WebSocket `autograb.progress` en tiempo real para el banner visual de Cine y Series.
pub(super) fn emit_autograb_progress(
    state: &AppState,
    stage: &str,
    title: &str,
    detail: &str,
    percent: u8,
    metrics: Option<Value>,
) {
    let title_owned = title.to_string();
    let detail_owned = detail.to_string();
    let stage_owned = stage.to_string();
    DaemonEvent::Custom {
        method: "autograb.progress".to_string(),
        payload: json!({
            "stage": stage_owned,
            "title": title_owned,
            "detail": detail_owned,
            "percent": percent.min(100),
            "metrics": metrics,
            "timestampMs": now_ms(),
        }),
    }
    .send(&state.event_tx);
}

/// Resume cuántos lanzamientos provienen de cada red (eD2K Servidor, Kad, BT Público, DHT Local, Torznab).
pub(super) fn summarize_release_sources(releases: &[ScoredRelease]) -> Value {
    let mut ed2k_server = 0usize;
    let mut ed2k_kad = 0usize;
    let mut bt_public = 0usize;
    let mut bt_dht_local = 0usize;
    let mut torznab = 0usize;

    for r in releases {
        let src = r.indexer_name.as_str();
        if src.contains("Server") {
            ed2k_server += 1;
        } else if src.contains("Kad") {
            ed2k_kad += 1;
        } else if src.starts_with("BT/apibay") || src.starts_with("BT/yts") {
            bt_public += 1;
        } else if src == "BT/DHT-Local" {
            bt_dht_local += 1;
        } else if r.protocol == "torrent" {
            torznab += 1;
        }
    }

    json!({
        "ed2kServer": ed2k_server,
        "ed2kKad": ed2k_kad,
        "btPublic": bt_public,
        "btDhtLocal": bt_dht_local,
        "torznab": torznab,
    })
}

impl AutoGrabber {
    pub async fn load_state(state: &AppState) -> AutoGrabberState {
        let mut ag_state = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .get_setting(AUTOGRABBER_STATE_KEY)
                .ok()
                .flatten()
                .and_then(|raw| serde_json::from_str::<AutoGrabberState>(&raw).ok())
                .unwrap_or_default()
        };
        ag_state.prune_expired_blacklists(now_ms(), STALL_BLACKLIST_TTL_MS);
        ag_state
    }

    pub async fn save_state(state: &AppState, ag_state: &AutoGrabberState) {
        if let Ok(raw) = serde_json::to_string(ag_state) {
            let mut dl_svc = state.downloads.lock().await;
            let _ = dl_svc.set_setting(AUTOGRABBER_STATE_KEY, &raw);
        }
    }

    /// Devuelve el informe de diagnóstico completo para una ficha multimedia (película o serie).
    pub async fn get_diagnostics_for_media(
        state: &AppState,
        media_id: &str,
        episodes: &[EpisodeItem],
    ) -> Value {
        let ag_state = Self::load_state(state).await;
        let item_diag = ag_state.diagnostics.get(media_id).cloned();
        let item_blacklist = ag_state
            .blacklisted_hashes
            .get(media_id)
            .cloned()
            .unwrap_or_default();

        let mut ep_diagnostics = HashMap::new();
        let mut total_ep_blacklisted = 0usize;
        for ep in episodes {
            if let Some(d) = ag_state.diagnostics.get(&ep.id) {
                ep_diagnostics.insert(ep.id.clone(), d.clone());
            }
            if let Some(bl) = ag_state.blacklisted_hashes.get(&ep.id) {
                total_ep_blacklisted += bl.len();
            }
        }

        json!({
            "diagnostic": item_diag,
            "blacklisted": item_blacklist,
            "totalBlacklisted": item_blacklist.len() + total_ep_blacklisted,
            "episodeDiagnostics": ep_diagnostics,
        })
    }

    /// Borra la lista negra y reinicia el retroceso para un medio (incluidos todos sus episodios si es serie) o un episodio concreto.
    pub async fn clear_blacklist(state: &AppState, target_id: &str) -> Value {
        let mut ag_state = Self::load_state(state).await;
        let episode_ids: Vec<String> = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .db()
                .get_episodes_for_media(target_id)
                .unwrap_or_default()
                .into_iter()
                .map(|ep| ep.id)
                .collect()
        };

        let mut removed = ag_state
            .blacklisted_hashes
            .remove(target_id)
            .map(|v| v.len())
            .unwrap_or(0);
        let mut reset_diagnostics = 0usize;
        if let Some(diag) = ag_state.diagnostics.get_mut(target_id) {
            diag.consecutive_empty_searches = 0;
            diag.next_eligible_search_ms = 0;
            reset_diagnostics += 1;
        }
        for ep_id in episode_ids {
            removed += ag_state
                .blacklisted_hashes
                .remove(&ep_id)
                .map(|v| v.len())
                .unwrap_or(0);
            if let Some(diag) = ag_state.diagnostics.get_mut(&ep_id) {
                diag.consecutive_empty_searches = 0;
                diag.next_eligible_search_ms = 0;
                reset_diagnostics += 1;
            }
        }
        Self::save_state(state, &ag_state).await;
        json!({
            "success": true,
            "clearedCount": removed,
            "resetDiagnosticsCount": reset_diagnostics
        })
    }

    /// Despacha la descarga al motor eMule o BitTorrent según el protocolo (película o episodio).
    pub async fn grab_for_media_id(
        state: &AppState,
        aggregator: &IndexerAggregator,
        media_id: &str,
        episode_id: Option<&str>,
        season_number: Option<u32>,
    ) -> Result<Value, String> {
        info!(
            "[AUTO_GRAB] Solicitud manual recibida para media_id={} (temporada={:?}, episodio={:?})",
            media_id, season_number, episode_id
        );
        let waiter = ManualWaiterGuard::new();
        let _pass_guard = match autograb_pass_mutex().try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                emit_autograb_progress(
                    state,
                    "queued",
                    "Auto-Grab manual",
                    "Esperando turno del planificador...",
                    5,
                    None,
                );
                autograb_pass_mutex().lock().await
            }
        };
        drop(waiter);

        aggregator.invalidate_cache().await;
        let mut ag_state = Self::load_state(state).await;
        let healed = Self::inspect_and_heal_stalled_downloads(state, &mut ag_state).await;

        let item = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .db()
                .get_media_item(media_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Media item '{media_id}' not found"))?
        };

        info!(
            "[AUTO_GRAB] Iniciando búsqueda manual para '{}' (tipo={}, perfil={})",
            item.title, item.media_type, item.quality_profile
        );

        let mut grabbed_movies = Vec::new();
        let mut grabbed_episodes = Vec::new();

        if item.media_type == "movie" {
            // Si la película está en 'wanted' o es elegible para una mejora (o forzada), iniciar la búsqueda
            if let Some(m) = Self::process_movie_item(state, aggregator, &item, &mut ag_state, true).await {
                grabbed_movies.push(m);
            }
        } else {
            grabbed_episodes = Self::process_series_item(
                state,
                aggregator,
                &item,
                episode_id,
                season_number,
                &mut ag_state,
                true,
            )
            .await;
        }

        info!(
            "[AUTO_GRAB] Búsqueda manual finalizada para '{}': {} película(s), {} episodio(s)/pack(s), {} reparada(s)",
            item.title,
            grabbed_movies.len(),
            grabbed_episodes.len(),
            healed.len()
        );
        emit_autograb_progress(
            state,
            "finished",
            &item.title,
            &format!(
                "{} película(s) y {} episodio(s)/pack(s) capturados",
                grabbed_movies.len(),
                grabbed_episodes.len()
            ),
            100,
            Some(json!({
                "grabbedMovies": grabbed_movies.len(),
                "grabbedEpisodes": grabbed_episodes.len(),
                "healedDownloads": healed.len(),
            })),
        );

        let diag = ag_state.diagnostics.get(media_id).cloned();
        Ok(json!({
            "grabbedMovies": grabbed_movies,
            "grabbedEpisodes": grabbed_episodes,
            "healedDownloads": healed,
            "diagnostic": diag
        }))
    }

    /// Pasada única sobre todos los elementos deseados/mejorables para reparar descargas estancadas, buscar y capturar automáticamente los mejores candidatos
    pub async fn run_auto_grab_round(
        state: &AppState,
        aggregator: &IndexerAggregator,
        force: bool,
    ) -> Value {
        let (healed_downloads, media_items) = {
            let _pass_guard = autograb_pass_mutex().lock().await;
            let mut ag_state = Self::load_state(state).await;

            // 1. Autorreparación de descargas bloqueadas (detección de estancamiento + lista negra + conmutación por error)
            let healed = Self::inspect_and_heal_stalled_downloads(state, &mut ag_state).await;
            if !healed.is_empty() {
                Self::save_state(state, &ag_state).await;
            }

            let mut items = {
                let dl_svc = state.downloads.lock().await;
                dl_svc
                    .db()
                    .list_media_items(None, None)
                    .unwrap_or_default()
            };

            // 2. Cola de prioridad: procesar primero los elementos recién añadidos (0 fallos)
            items.sort_by_key(|item| {
                let empty_count = ag_state
                    .diagnostics
                    .get(&item.id)
                    .map(|d| d.consecutive_empty_searches)
                    .unwrap_or(0);
                let is_wanted_priority = if item.status == "wanted" { 0 } else { 1 };
                (is_wanted_priority, empty_count, std::cmp::Reverse(item.updated_at))
            });

            (healed, items)
        };

        let mut grabbed_movies = Vec::new();
        let mut grabbed_episodes = Vec::new();

        for item in media_items {
            if !item.monitored {
                continue;
            }

            // Ceder prioridad inmediata si hay una solicitud manual de Auto-Grab esperando turno.
            if !force && MANUAL_AUTOGRAB_WAITERS.load(Ordering::Acquire) > 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            let _pass_guard = autograb_pass_mutex().lock().await;
            let mut ag_state = Self::load_state(state).await;

            if item.media_type == "movie" {
                if let Some(grabbed) =
                    Self::process_movie_item(state, aggregator, &item, &mut ag_state, force).await
                {
                    grabbed_movies.push(grabbed);
                }
            } else if item.media_type == "tv" {
                let mut eps = Self::process_series_item(
                    state,
                    aggregator,
                    &item,
                    None,
                    None,
                    &mut ag_state,
                    force,
                )
                .await;
                grabbed_episodes.append(&mut eps);
            }
        }

        emit_autograb_progress(
            state,
            "finished",
            "Auto-Grab finalizado",
            &format!(
                "{} película(s) y {} episodio(s) capturados",
                grabbed_movies.len(),
                grabbed_episodes.len()
            ),
            100,
            Some(json!({
                "grabbedMovies": grabbed_movies.len(),
                "grabbedEpisodes": grabbed_episodes.len(),
                "healedDownloads": healed_downloads.len(),
            })),
        );

        json!({
            "grabbedMovies": grabbed_movies,
            "grabbedEpisodes": grabbed_episodes,
            "healedDownloads": healed_downloads
        })
    }
}
