// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use futures_util::future::join_all;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::services::downloads::now_ms;
use crate::services::indexer_aggregator::IndexerAggregator;
use crate::services::media_probe::probe_video_file;
use crate::services::organizer::detect_quality_tag;
use crate::services::scorer::{
    apply_strict_castilian_filter, castilian_confidence_level, evaluate_release,
    extract_tv_episode_numbers, matches_release_with_alts, matches_release_with_options,
    matches_season_pack_release,
    spanish_title_search_variants, strip_trailing_year_suffix, ScoredRelease,
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
        let _pass_guard = autograb_pass_mutex().lock().await;
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
        let _pass_guard = autograb_pass_mutex().lock().await;
        let mut ag_state = Self::load_state(state).await;

        // 1. Autorreparación de descargas bloqueadas (detección de estancamiento + lista negra + conmutación por error)
        let healed_downloads = Self::inspect_and_heal_stalled_downloads(state, &mut ag_state).await;
        if !healed_downloads.is_empty() {
            Self::save_state(state, &ag_state).await;
        }

        let mut media_items = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .db()
                .list_media_items(None, None)
                .unwrap_or_default()
        };

        // 2. Cola de prioridad: procesar primero los elementos recién añadidos (0 fallos)
        media_items.sort_by_key(|item| {
            let empty_count = ag_state
                .diagnostics
                .get(&item.id)
                .map(|d| d.consecutive_empty_searches)
                .unwrap_or(0);
            let is_wanted_priority = if item.status == "wanted" { 0 } else { 1 };
            (is_wanted_priority, empty_count, std::cmp::Reverse(item.updated_at))
        });

        let mut grabbed_movies = Vec::new();
        let mut grabbed_episodes = Vec::new();

        for item in media_items {
            if !item.monitored {
                continue;
            }

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

        json!({
            "grabbedMovies": grabbed_movies,
            "grabbedEpisodes": grabbed_episodes,
            "healedDownloads": healed_downloads
        })
    }
}
