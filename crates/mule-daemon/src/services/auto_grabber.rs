// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::services::downloads::now_ms;
use crate::services::indexer_aggregator::IndexerAggregator;
use crate::services::media_probe::probe_video_file;
use crate::services::organizer::detect_quality_tag;
use crate::services::scorer::{
    evaluate_release, extract_tv_episode_numbers, matches_release, matches_release_with_alts,
    matches_season_pack_release, spanish_title_search_variants, ScoredRelease,
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

/// Plazos de detección de bloqueo (detección de estancamiento / Stall Detection).
const STALL_ZERO_SOURCES_MS: i64 = 45 * 60 * 1000; // 45 min a 0 bytes y 0 fuentes
const STALL_ZERO_BYTES_MAX_MS: i64 = 3 * 3600 * 1000; // 3 h a 0 bytes incluso con fuentes fantasma
const STALL_PARTIAL_NO_PROGRESS_MS: i64 = 12 * 3600 * 1000; // 12 h sin avance y 0 fuentes

/// Intervalo entre dos comprobaciones de mejora de calidad/idioma (12 h).
const UPGRADE_CHECK_INTERVAL_MS: i64 = 12 * 3600 * 1000;
/// Ganancia mínima de puntuación requerida para activar una mejora automática.
const MIN_UPGRADE_SCORE_DELTA: i32 = 35;
/// Número mínimo de episodios faltantes en una misma temporada para buscar primero un paquete de temporada.
const MIN_WANTED_EPS_FOR_SEASON_PACK: usize = 3;
/// Puntuación mínima requerida para aceptar un paquete de temporada.
const MIN_SEASON_PACK_SCORE: i32 = 25;

/// Intervalos de retroceso exponencial (en ms) cuando no se encuentra ninguna versión válida.
const BACKOFF_STEPS_MS: &[i64] = &[
    30 * 60 * 1000,      // 1er échec : 30 min
    2 * 3600 * 1000,     // 2e échec : 2h
    6 * 3600 * 1000,     // 3e échec : 6h
    12 * 3600 * 1000,    // 4e+ échec : 12h
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BlacklistedEntry {
    pub hash: String,
    pub title: String,
    pub reason: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveGrabTracker {
    pub download_id: String,
    pub target_id: String,
    pub media_id: String,
    pub is_episode: bool,
    pub hash: String,
    pub release_title: String,
    pub protocol: String,
    pub quality: String,
    pub language: String,
    pub score: i32,
    pub started_at_ms: i64,
    pub last_progress_bytes: u64,
    pub last_progress_at_ms: i64,
    pub is_upgrade: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GrabDiagnostic {
    pub last_search_ms: i64,
    pub consecutive_empty_searches: u32,
    pub next_eligible_search_ms: i64,
    pub total_candidates: usize,
    pub valid_candidates: usize,
    pub last_status: String,
    pub summary_message: String,
    pub top_rejection_reasons: Vec<String>,
    pub grabbed_title: Option<String>,
    pub grabbed_quality: Option<String>,
    pub grabbed_language: Option<String>,
    pub grabbed_score: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoGrabberState {
    #[serde(default)]
    pub blacklisted_hashes: HashMap<String, Vec<BlacklistedEntry>>,
    #[serde(default)]
    pub active_grabs: HashMap<String, ActiveGrabTracker>,
    #[serde(default)]
    pub diagnostics: HashMap<String, GrabDiagnostic>,
}

impl AutoGrabberState {
    pub fn is_hash_blacklisted(&self, target_id: &str, hash: &str) -> bool {
        if hash.is_empty() {
            return false;
        }
        let h_lower = hash.to_lowercase();
        self.blacklisted_hashes
            .get(target_id)
            .map(|list| list.iter().any(|e| e.hash.eq_ignore_ascii_case(&h_lower)))
            .unwrap_or(false)
    }

    pub fn blacklist_hash(&mut self, target_id: &str, hash: &str, title: &str, reason: &str, now: i64) {
        if hash.is_empty() {
            return;
        }
        let entry = self.blacklisted_hashes.entry(target_id.to_string()).or_default();
        if !entry.iter().any(|e| e.hash.eq_ignore_ascii_case(hash)) {
            entry.push(BlacklistedEntry {
                hash: hash.to_lowercase(),
                title: title.to_string(),
                reason: reason.to_string(),
                timestamp_ms: now,
            });
        }
    }
}

pub struct AutoGrabber;

impl AutoGrabber {
    pub async fn load_state(state: &AppState) -> AutoGrabberState {
        let dl_svc = state.downloads.lock().await;
        dl_svc
            .get_setting(AUTOGRABBER_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<AutoGrabberState>(&raw).ok())
            .unwrap_or_default()
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

    /// Borra la lista negra y reinicia el retroceso para un medio o episodio.
    pub async fn clear_blacklist(state: &AppState, target_id: &str) -> Value {
        let mut ag_state = Self::load_state(state).await;
        let removed = ag_state
            .blacklisted_hashes
            .remove(target_id)
            .map(|v| v.len())
            .unwrap_or(0);
        if let Some(diag) = ag_state.diagnostics.get_mut(target_id) {
            diag.consecutive_empty_searches = 0;
            diag.next_eligible_search_ms = 0;
        }
        Self::save_state(state, &ag_state).await;
        json!({
            "success": true,
            "clearedCount": removed
        })
    }

    /// Despacha la descarga al motor eMule o BitTorrent según el protocolo
    pub async fn grab_release(
        state: &AppState,
        media_id: &str,
        release: &ScoredRelease,
    ) -> Result<String, String> {
        let (is_movie, item_title, target_dest, is_upgrade) = {
            let dl_svc = state.downloads.lock().await;
            let item = dl_svc
                .db()
                .get_media_item(media_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Media item '{media_id}' not found"))?;
            let incoming_dir = dl_svc
                .get_setting("default_download_directory")
                .ok()
                .flatten()
                .map(PathBuf::from)
                .unwrap_or_else(|| state.config.default_download_dir.clone());
            let upgrading = item.status == "completed" && item.file_path.is_some();
            (item.media_type == "movie", item.title, incoming_dir, upgrading)
        };
        let now = now_ms();

        let dl_id_str = if release.protocol == "ed2k" {
            let mut dl_svc = state.downloads.lock().await;
            let snap = dl_svc
                .add_ed2k_link(
                    &release.link,
                    &target_dest,
                    AllocationPolicy::SparseIfSupported,
                    Some(if is_movie { "movies".to_string() } else { "tv".to_string() }),
                )
                .map_err(|e| format!("eD2k grab error: {e}"))?;

            let id_str = snap.id.to_string();
            let _ = dl_svc.db().update_media_status(
                media_id,
                "downloading",
                Some(&id_str),
                now,
            );

            info!(
                "[AUTO_GRAB] Grabbed movie '{}' via eD2k (hash: {}, score: {}, upgrade: {})",
                item_title, release.hash, release.score, is_upgrade
            );
            id_str
        } else {
            let torrent_engine = state
                .torrent_engine()
                .await
                .map_err(|e| format!("BitTorrent engine error: {e}"))?;
            match torrent_engine
                .add_url(
                    &release.link,
                    Some(target_dest),
                    Some(if is_movie { "movies".to_string() } else { "tv".to_string() }),
                )
                .await
            {
                Ok(snap) => {
                    let dl_svc = state.downloads.lock().await;
                    let hash_str = snap.info_hash;
                    let _ = dl_svc.db().update_media_status(
                        media_id,
                        "downloading",
                        Some(&hash_str),
                        now,
                    );

                    info!(
                        "[AUTO_GRAB] Grabbed movie '{}' via BitTorrent (info_hash: {}, score: {}, upgrade: {})",
                        item_title, hash_str, release.score, is_upgrade
                    );
                    hash_str
                }
                Err(e) => return Err(format!("BitTorrent grab error: {e}")),
            }
        };

        // Registrar el seguimiento activo y el diagnóstico en AutoGrabberState
        let mut ag_state = Self::load_state(state).await;
        ag_state.active_grabs.insert(
            dl_id_str.clone(),
            ActiveGrabTracker {
                download_id: dl_id_str.clone(),
                target_id: media_id.to_string(),
                media_id: media_id.to_string(),
                is_episode: false,
                hash: release.hash.clone(),
                release_title: release.title.clone(),
                protocol: release.protocol.clone(),
                quality: release.quality.clone(),
                language: release.language.clone(),
                score: release.score,
                started_at_ms: now,
                last_progress_bytes: 0,
                last_progress_at_ms: now,
                is_upgrade,
            },
        );
        let diag = ag_state.diagnostics.entry(media_id.to_string()).or_default();
        diag.last_search_ms = now;
        diag.consecutive_empty_searches = 0;
        diag.next_eligible_search_ms = 0;
        diag.last_status = if is_upgrade {
            "upgrade_grabbed".to_string()
        } else {
            "grabbed".to_string()
        };
        diag.summary_message = format!(
            "Descargando '{}' [{} · {} · {} pts]",
            release.title, release.quality, release.language, release.score
        );
        diag.grabbed_title = Some(release.title.clone());
        diag.grabbed_quality = Some(release.quality.clone());
        diag.grabbed_language = Some(release.language.clone());
        diag.grabbed_score = Some(release.score);
        Self::save_state(state, &ag_state).await;

        Ok(dl_id_str)
    }

    /// Inspecciona las descargas activas vinculadas a la mediateca para detectar archivos bloqueados
    /// (0 fuentes / 0 bytes prolongado), colocarlos en la lista negra y cambiar automáticamente a otra versión.
    async fn inspect_and_heal_stalled_downloads(
        state: &AppState,
        ag_state: &mut AutoGrabberState,
    ) -> Vec<Value> {
        let now = now_ms();
        let mut healed_events = Vec::new();

        let (media_items, ed2k_details, tv_episodes_by_media) = {
            let dl_svc = state.downloads.lock().await;
            let items = dl_svc.db().list_media_items(None, None).unwrap_or_default();
            let details = dl_svc.list_download_details();
            let mut ep_map = HashMap::new();
            for item in &items {
                if item.media_type == "tv" {
                    let eps = dl_svc.db().get_episodes_for_media(&item.id).unwrap_or_default();
                    ep_map.insert(item.id.clone(), eps);
                }
            }
            (items, details, ep_map)
        };

        let torrent_list = if let Ok(te) = state.torrent_engine().await {
            te.list()
        } else {
            Vec::new()
        };

        // Construir una lista de objetivos en curso de descarga: (target_id, media_id, is_episode, title_label, download_id, has_existing_file)
        let mut downloading_targets: Vec<(String, String, bool, String, String, bool)> = Vec::new();
        for item in &media_items {
            if item.media_type == "movie" && item.status == "downloading" {
                if let Some(ref dl_id) = item.download_id {
                    if !dl_id.is_empty() {
                        downloading_targets.push((
                            item.id.clone(),
                            item.id.clone(),
                            false,
                            item.title.clone(),
                            dl_id.clone(),
                            item.file_path.is_some(),
                        ));
                    }
                }
            } else if item.media_type == "tv" {
                if let Some(eps) = tv_episodes_by_media.get(&item.id) {
                    for ep in eps {
                        if ep.status == "downloading" {
                            if let Some(ref dl_id) = ep.download_id {
                                if !dl_id.is_empty() {
                                    downloading_targets.push((
                                        ep.id.clone(),
                                        item.id.clone(),
                                        true,
                                        format!("{} S{:02}E{:02}", item.title, ep.season_number, ep.episode_number),
                                        dl_id.clone(),
                                        ep.file_path.is_some(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        for (target_id, media_id, is_episode, label, dl_id, has_existing_file) in downloading_targets {
            // Buscar en eD2k o BitTorrent
            let ed2k_match = ed2k_details
                .iter()
                .find(|d| d.snapshot.id.to_string().eq_ignore_ascii_case(&dl_id));
            let bt_match = torrent_list
                .iter()
                .find(|t| t.info_hash.eq_ignore_ascii_case(&dl_id));

            let (received_bytes, sources, created_at_ms, hash, release_title, is_ed2k, bt_id) =
                if let Some(d) = ed2k_match {
                    if d.snapshot.status == "completed" {
                        continue;
                    }
                    (
                        d.snapshot.received_bytes,
                        d.snapshot.source_count as u64,
                        d.snapshot.created_at_unix_ms,
                        d.ed2k_hash.clone(),
                        d.snapshot.file_name.clone(),
                        true,
                        None,
                    )
                } else if let Some(t) = bt_match {
                    if t.status == "completed" {
                        continue;
                    }
                    let started = ag_state
                        .active_grabs
                        .get(&dl_id)
                        .map(|g| g.started_at_ms)
                        .unwrap_or(now);
                    (
                        t.downloaded_bytes,
                        t.peer_count as u64,
                        started,
                        t.info_hash.clone(),
                        t.name.clone(),
                        false,
                        Some(t.id),
                    )
                } else {
                    // Descarga no encontrada (eliminada manualmente) -> volver a poner el elemento en 'wanted' o 'completed'
                    let fallback_status = if has_existing_file { "completed" } else { "wanted" };
                    let dl_svc = state.downloads.lock().await;
                    if is_episode {
                        let _ = dl_svc.db().update_episode_status(&target_id, fallback_status, None, None, now);
                    } else {
                        let _ = dl_svc.db().update_media_status(&target_id, fallback_status, None, now);
                    }
                    ag_state.active_grabs.remove(&dl_id);
                    continue;
                };

            let tracker = ag_state.active_grabs.entry(dl_id.clone()).or_insert_with(|| {
                ActiveGrabTracker {
                    download_id: dl_id.clone(),
                    target_id: target_id.clone(),
                    media_id: media_id.clone(),
                    is_episode,
                    hash: hash.clone(),
                    release_title: release_title.clone(),
                    protocol: if is_ed2k { "ed2k".to_string() } else { "torrent".to_string() },
                    quality: detect_quality_tag(&release_title).to_string(),
                    language: "Dual".to_string(),
                    score: 100,
                    started_at_ms: created_at_ms.max(1),
                    last_progress_bytes: received_bytes,
                    last_progress_at_ms: now,
                    is_upgrade: has_existing_file,
                }
            });

            if received_bytes > tracker.last_progress_bytes {
                tracker.last_progress_bytes = received_bytes;
                tracker.last_progress_at_ms = now;
            }

            let elapsed_since_start = now.saturating_sub(tracker.started_at_ms);
            let elapsed_since_progress = now.saturating_sub(tracker.last_progress_at_ms);
            let tracked_lang = tracker.language.clone();

            // Volver a verificar si la descarga activa infringe la política estricta de idioma (ej.: Unknown / Foreign / English)
            let reevaluated = evaluate_release(
                &release_title,
                if is_ed2k { "ed2k" } else { "torrent" },
                "",
                &hash,
                1_000_000_000,
                sources.max(1),
                "eD2k/Kad",
                !is_episode,
                "1080p-es",
                "es",
            );
            let invalid_lang_reason = if tracked_lang == "Unknown"
                || tracked_lang == "English"
                || tracked_lang == "Foreign"
                || tracked_lang == "Latino"
                || reevaluated.language == "Unknown"
                || reevaluated.language == "English"
                || reevaluated.language == "Foreign"
                || reevaluated.language == "Latino"
            {
                Some("Idioma no válido (Inglés/Extranjero sin Castellano ni VOSE)")
            } else {
                None
            };

            let stall_reason = if let Some(lang_r) = invalid_lang_reason {
                Some(lang_r)
            } else if received_bytes == 0
                && sources == 0
                && elapsed_since_start >= STALL_ZERO_SOURCES_MS
            {
                Some("Estancada (0% y 0 fuentes durante >45 min)")
            } else if received_bytes == 0 && elapsed_since_start >= STALL_ZERO_BYTES_MAX_MS {
                Some("Estancada (0 bytes recibidos durante >3 horas)")
            } else if received_bytes > 0
                && sources == 0
                && elapsed_since_progress >= STALL_PARTIAL_NO_PROGRESS_MS
            {
                Some("Abandonada (sin avance ni fuentes durante >12 horas)")
            } else {
                None
            };

            if let Some(reason) = stall_reason {
                warn!(
                    "[AUTO_GRAB] Healing stalled download for '{}': '{}' ({}) -> blacklisting hash {}",
                    label, release_title, reason, hash
                );

                // 1. Eliminar la descarga muerta
                if is_ed2k {
                    if let Ok(uuid) = Uuid::parse_str(&dl_id) {
                        let mut dl_svc = state.downloads.lock().await;
                        let _ = dl_svc.remove_download(&uuid, true);
                    }
                } else if let Some(tid) = bt_id {
                    if let Ok(te) = state.torrent_engine().await {
                        let _ = te.delete(tid, true).await;
                    }
                }

                // 2. Añadir a la lista negra el hash para este objetivo
                ag_state.blacklist_hash(&target_id, &hash, &release_title, reason, now);
                ag_state.active_grabs.remove(&dl_id);

                // 3. Volver a poner en 'wanted' (o 'completed' si era una mejora) y reiniciar el retroceso
                let fallback_status = if has_existing_file { "completed" } else { "wanted" };
                {
                    let dl_svc = state.downloads.lock().await;
                    if is_episode {
                        let _ = dl_svc
                            .db()
                            .update_episode_status(&target_id, fallback_status, None, None, now);
                    } else {
                        let _ = dl_svc
                            .db()
                            .update_media_status(&target_id, fallback_status, None, now);
                    }
                }

                let diag = ag_state.diagnostics.entry(target_id.clone()).or_default();
                diag.consecutive_empty_searches = 0;
                diag.next_eligible_search_ms = 0;
                diag.last_status = "stalled_failover".to_string();
                diag.summary_message = format!(
                    "Auto-Curación: '{}' descartada por {} — buscando alternativa...",
                    release_title, reason
                );

                healed_events.push(json!({
                    "target": label,
                    "discardedRelease": release_title,
                    "hash": hash,
                    "reason": reason
                }));
            }
        }

        healed_events
    }

    /// Comprueba si una película ya descargada (`completed`) es elegible para una mejora automática
    /// inspeccionando la resolución real y las pistas de audio/subtítulos del archivo en disco mediante `MediaProbe`.
    async fn check_movie_upgrade_eligibility(
        item: &MediaItem,
        ag_state: &AutoGrabberState,
        now: i64,
        force: bool,
    ) -> Option<(String, String, i32)> {
        if item.status != "completed" || !item.monitored {
            return None;
        }
        let fp = item.file_path.as_deref().unwrap_or("");
        if fp.is_empty() {
            return None;
        }

        let diag = ag_state.diagnostics.get(&item.id);
        if !force {
            if let Some(d) = diag {
                if now.saturating_sub(d.last_search_ms) < UPGRADE_CHECK_INTERVAL_MS {
                    return None;
                }
            }
        }

        let probed = if diag.and_then(|d| d.grabbed_quality.as_ref()).is_none()
            || diag.and_then(|d| d.grabbed_language.as_ref()).is_none()
        {
            Some(probe_video_file(Path::new(fp)).await)
        } else {
            None
        };

        let current_quality = diag
            .and_then(|d| d.grabbed_quality.clone())
            .or_else(|| probed.as_ref().map(|p| p.quality_tag.clone()))
            .unwrap_or_else(|| detect_quality_tag(fp).to_string());
        let current_language = diag
            .and_then(|d| d.grabbed_language.clone())
            .or_else(|| probed.as_ref().map(|p| p.detected_language_profile.clone()))
            .unwrap_or_else(|| {
                let lower = fp.to_lowercase();
                if lower.contains("vose") || lower.contains("sub") {
                    "VOSE".to_string()
                } else {
                    "Castellano".to_string()
                }
            });
        let current_score = diag.and_then(|d| d.grabbed_score).unwrap_or_else(|| {
            let q_pts = match current_quality.as_str() {
                "2160p" | "1080p" => 50,
                "720p" => 35,
                _ => 15,
            };
            let l_pts = match current_language.as_str() {
                "Castellano" | "Dual" => 60,
                _ => 30,
            };
            let codec_pts = if probed
                .as_ref()
                .and_then(|p| p.video_codec.as_deref())
                .map(|c| c.contains("hevc") || c.contains("265") || c.contains("av1"))
                .unwrap_or(false)
            {
                25
            } else {
                0
            };
            q_pts + l_pts + 20 + codec_pts
        });

        let wants_4k = item.quality_profile.to_lowercase().contains("4k")
            || item.quality_profile.to_lowercase().contains("2160p");
        let quality_below_cutoff = current_quality == "SD"
            || current_quality == "720p"
            || (wants_4k && current_quality != "2160p");
        let language_below_cutoff =
            current_language != "Castellano" && current_language != "Dual";

        if quality_below_cutoff || language_below_cutoff {
            Some((current_quality, current_language, current_score))
        } else {
            None
        }
    }

    fn compute_next_backoff_ms(now: i64, consecutive_failures: u32) -> i64 {
        let idx = (consecutive_failures.saturating_sub(1) as usize)
            .min(BACKOFF_STEPS_MS.len().saturating_sub(1));
        now + BACKOFF_STEPS_MS[idx]
    }

    /// Ejecuta la búsqueda, el filtrado, el diagnóstico y la descarga automática para una película dada.
    async fn process_movie_item(
        state: &AppState,
        aggregator: &IndexerAggregator,
        item: &MediaItem,
        ag_state: &mut AutoGrabberState,
        force: bool,
    ) -> Option<Value> {
        let now = now_ms();
        let upgrade_ctx = Self::check_movie_upgrade_eligibility(item, ag_state, now, force).await;
        let is_upgrade = upgrade_ctx.is_some();

        if item.status != "wanted" && !is_upgrade {
            return None;
        }

        // Comprobar el retroceso inteligente si no está forzado
        if !force && item.status == "wanted" {
            if let Some(diag) = ag_state.diagnostics.get(&item.id) {
                if now < diag.next_eligible_search_ms {
                    return None;
                }
            }
        }

        let mut queries = Vec::new();
        let clean_title = item.title.replace([':', '-', '\''], " ");
        if let Some(year) = item.year {
            queries.push(format!("{} {year}", item.title));
            if clean_title != item.title {
                queries.push(format!("{clean_title} {year}"));
            }
        } else {
            queries.push(item.title.clone());
            if clean_title != item.title {
                queries.push(clean_title.clone());
            }
        }
        if let Some(ref orig) = item.original_title {
            if !orig.is_empty() && !orig.eq_ignore_ascii_case(&item.title) {
                let clean_orig = orig.replace([':', '-', '\''], " ");
                if let Some(year) = item.year {
                    queries.push(format!("{orig} {year}"));
                    if clean_orig != *orig {
                        queries.push(format!("{clean_orig} {year}"));
                    }
                } else {
                    queries.push(orig.clone());
                    if clean_orig != *orig {
                        queries.push(clean_orig);
                    }
                }
            }
        }

        let es_variants =
            spanish_title_search_variants(&item.title, item.original_title.as_deref());
        for es_t in &es_variants {
            if let Some(year) = item.year {
                queries.push(format!("{es_t} {year}"));
            }
            queries.push(es_t.clone());
        }

        // Títulos alternativos y traducciones oficiales proporcionados por el servicio de metadatos
        let ext_details = state.media_metadata.get_extended_details(item).await;
        for alt_title in &ext_details.alternative_titles {
            let clean_alt = alt_title.replace([':', '-', '\''], " ");
            if let Some(year) = item.year {
                queries.push(format!("{alt_title} {year}"));
                if clean_alt != *alt_title {
                    queries.push(format!("{clean_alt} {year}"));
                }
            } else {
                queries.push(alt_title.clone());
                if clean_alt != *alt_title {
                    queries.push(clean_alt);
                }
            }
        }

        // Dividir por ':' (ej. "Ali G: Who Iz I?" -> "Ali G", "Who Iz I")
        if item.title.contains(':') {
            let parts: Vec<&str> = item.title.split(':').collect();
            if parts.len() >= 2 {
                let prefix = parts[0].trim();
                let suffix = parts[1].trim();
                if !prefix.is_empty() {
                    if let Some(year) = item.year {
                        queries.push(format!("{prefix} {year}"));
                    }
                    queries.push(prefix.to_string());
                }
                if !suffix.is_empty() {
                    if let Some(year) = item.year {
                        queries.push(format!("{suffix} {year}"));
                    }
                    queries.push(suffix.to_string());
                }
            }
        }

        // Variantes en español y números para secuelas (ej. "Part Two" -> "Parte Dos", "Parte 2", "2")
        let title_lower = item.title.to_lowercase();
        if title_lower.contains("part two") || title_lower.contains("part 2") {
            let base = item.title.split(':').next().unwrap_or(&item.title).trim();
            if let Some(year) = item.year {
                queries.push(format!("{base} Parte Dos {year}"));
                queries.push(format!("{base} Parte 2 {year}"));
                queries.push(format!("{base} 2 {year}"));
            } else {
                queries.push(format!("{base} Parte Dos"));
                queries.push(format!("{base} Parte 2"));
                queries.push(format!("{base} 2"));
            }
        } else if title_lower.contains("part three") || title_lower.contains("part 3") {
            let base = item.title.split(':').next().unwrap_or(&item.title).trim();
            if let Some(year) = item.year {
                queries.push(format!("{base} Parte Tres {year}"));
                queries.push(format!("{base} Parte 3 {year}"));
                queries.push(format!("{base} 3 {year}"));
            } else {
                queries.push(format!("{base} Parte Tres"));
                queries.push(format!("{base} Parte 3"));
                queries.push(format!("{base} 3"));
            }
        }

        let search_futs = queries
            .iter()
            .map(|q| aggregator.search_all(state, q, true, &item.quality_profile));
        let query_batches = join_all(search_futs).await;

        let mut all_releases = Vec::new();
        let mut seen_links = HashSet::new();
        for rels in query_batches {
            for r in rels {
                if seen_links.insert(r.link.clone()) {
                    all_releases.push(r);
                }
            }
        }

        let total_candidates = all_releases.len();
        let mut rejection_counts: HashMap<String, usize> = HashMap::new();
        let mut valid_releases = Vec::new();

        for r in all_releases {
            if ag_state.is_hash_blacklisted(&item.id, &r.hash) {
                *rejection_counts
                    .entry("En lista negra (estancada)".to_string())
                    .or_insert(0) += 1;
                continue;
            }
            if r.is_rejected {
                let reason = r
                    .rejection_reason
                    .clone()
                    .unwrap_or_else(|| "Calidad/idioma no válido".to_string());
                *rejection_counts.entry(reason).or_insert(0) += 1;
                continue;
            }
            let alt_refs: Vec<&str> = ext_details.alternative_titles.iter().map(|s| s.as_str()).collect();
            if !matches_release_with_alts(
                &r.title,
                &item.title,
                item.original_title.as_deref(),
                &alt_refs,
                item.year,
                true,
                None,
            ) {
                *rejection_counts
                    .entry("No coincide título/año exacto o es secuela".to_string())
                    .or_insert(0) += 1;
                continue;
            }
            if r.score < 20 {
                *rejection_counts
                    .entry("Puntuación insuficiente".to_string())
                    .or_insert(0) += 1;
                continue;
            }
            if let Some((_, _, current_score)) = upgrade_ctx {
                let is_better_lang = r.language == "Castellano" || r.language == "Dual";
                let wants_4k = item.quality_profile.to_lowercase().contains("4k")
                    || item.quality_profile.to_lowercase().contains("2160p");
                let is_acceptable_hd = r.quality == "1080p" || (wants_4k && r.quality == "2160p");
                if !is_better_lang || !is_acceptable_hd || r.score < current_score + MIN_UPGRADE_SCORE_DELTA {
                    *rejection_counts
                        .entry("No supera la versión actual para Upgrade".to_string())
                        .or_insert(0) += 1;
                    continue;
                }
            }
            valid_releases.push(r);
        }

        valid_releases.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.sources.cmp(&a.sources))
        });

        let valid_count = valid_releases.len();
        let mut top_reasons: Vec<(String, usize)> = rejection_counts.into_iter().collect();
        top_reasons.sort_by_key(|a| std::cmp::Reverse(a.1));
        let formatted_reasons: Vec<String> = top_reasons
            .iter()
            .take(3)
            .map(|(r, c)| format!("{c}× {r}"))
            .collect();

        if let Some(best) = valid_releases.into_iter().next() {
            // Guardar temporalmente el estado antes de `grab_release`, que lo recarga
            Self::save_state(state, ag_state).await;
            if let Ok(dl_id) = Self::grab_release(state, &item.id, &best).await {
                *ag_state = Self::load_state(state).await;
                if let Some(diag) = ag_state.diagnostics.get_mut(&item.id) {
                    diag.total_candidates = total_candidates;
                    diag.valid_candidates = valid_count;
                    diag.top_rejection_reasons = formatted_reasons;
                    diag.summary_message = format!(
                        "Analizadas {} releases ({} válidas) → Seleccionada '{}' [{} · {} · {} pts]",
                        total_candidates, valid_count, best.title, best.quality, best.language, best.score
                    );
                }
                Self::save_state(state, ag_state).await;
                return Some(json!({
                    "title": item.title,
                    "release": best.title,
                    "protocol": best.protocol,
                    "quality": best.quality,
                    "language": best.language,
                    "score": best.score,
                    "isUpgrade": is_upgrade,
                    "downloadId": dl_id
                }));
            }
        }

        // Ninguna versión válida encontrada -> actualizar el diagnóstico y el retroceso inteligente
        let diag = ag_state.diagnostics.entry(item.id.clone()).or_default();
        diag.last_search_ms = now;
        diag.total_candidates = total_candidates;
        diag.valid_candidates = 0;
        diag.consecutive_empty_searches = diag.consecutive_empty_searches.saturating_add(1);
        diag.next_eligible_search_ms =
            Self::compute_next_backoff_ms(now, diag.consecutive_empty_searches);
        diag.top_rejection_reasons = formatted_reasons.clone();

        if is_upgrade {
            diag.last_status = "completed_monitoring_upgrade".to_string();
            diag.summary_message = format!(
                "Búsqueda de Upgrade ({total_candidates} analizadas): aún no hay versión superior en Castellano/Dual 1080p+."
            );
        } else if total_candidates == 0 {
            diag.last_status = "no_results".to_string();
            diag.summary_message =
                "0 resultados encontrados en la red Kademlia/Torznab. Reintento programado con Smart Backoff.".to_string();
        } else {
            diag.last_status = "rejected_all".to_string();
            diag.summary_message = format!(
                "Analizadas {} releases, ninguna superó el filtro estricto ({})",
                total_candidates,
                formatted_reasons.join(" · ")
            );
        }
        Self::save_state(state, ag_state).await;
        None
    }

    /// Ejecuta la búsqueda y descarga automática para los episodios buscados de una serie de TV.
    async fn process_series_item(
        state: &AppState,
        aggregator: &IndexerAggregator,
        item: &MediaItem,
        only_episode_id: Option<&str>,
        ag_state: &mut AutoGrabberState,
        force: bool,
    ) -> Vec<Value> {
        let now = now_ms();
        let mut grabbed_episodes = Vec::new();

        let episodes = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().get_episodes_for_media(&item.id).unwrap_or_default()
        };

        let today = time::OffsetDateTime::now_utc().date();
        let today_str = format!("{today}");

        let mut wanted_eps: Vec<EpisodeItem> = episodes
            .iter()
            .filter(|ep| {
                if let Some(target_ep_id) = only_episode_id {
                    return ep.id == target_ep_id;
                }
                if !ep.monitored || ep.status != "wanted" {
                    return false;
                }
                if let Some(ref d) = ep.air_date {
                    if !d.is_empty() && d.as_str() > today_str.as_str() {
                        return false;
                    }
                }
                if !force {
                    if let Some(diag) = ag_state.diagnostics.get(&ep.id) {
                        if now < diag.next_eligible_search_ms {
                            return false;
                        }
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Priorizar los episodios nunca buscados (`consecutive_empty_searches == 0`)
        wanted_eps.sort_by_key(|ep| {
            let attempts = ag_state
                .diagnostics
                .get(&ep.id)
                .map(|d| d.consecutive_empty_searches)
                .unwrap_or(0);
            (attempts, ep.season_number, ep.episode_number)
        });

        let mut handled_ep_ids: HashSet<String> = HashSet::new();

        // 0. Búsqueda inteligente de paquetes de temporada completa si se buscan >= 3 episodios de una misma temporada
        if only_episode_id.is_none() {
            let mut eps_by_season: HashMap<u32, Vec<EpisodeItem>> = HashMap::new();
            for ep in &wanted_eps {
                eps_by_season.entry(ep.season_number).or_default().push(ep.clone());
            }

            let mut candidate_seasons: Vec<u32> = eps_by_season
                .iter()
                .filter(|(_, eps)| eps.len() >= MIN_WANTED_EPS_FOR_SEASON_PACK)
                .map(|(&s, _)| s)
                .collect();
            candidate_seasons.sort_unstable();

            for season_num in candidate_seasons {
                let season_eps = match eps_by_season.get(&season_num) {
                    Some(v) => v,
                    None => continue,
                };
                let es_variants =
                    spanish_title_search_variants(&item.title, item.original_title.as_deref());
                let mut pack_queries = vec![
                    format!("{} Temporada {season_num}", item.title),
                    format!("{} S{season_num:02}", item.title),
                ];
                if let Some(ref orig) = item.original_title {
                    if !orig.is_empty() && !orig.eq_ignore_ascii_case(&item.title) {
                        pack_queries.push(format!("{orig} S{season_num:02}"));
                    }
                }
                for es_t in &es_variants {
                    pack_queries.push(format!("{es_t} Temporada {season_num}"));
                    pack_queries.push(format!("{es_t} S{season_num:02}"));
                }

                let pack_futs = pack_queries
                    .iter()
                    .map(|q| aggregator.search_all(state, q, false, &item.quality_profile));
                let pack_batches = join_all(pack_futs).await;

                let mut valid_packs = Vec::new();
                let mut seen_pack_links = HashSet::new();
                for rels in pack_batches {
                    for r in rels {
                        if !seen_pack_links.insert(r.link.clone()) {
                            continue;
                        }
                        if r.is_rejected || r.score < MIN_SEASON_PACK_SCORE {
                            continue;
                        }
                        if ag_state.is_hash_blacklisted(&item.id, &r.hash) {
                            continue;
                        }
                        if matches_season_pack_release(
                            &r.title,
                            &item.title,
                            item.original_title.as_deref(),
                            item.year,
                            season_num,
                        ) {
                            valid_packs.push(r);
                        }
                    }
                }

                valid_packs.sort_by(|a, b| {
                    b.score
                        .cmp(&a.score)
                        .then_with(|| b.sources.cmp(&a.sources))
                });

                if let Some(best_pack) = valid_packs.into_iter().next() {
                    let target_dest = {
                        let dl_svc = state.downloads.lock().await;
                        dl_svc
                            .get_setting("default_download_directory")
                            .ok()
                            .flatten()
                            .map(PathBuf::from)
                            .unwrap_or_else(|| state.config.default_download_dir.clone())
                    };

                    let pack_dl_id = if best_pack.protocol == "ed2k" {
                        let mut dl_svc = state.downloads.lock().await;
                        dl_svc
                            .add_ed2k_link(
                                &best_pack.link,
                                &target_dest,
                                AllocationPolicy::SparseIfSupported,
                                Some("tv".to_string()),
                            )
                            .ok()
                            .map(|snap| snap.id.to_string())
                    } else if let Ok(torrent_engine) = state.torrent_engine().await {
                        torrent_engine
                            .add_url(&best_pack.link, Some(target_dest), Some("tv".to_string()))
                            .await
                            .ok()
                            .map(|snap| snap.info_hash)
                    } else {
                        None
                    };

                    if let Some(dl_id_str) = pack_dl_id {
                        let covered_nums = extract_tv_episode_numbers(&best_pack.title, season_num);
                        {
                            let dl_svc = state.downloads.lock().await;
                            for sep in season_eps {
                                if covered_nums.is_empty() || covered_nums.contains(&sep.episode_number) {
                                    let _ = dl_svc.db().update_episode_status(
                                        &sep.id,
                                        "downloading",
                                        None,
                                        Some(&dl_id_str),
                                        now,
                                    );
                                    handled_ep_ids.insert(sep.id.clone());
                                }
                            }
                            let _ = dl_svc.db().update_media_status(
                                &item.id,
                                "downloading",
                                Some(&dl_id_str),
                                now,
                            );
                        }

                        info!(
                            "[AUTO_GRAB] Grabbed Season Pack '{}' S{:02} ({} episodes linked) via {} (score: {})",
                            item.title,
                            season_num,
                            handled_ep_ids.len(),
                            best_pack.protocol,
                            best_pack.score
                        );

                        let first_ep_id = season_eps
                            .first()
                            .map(|e| e.id.clone())
                            .unwrap_or_else(|| item.id.clone());
                        ag_state.active_grabs.insert(
                            dl_id_str.clone(),
                            ActiveGrabTracker {
                                download_id: dl_id_str.clone(),
                                target_id: first_ep_id,
                                media_id: item.id.clone(),
                                is_episode: true,
                                hash: best_pack.hash.clone(),
                                release_title: best_pack.title.clone(),
                                protocol: best_pack.protocol.clone(),
                                quality: best_pack.quality.clone(),
                                language: best_pack.language.clone(),
                                score: best_pack.score,
                                started_at_ms: now,
                                last_progress_bytes: 0,
                                last_progress_at_ms: now,
                                is_upgrade: false,
                            },
                        );

                        let pack_msg = format!(
                            "Pack Temporada {season_num:02}: '{}' [{} · {} · {} pts]",
                            best_pack.title, best_pack.quality, best_pack.language, best_pack.score
                        );
                        let show_diag = ag_state.diagnostics.entry(item.id.clone()).or_default();
                        show_diag.last_search_ms = now;
                        show_diag.last_status = "grabbed".to_string();
                        show_diag.summary_message = pack_msg.clone();

                        for sep in season_eps {
                            if handled_ep_ids.contains(&sep.id) {
                                let ep_diag = ag_state.diagnostics.entry(sep.id.clone()).or_default();
                                ep_diag.last_search_ms = now;
                                ep_diag.consecutive_empty_searches = 0;
                                ep_diag.next_eligible_search_ms = 0;
                                ep_diag.last_status = "grabbed".to_string();
                                ep_diag.summary_message = pack_msg.clone();
                                ep_diag.grabbed_title = Some(best_pack.title.clone());
                                ep_diag.grabbed_quality = Some(best_pack.quality.clone());
                                ep_diag.grabbed_language = Some(best_pack.language.clone());
                                ep_diag.grabbed_score = Some(best_pack.score);
                            }
                        }

                        grabbed_episodes.push(json!({
                            "series": item.title,
                            "season": season_num,
                            "episode": 0,
                            "isSeasonPack": true,
                            "release": best_pack.title,
                            "quality": best_pack.quality,
                            "language": best_pack.language,
                            "score": best_pack.score,
                            "protocol": best_pack.protocol,
                            "downloadId": dl_id_str
                        }));
                    }
                }
            }
        }

        let batch_limit = if only_episode_id.is_some() || force { 24 } else { 8 };
        let es_variants =
            spanish_title_search_variants(&item.title, item.original_title.as_deref());
        let remaining_eps: Vec<EpisodeItem> = wanted_eps
            .into_iter()
            .filter(|e| !handled_ep_ids.contains(&e.id))
            .take(batch_limit)
            .collect();

        // Precargar los resultados de Kad/Torznab a nivel de serie (y sus variantes en español)
        // una sola vez para evitar N búsquedas redundantes sobre la misma palabra clave Kad de título.
        let mut base_series_releases = Vec::new();
        if !remaining_eps.is_empty() {
            let mut base_queries = vec![item.title.clone()];
            if let Some(ref orig) = item.original_title {
                if !orig.is_empty() && !orig.eq_ignore_ascii_case(&item.title) {
                    base_queries.push(orig.clone());
                }
            }
            for es_t in &es_variants {
                if !base_queries.iter().any(|q| q.eq_ignore_ascii_case(es_t)) {
                    base_queries.push(es_t.clone());
                }
            }
            let base_futs = base_queries
                .iter()
                .map(|q| aggregator.search_all(state, q, false, &item.quality_profile));
            let base_batches = join_all(base_futs).await;
            let mut seen_base_links = HashSet::new();
            for rels in base_batches {
                for r in rels {
                    if seen_base_links.insert(r.link.clone()) {
                        base_series_releases.push(r);
                    }
                }
            }

            // Si solo un subconjunto de pares de indexación Kad respondió en el primer intento,
            // realizar una segunda pasada de fusión para maximizar la cobertura de episodios.
            if base_series_releases.len() < 150 {
                aggregator.invalidate_cache().await;
                let retry_futs = base_queries
                    .iter()
                    .map(|q| aggregator.search_all(state, q, false, &item.quality_profile));
                for rels in join_all(retry_futs).await {
                    for r in rels {
                        if seen_base_links.insert(r.link.clone()) {
                            base_series_releases.push(r);
                        }
                    }
                }
            }
        }

        for ep in remaining_eps {
            if handled_ep_ids.contains(&ep.id) {
                continue;
            }
            let mut ep_queries = vec![
                format!("{} S{:02}E{:02}", item.title, ep.season_number, ep.episode_number),
                format!("{} {}x{:02}", item.title, ep.season_number, ep.episode_number),
            ];
            if ep.season_number >= 1 && ep.episode_number < 100 {
                let cap_code = ep.season_number * 100 + ep.episode_number;
                ep_queries.push(format!("{} Cap.{}", item.title, cap_code));
            }
            for es_t in &es_variants {
                ep_queries.push(format!(
                    "{es_t} {}x{:02}",
                    ep.season_number, ep.episode_number
                ));
            }

            let ep_futs = ep_queries
                .iter()
                .map(|q| aggregator.search_all(state, q, false, &item.quality_profile));
            let ep_batches = join_all(ep_futs).await;

            let mut all_releases = Vec::new();
            let mut seen_links = HashSet::new();
            for r in &base_series_releases {
                if seen_links.insert(r.link.clone()) {
                    all_releases.push(r.clone());
                }
            }
            for rels in ep_batches {
                for r in rels {
                    if seen_links.insert(r.link.clone()) {
                        all_releases.push(r);
                    }
                }
            }

            let total_candidates = all_releases.len();
            let mut rejection_counts: HashMap<String, usize> = HashMap::new();
            let mut valid_releases = Vec::new();

            for r in all_releases {
                if ag_state.is_hash_blacklisted(&ep.id, &r.hash) {
                    *rejection_counts
                        .entry("En lista negra (estancada)".to_string())
                        .or_insert(0) += 1;
                    continue;
                }
                if r.is_rejected {
                    let reason = r
                        .rejection_reason
                        .clone()
                        .unwrap_or_else(|| "Calidad/idioma no válido".to_string());
                    *rejection_counts.entry(reason).or_insert(0) += 1;
                    continue;
                }
                if !matches_release(
                    &r.title,
                    &item.title,
                    item.original_title.as_deref(),
                    item.year,
                    false,
                    Some((ep.season_number, ep.episode_number)),
                ) {
                    *rejection_counts
                        .entry("No coincide serie o número de episodio".to_string())
                        .or_insert(0) += 1;
                    continue;
                }
                if r.score < 15 {
                    *rejection_counts
                        .entry("Puntuación insuficiente".to_string())
                        .or_insert(0) += 1;
                    continue;
                }
                valid_releases.push(r);
            }

            valid_releases.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| b.sources.cmp(&a.sources))
            });

            let valid_count = valid_releases.len();
            let mut top_reasons: Vec<(String, usize)> = rejection_counts.into_iter().collect();
            top_reasons.sort_by_key(|a| std::cmp::Reverse(a.1));
            let formatted_reasons: Vec<String> = top_reasons
                .iter()
                .take(3)
                .map(|(r, c)| format!("{c}× {r}"))
                .collect();

            if let Some(best) = valid_releases.into_iter().next() {
                let target_dest = {
                    let dl_svc = state.downloads.lock().await;
                    dl_svc
                        .get_setting("default_download_directory")
                        .ok()
                        .flatten()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| state.config.default_download_dir.clone())
                };

                let grabbed_dl_id = if best.protocol == "ed2k" {
                    let mut dl_svc = state.downloads.lock().await;
                    match dl_svc.add_ed2k_link(
                        &best.link,
                        &target_dest,
                        AllocationPolicy::SparseIfSupported,
                        Some("tv".to_string()),
                    ) {
                        Ok(snap) => Some(snap.id.to_string()),
                        Err(e) => {
                            warn!(
                                "[AUTO_GRAB] Failed to add eD2k link for '{}' S{:02}E{:02}: {e}",
                                item.title, ep.season_number, ep.episode_number
                            );
                            None
                        }
                    }
                } else if let Ok(torrent_engine) = state.torrent_engine().await {
                    match torrent_engine
                        .add_url(&best.link, Some(target_dest), Some("tv".to_string()))
                        .await
                    {
                        Ok(snap) => Some(snap.info_hash),
                        Err(e) => {
                            warn!(
                                "[AUTO_GRAB] Failed to add torrent for '{}' S{:02}E{:02}: {e}",
                                item.title, ep.season_number, ep.episode_number
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                if let Some(dl_id_str) = grabbed_dl_id {
                    let covered_ep_nums =
                        extract_tv_episode_numbers(&best.title, ep.season_number);
                    {
                        let dl_svc = state.downloads.lock().await;
                        let _ = dl_svc.db().update_episode_status(
                            &ep.id,
                            "downloading",
                            None,
                            Some(&dl_id_str),
                            now,
                        );
                        handled_ep_ids.insert(ep.id.clone());

                        // Si la versión cubre múltiples episodios (ej.: S01E01-E03 o Cap.101_102), vincular todos los episodios cubiertos
                        for covered_num in covered_ep_nums {
                            if covered_num != ep.episode_number {
                                if let Some(other_ep) = episodes.iter().find(|e| {
                                    e.season_number == ep.season_number
                                        && e.episode_number == covered_num
                                        && e.status == "wanted"
                                }) {
                                    let _ = dl_svc.db().update_episode_status(
                                        &other_ep.id,
                                        "downloading",
                                        None,
                                        Some(&dl_id_str),
                                        now,
                                    );
                                    handled_ep_ids.insert(other_ep.id.clone());
                                }
                            }
                        }
                        let _ = dl_svc.db().update_media_status(
                            &item.id,
                            "downloading",
                            None,
                            now,
                        );
                    }

                    info!(
                        "[AUTO_GRAB] Grabbed episode '{}' S{:02}E{:02} via {} (hash: {}, score: {})",
                        item.title, ep.season_number, ep.episode_number, best.protocol, best.hash, best.score
                    );

                    ag_state.active_grabs.insert(
                        dl_id_str.clone(),
                        ActiveGrabTracker {
                            download_id: dl_id_str.clone(),
                            target_id: ep.id.clone(),
                            media_id: item.id.clone(),
                            is_episode: true,
                            hash: best.hash.clone(),
                            release_title: best.title.clone(),
                            protocol: best.protocol.clone(),
                            quality: best.quality.clone(),
                            language: best.language.clone(),
                            score: best.score,
                            started_at_ms: now,
                            last_progress_bytes: 0,
                            last_progress_at_ms: now,
                            is_upgrade: false,
                        },
                    );

                    let msg = format!(
                        "S{:02}E{:02}: Seleccionada '{}' [{} · {} · {} pts]",
                        ep.season_number, ep.episode_number, best.title, best.quality, best.language, best.score
                    );
                    let ep_diag = ag_state.diagnostics.entry(ep.id.clone()).or_default();
                    ep_diag.last_search_ms = now;
                    ep_diag.consecutive_empty_searches = 0;
                    ep_diag.next_eligible_search_ms = 0;
                    ep_diag.total_candidates = total_candidates;
                    ep_diag.valid_candidates = valid_count;
                    ep_diag.last_status = "grabbed".to_string();
                    ep_diag.summary_message = msg.clone();
                    ep_diag.top_rejection_reasons = formatted_reasons;
                    ep_diag.grabbed_title = Some(best.title.clone());
                    ep_diag.grabbed_quality = Some(best.quality.clone());
                    ep_diag.grabbed_language = Some(best.language.clone());
                    ep_diag.grabbed_score = Some(best.score);

                    let show_diag = ag_state.diagnostics.entry(item.id.clone()).or_default();
                    show_diag.last_search_ms = now;
                    show_diag.last_status = "grabbed".to_string();
                    show_diag.summary_message = msg;

                    grabbed_episodes.push(json!({
                        "series": item.title,
                        "season": ep.season_number,
                        "episode": ep.episode_number,
                        "release": best.title,
                        "quality": best.quality,
                        "language": best.language,
                        "score": best.score,
                        "protocol": best.protocol,
                        "downloadId": dl_id_str
                    }));
                    continue;
                }
            }

            // Ninguna versión encontrada para este episodio
            let (ep_status, ep_msg) = {
                let ep_diag = ag_state.diagnostics.entry(ep.id.clone()).or_default();
                ep_diag.last_search_ms = now;
                ep_diag.total_candidates = total_candidates;
                ep_diag.valid_candidates = 0;
                ep_diag.consecutive_empty_searches =
                    ep_diag.consecutive_empty_searches.saturating_add(1);
                ep_diag.next_eligible_search_ms =
                    Self::compute_next_backoff_ms(now, ep_diag.consecutive_empty_searches);
                ep_diag.top_rejection_reasons = formatted_reasons.clone();
                ep_diag.last_status = if total_candidates == 0 {
                    "no_results".to_string()
                } else {
                    "rejected_all".to_string()
                };
                ep_diag.summary_message = if total_candidates == 0 {
                    format!(
                        "S{:02}E{:02}: 0 resultados en la red (Backoff activo)",
                        ep.season_number, ep.episode_number
                    )
                } else {
                    format!(
                        "S{:02}E{:02}: {} analizadas, todas descartadas ({})",
                        ep.season_number,
                        ep.episode_number,
                        total_candidates,
                        formatted_reasons.join(" · ")
                    )
                };
                (ep_diag.last_status.clone(), ep_diag.summary_message.clone())
            };

            let show_diag = ag_state.diagnostics.entry(item.id.clone()).or_default();
            show_diag.last_search_ms = now;
            show_diag.total_candidates = total_candidates;
            if show_diag.last_status != "grabbed" {
                show_diag.last_status = ep_status;
                show_diag.summary_message = ep_msg;
                show_diag.top_rejection_reasons = formatted_reasons;
            }
        }

        Self::save_state(state, ag_state).await;
        grabbed_episodes
    }

    /// Fuerza la ejecución inmediata de AutoGrabber sobre una película o serie específica desde la interfaz.
    pub async fn grab_for_media_id(
        state: &AppState,
        aggregator: &IndexerAggregator,
        media_id: &str,
        episode_id: Option<&str>,
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
