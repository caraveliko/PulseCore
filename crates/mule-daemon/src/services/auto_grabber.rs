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
    apply_strict_castilian_filter, castilian_confidence_level, evaluate_release,
    extract_tv_episode_numbers, matches_release_with_alts, matches_season_pack_release,
    normalize_accents_and_symbols, spanish_title_search_variants, strip_trailing_year_suffix,
    ScoredRelease,
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
/// Motivo normalizado registrado en lista negra cuando el audio no está en Castellano de España.
pub const WRONG_LANGUAGE_REASON: &str = "wrong_language_not_castilian";
/// Prefijo para entradas de lista negra basadas en título normalizado sin hash conocido.
const TITLE_BLACKLIST_PREFIX: &str = "title:";
/// Puntuación mínima exigida para seleccionar automáticamente un reemplazo en Castellano estricto.
const MIN_STRICT_REPLACEMENT_SCORE: i32 = 15;

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
    fn normalized_title_key(title: &str) -> String {
        normalize_accents_and_symbols(title)
    }

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

    /// Comprueba si una release está en lista negra por hash o por título normalizado equivalente.
    pub fn is_release_blacklisted(&self, target_id: &str, hash: &str, title: &str) -> bool {
        if self.is_hash_blacklisted(target_id, hash) {
            return true;
        }
        let title_key = Self::normalized_title_key(title);
        if title_key.is_empty() {
            return false;
        }
        self.blacklisted_hashes
            .get(target_id)
            .map(|list| {
                list.iter()
                    .any(|e| !e.title.is_empty() && Self::normalized_title_key(&e.title) == title_key)
            })
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

    /// Registra una release en lista negra usando su hash o, si no existe, una clave sintética de título.
    pub fn blacklist_release(
        &mut self,
        target_id: &str,
        hash: Option<&str>,
        title: &str,
        reason: &str,
        now: i64,
    ) {
        if let Some(h) = hash.filter(|s| !s.trim().is_empty()) {
            self.blacklist_hash(target_id, h.trim(), title, reason, now);
            return;
        }
        let title_key = Self::normalized_title_key(title);
        if title_key.is_empty() {
            return;
        }
        let synthetic_hash = format!("{TITLE_BLACKLIST_PREFIX}{title_key}");
        self.blacklist_hash(target_id, &synthetic_hash, title, reason, now);
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

    /// Despacha la descarga al motor eMule o BitTorrent según el protocolo (película o episodio).
    pub async fn grab_release(
        state: &AppState,
        media_id: &str,
        release: &ScoredRelease,
    ) -> Result<String, String> {
        Self::grab_release_for_target(state, media_id, None, release).await
    }

    /// Despacha la descarga al motor eMule o BitTorrent para una película (`episode_id = None`)
    /// o un episodio concreto (`episode_id = Some(ep_id)`), manteniendo intacto el archivo actual en disco
    /// hasta que la nueva descarga finalice y sea organizada.
    pub async fn grab_release_for_target(
        state: &AppState,
        media_id: &str,
        episode_id: Option<&str>,
        release: &ScoredRelease,
    ) -> Result<String, String> {
        let (is_movie, item_title, target_dest, is_upgrade, target_id, target_label, is_episode) = {
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

            if let Some(ep_id) = episode_id {
                let eps = dl_svc
                    .db()
                    .get_episodes_for_media(media_id)
                    .map_err(|e| e.to_string())?;
                let ep = eps
                    .into_iter()
                    .find(|e| e.id == ep_id)
                    .ok_or_else(|| format!("Episode '{ep_id}' not found for media '{media_id}'"))?;
                let upgrading = ep.status == "completed" && ep.file_path.is_some();
                let label = format!(
                    "{} S{:02}E{:02}",
                    item.title, ep.season_number, ep.episode_number
                );
                (false, item.title, incoming_dir, upgrading, ep.id, label, true)
            } else {
                let upgrading = item.status == "completed" && item.file_path.is_some();
                let label = item.title.clone();
                (
                    item.media_type == "movie",
                    item.title,
                    incoming_dir,
                    upgrading,
                    media_id.to_string(),
                    label,
                    false,
                )
            }
        };
        let now = now_ms();
        let category = if is_movie { "movies" } else { "tv" }.to_string();

        let dl_id_str = if release.protocol == "ed2k" {
            let mut dl_svc = state.downloads.lock().await;
            let snap = dl_svc
                .add_ed2k_link(
                    &release.link,
                    &target_dest,
                    AllocationPolicy::SparseIfSupported,
                    Some(category),
                )
                .map_err(|e| format!("eD2k grab error: {e}"))?;

            let id_str = snap.id.to_string();
            if is_episode {
                let _ = dl_svc.db().update_episode_status(
                    &target_id,
                    "downloading",
                    None,
                    Some(&id_str),
                    now,
                );
            } else {
                let _ = dl_svc.db().update_media_status(
                    media_id,
                    "downloading",
                    Some(&id_str),
                    now,
                );
            }

            info!(
                "[AUTO_GRAB] Grabbed '{}' via eD2k (hash: {}, score: {}, upgrade: {})",
                target_label, release.hash, release.score, is_upgrade
            );
            id_str
        } else {
            let torrent_engine = state
                .torrent_engine()
                .await
                .map_err(|e| format!("BitTorrent engine error: {e}"))?;
            match torrent_engine
                .add_url(&release.link, Some(target_dest), Some(category))
                .await
            {
                Ok(snap) => {
                    let dl_svc = state.downloads.lock().await;
                    let hash_str = snap.info_hash;
                    if is_episode {
                        let _ = dl_svc.db().update_episode_status(
                            &target_id,
                            "downloading",
                            None,
                            Some(&hash_str),
                            now,
                        );
                    } else {
                        let _ = dl_svc.db().update_media_status(
                            media_id,
                            "downloading",
                            Some(&hash_str),
                            now,
                        );
                    }

                    info!(
                        "[AUTO_GRAB] Grabbed '{}' via BitTorrent (info_hash: {}, score: {}, upgrade: {})",
                        target_label, hash_str, release.score, is_upgrade
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
                target_id: target_id.clone(),
                media_id: media_id.to_string(),
                is_episode,
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
        let summary = format!(
            "Descargando '{}' [{} · {} · {} pts]",
            release.title, release.quality, release.language, release.score
        );
        let diag = ag_state.diagnostics.entry(target_id).or_default();
        diag.last_search_ms = now;
        diag.consecutive_empty_searches = 0;
        diag.next_eligible_search_ms = 0;
        diag.last_status = if is_upgrade {
            "upgrade_grabbed".to_string()
        } else {
            "grabbed".to_string()
        };
        diag.summary_message = summary.clone();
        diag.grabbed_title = Some(release.title.clone());
        diag.grabbed_quality = Some(release.quality.clone());
        diag.grabbed_language = Some(release.language.clone());
        diag.grabbed_score = Some(release.score);

        if is_episode {
            let show_diag = ag_state.diagnostics.entry(media_id.to_string()).or_default();
            show_diag.last_search_ms = now;
            show_diag.last_status = "grabbed".to_string();
            show_diag.summary_message = format!("{target_label}: {summary}");
        } else {
            let _ = item_title;
        }

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
        let (upgrade_enabled, max_res) = {
            let dl_svc = state.downloads.lock().await;
            let upg = dl_svc
                .get_setting("quality_upgrade_enabled")
                .ok()
                .flatten()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true);
            let res = dl_svc
                .get_setting("quality_max_resolution")
                .ok()
                .flatten()
                .unwrap_or_else(|| "1080p".to_string());
            (upg, res)
        };
        let upgrade_ctx = if upgrade_enabled || force {
            Self::check_movie_upgrade_eligibility(item, ag_state, now, force).await
        } else {
            None
        };
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

        let effective_profile = match max_res.as_str() {
            "2160p" if !item.quality_profile.contains("2160p") && !item.quality_profile.contains("4k") => {
                format!("{}-2160p", item.quality_profile)
            }
            "720p" => "720p-es".to_string(),
            _ => item.quality_profile.clone(),
        };

        let search_futs = queries
            .iter()
            .map(|q| aggregator.search_all(state, q, true, &effective_profile));
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
            if ag_state.is_release_blacklisted(&item.id, &r.hash, &r.title) {
                *rejection_counts
                    .entry("En lista negra (descartada)".to_string())
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
        only_season: Option<u32>,
        ag_state: &mut AutoGrabberState,
        force: bool,
    ) -> Vec<Value> {
        let now = now_ms();
        let mut grabbed_episodes = Vec::new();

        let episodes = {
            let dl_svc = state.downloads.lock().await;
            if let Some(target_season) = only_season {
                let _ = dl_svc
                    .db()
                    .set_season_monitored(&item.id, target_season, true, now);
            }
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
                if let Some(target_season) = only_season {
                    if ep.season_number != target_season {
                        return false;
                    }
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
        let search_title = strip_trailing_year_suffix(&item.title);
        let ext_details = state.media_metadata.get_extended_details(item).await;
        let alt_refs: Vec<&str> = ext_details
            .alternative_titles
            .iter()
            .map(|s| s.as_str())
            .collect();

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
                    spanish_title_search_variants(search_title, item.original_title.as_deref());
                let mut pack_queries = vec![
                    format!("{search_title} Temporada {season_num}"),
                    format!("{search_title} S{season_num:02}"),
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
                        if ag_state.is_release_blacklisted(&item.id, &r.hash, &r.title) {
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
            spanish_title_search_variants(search_title, item.original_title.as_deref());
        let remaining_eps: Vec<EpisodeItem> = wanted_eps
            .into_iter()
            .filter(|e| !handled_ep_ids.contains(&e.id))
            .take(batch_limit)
            .collect();

        // Precargar los resultados de Kad/Torznab a nivel de serie (y sus variantes en español)
        // una sola vez para evitar N búsquedas redundantes sobre la misma palabra clave Kad de título.
        let mut base_series_releases = Vec::new();
        if !remaining_eps.is_empty() {
            let mut base_queries = vec![search_title.to_string()];
            if let Some(ref orig) = item.original_title {
                let clean_orig = strip_trailing_year_suffix(orig);
                if !clean_orig.is_empty() && !clean_orig.eq_ignore_ascii_case(search_title) {
                    base_queries.push(clean_orig.to_string());
                }
            }
            for es_t in &es_variants {
                if !base_queries.iter().any(|q| q.eq_ignore_ascii_case(es_t)) {
                    base_queries.push(es_t.clone());
                }
            }
            for alt_title in ext_details.alternative_titles.iter().take(4) {
                let clean_alt = strip_trailing_year_suffix(alt_title);
                if !clean_alt.is_empty()
                    && !base_queries.iter().any(|q| q.eq_ignore_ascii_case(clean_alt))
                {
                    base_queries.push(clean_alt.to_string());
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
                format!("{search_title} S{:02}E{:02}", ep.season_number, ep.episode_number),
                format!("{search_title} {}x{:02}", ep.season_number, ep.episode_number),
            ];
            if ep.season_number >= 1 && ep.episode_number < 100 {
                let cap_code = ep.season_number * 100 + ep.episode_number;
                ep_queries.push(format!("{search_title} Cap.{cap_code}"));
            }
            for es_t in &es_variants {
                ep_queries.push(format!(
                    "{es_t} {}x{:02}",
                    ep.season_number, ep.episode_number
                ));
                ep_queries.push(format!(
                    "{es_t} S{:02}E{:02}",
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
                if ag_state.is_release_blacklisted(&ep.id, &r.hash, &r.title) {
                    *rejection_counts
                        .entry("En lista negra (descartada)".to_string())
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
                if !matches_release_with_alts(
                    &r.title,
                    &item.title,
                    item.original_title.as_deref(),
                    &alt_refs,
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

    /// Fuerza la ejecución inmediata de AutoGrabber sobre una película, serie o temporada específica desde la interfaz.
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

    /// Registra en lista negra la versión actual (activa o ya catalogada) de un medio o episodio
    /// sin borrar el archivo en disco, cancelando opcionalmente su descarga activa.
    pub async fn blacklist_current_for_target(
        state: &AppState,
        ag_state: &mut AutoGrabberState,
        media_id: &str,
        episode_id: Option<&str>,
        season_number: Option<u32>,
        reason: &str,
        cancel_active_download: bool,
    ) -> Result<usize, String> {
        let now = now_ms();
        let (item, episodes) = {
            let dl_svc = state.downloads.lock().await;
            let item = dl_svc
                .db()
                .get_media_item(media_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Media item '{media_id}' not found"))?;
            let eps = if item.media_type == "tv" {
                dl_svc
                    .db()
                    .get_episodes_for_media(media_id)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            (item, eps)
        };

        struct BlacklistTarget {
            id: String,
            is_episode: bool,
            download_id: Option<String>,
            original_release_name: Option<String>,
            file_path: Option<String>,
        }

        let mut targets: Vec<BlacklistTarget> = Vec::new();
        if let Some(ep_id) = episode_id {
            let ep = episodes
                .iter()
                .find(|e| e.id == ep_id)
                .ok_or_else(|| format!("Episode '{ep_id}' not found for media '{media_id}'"))?;
            targets.push(BlacklistTarget {
                id: ep.id.clone(),
                is_episode: true,
                download_id: ep.download_id.clone(),
                original_release_name: ep.original_release_name.clone(),
                file_path: ep.file_path.clone(),
            });
        } else if let Some(s_num) = season_number {
            targets.push(BlacklistTarget {
                id: item.id.clone(),
                is_episode: false,
                download_id: item.download_id.clone(),
                original_release_name: item.original_release_name.clone(),
                file_path: None,
            });
            for ep in episodes.iter().filter(|e| e.season_number == s_num) {
                targets.push(BlacklistTarget {
                    id: ep.id.clone(),
                    is_episode: true,
                    download_id: ep.download_id.clone(),
                    original_release_name: ep.original_release_name.clone(),
                    file_path: ep.file_path.clone(),
                });
            }
        } else {
            targets.push(BlacklistTarget {
                id: item.id.clone(),
                is_episode: false,
                download_id: item.download_id.clone(),
                original_release_name: item.original_release_name.clone(),
                file_path: item.file_path.clone(),
            });
        }

        let mut blacklisted_count = 0usize;
        let mut dl_ids_to_cancel: HashSet<String> = HashSet::new();

        for target in &targets {
            let tid = &target.id;
            let is_ep = target.is_episode;
            let dl_id_opt = &target.download_id;
            let orig_rel = &target.original_release_name;
            let fp_opt = &target.file_path;
            let before_len = ag_state
                .blacklisted_hashes
                .get(tid)
                .map(|v| v.len())
                .unwrap_or(0);

            // 1. Descargas activas rastreadas para este objetivo
            let matching_trackers: Vec<ActiveGrabTracker> = ag_state
                .active_grabs
                .values()
                .filter(|g| {
                    g.target_id == *tid
                        || dl_id_opt
                            .as_deref()
                            .is_some_and(|dl| !dl.is_empty() && g.download_id == dl)
                })
                .cloned()
                .collect();

            for tracker in matching_trackers {
                ag_state.blacklist_release(
                    tid,
                    Some(&tracker.hash),
                    &tracker.release_title,
                    reason,
                    now,
                );
                if cancel_active_download {
                    dl_ids_to_cancel.insert(tracker.download_id.clone());
                    ag_state.active_grabs.remove(&tracker.download_id);
                }
            }

            if cancel_active_download {
                if let Some(dl_id) = dl_id_opt.as_deref().filter(|s| !s.is_empty()) {
                    dl_ids_to_cancel.insert(dl_id.to_string());
                }
            }

            // 2. Título registrado en diagnóstico previo
            if let Some(grabbed_title) = ag_state
                .diagnostics
                .get(tid)
                .and_then(|d| d.grabbed_title.clone())
            {
                ag_state.blacklist_release(tid, None, &grabbed_title, reason, now);
            }

            // 3. Nombre original de lanzamiento guardado en base de datos
            if let Some(rel_name) = orig_rel.as_deref().filter(|s| !s.trim().is_empty()) {
                ag_state.blacklist_release(tid, None, rel_name, reason, now);
            }

            // 4. Nombre de archivo en disco
            if let Some(fp) = fp_opt.as_deref().filter(|s| !s.trim().is_empty()) {
                if let Some(fname) = Path::new(fp).file_name().and_then(|n| n.to_str()) {
                    ag_state.blacklist_release(tid, None, fname, reason, now);
                }
            }

            if let Some(diag) = ag_state.diagnostics.get_mut(tid) {
                diag.consecutive_empty_searches = 0;
                diag.next_eligible_search_ms = 0;
            }

            let after_len = ag_state
                .blacklisted_hashes
                .get(tid)
                .map(|v| v.len())
                .unwrap_or(0);
            blacklisted_count += after_len.saturating_sub(before_len);

            // Si se cancela la descarga activa, restaurar estado sin borrar el archivo catalogado en disco
            if cancel_active_download && dl_id_opt.is_some() {
                let fallback_status = if fp_opt.is_some() { "completed" } else { "wanted" };
                let dl_svc = state.downloads.lock().await;
                if is_ep {
                    let _ = dl_svc
                        .db()
                        .update_episode_status(tid, fallback_status, None, None, now);
                } else {
                    let _ = dl_svc
                        .db()
                        .update_media_status(tid, fallback_status, None, now);
                }
            }
        }

        // Cancelar en eD2k / BitTorrent las descargas activas descartadas
        if cancel_active_download && !dl_ids_to_cancel.is_empty() {
            let torrent_list = if let Ok(te) = state.torrent_engine().await {
                te.list()
            } else {
                Vec::new()
            };
            for dl_id in dl_ids_to_cancel {
                if let Ok(uuid) = Uuid::parse_str(&dl_id) {
                    let mut dl_svc = state.downloads.lock().await;
                    let _ = dl_svc.remove_download(&uuid, true);
                } else if let Some(t) = torrent_list
                    .iter()
                    .find(|t| t.info_hash.eq_ignore_ascii_case(&dl_id))
                {
                    if let Ok(te) = state.torrent_engine().await {
                        let _ = te.delete(t.id, true).await;
                    }
                }
            }
        }

        Ok(blacklisted_count)
    }

    /// Busca candidatos en modo Castellano de España estricto para una película o episodio concreto.
    async fn search_strict_castilian_candidates(
        state: &AppState,
        aggregator: &IndexerAggregator,
        ag_state: &AutoGrabberState,
        item: &MediaItem,
        episode: Option<&EpisodeItem>,
    ) -> Vec<ScoredRelease> {
        let search_title = strip_trailing_year_suffix(&item.title);
        let ext_details = state.media_metadata.get_extended_details(item).await;
        let alt_refs: Vec<&str> = ext_details
            .alternative_titles
            .iter()
            .map(|s| s.as_str())
            .collect();
        let es_variants =
            spanish_title_search_variants(search_title, item.original_title.as_deref());

        let mut queries = Vec::new();
        if let Some(ep) = episode {
            queries.push(format!(
                "{search_title} S{:02}E{:02} Castellano",
                ep.season_number, ep.episode_number
            ));
            queries.push(format!(
                "{search_title} S{:02}E{:02}",
                ep.season_number, ep.episode_number
            ));
            queries.push(format!(
                "{search_title} {}x{:02}",
                ep.season_number, ep.episode_number
            ));
            if ep.season_number >= 1 && ep.episode_number < 100 {
                let cap_code = ep.season_number * 100 + ep.episode_number;
                queries.push(format!("{search_title} Cap.{cap_code}"));
            }
            for es_t in &es_variants {
                queries.push(format!(
                    "{es_t} {}x{:02} Castellano",
                    ep.season_number, ep.episode_number
                ));
                queries.push(format!(
                    "{es_t} S{:02}E{:02}",
                    ep.season_number, ep.episode_number
                ));
            }
        } else {
            if let Some(year) = item.year {
                queries.push(format!("{search_title} {year} Castellano"));
                queries.push(format!("{search_title} {year}"));
            } else {
                queries.push(format!("{search_title} Castellano"));
            }
            queries.push(search_title.to_string());
            if let Some(ref orig) = item.original_title {
                let clean_orig = strip_trailing_year_suffix(orig);
                if !clean_orig.is_empty() && !clean_orig.eq_ignore_ascii_case(search_title) {
                    queries.push(format!("{clean_orig} Castellano"));
                    queries.push(clean_orig.to_string());
                }
            }
            for es_t in &es_variants {
                queries.push(format!("{es_t} Castellano"));
                queries.push(es_t.clone());
            }
        }

        let is_movie = item.media_type == "movie";
        let futs = queries
            .iter()
            .map(|q| aggregator.search_all(state, q, is_movie, &item.quality_profile));
        let batches = join_all(futs).await;

        let target_id = episode.map(|e| e.id.as_str()).unwrap_or(item.id.as_str());
        let ep_coords = episode.map(|e| (e.season_number, e.episode_number));
        let mut seen_links = HashSet::new();
        let mut candidates = Vec::new();

        for rels in batches {
            for r in rels {
                if !seen_links.insert(r.link.clone()) {
                    continue;
                }
                if r.is_rejected || ag_state.is_release_blacklisted(target_id, &r.hash, &r.title) {
                    continue;
                }
                if !matches_release_with_alts(
                    &r.title,
                    &item.title,
                    item.original_title.as_deref(),
                    &alt_refs,
                    item.year,
                    is_movie,
                    ep_coords,
                ) {
                    continue;
                }
                candidates.push(r);
            }
        }

        apply_strict_castilian_filter(&mut candidates);
        candidates.retain(|r| !r.is_rejected && r.score >= MIN_STRICT_REPLACEMENT_SCORE);
        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.sources.cmp(&a.sources))
        });
        candidates
    }

    /// Busca candidatos de paquete de temporada en modo Castellano de España estricto.
    async fn search_strict_castilian_season_packs(
        state: &AppState,
        aggregator: &IndexerAggregator,
        ag_state: &AutoGrabberState,
        item: &MediaItem,
        season_num: u32,
    ) -> Vec<ScoredRelease> {
        let search_title = strip_trailing_year_suffix(&item.title);
        let es_variants =
            spanish_title_search_variants(search_title, item.original_title.as_deref());

        let mut pack_queries = vec![
            format!("{search_title} Temporada {season_num} Castellano"),
            format!("{search_title} Temporada {season_num}"),
            format!("{search_title} S{season_num:02} Castellano"),
            format!("{search_title} S{season_num:02}"),
        ];
        for es_t in &es_variants {
            pack_queries.push(format!("{es_t} Temporada {season_num}"));
            pack_queries.push(format!("{es_t} S{season_num:02}"));
        }

        let futs = pack_queries
            .iter()
            .map(|q| aggregator.search_all(state, q, false, &item.quality_profile));
        let batches = join_all(futs).await;

        let mut seen_links = HashSet::new();
        let mut packs = Vec::new();
        for rels in batches {
            for r in rels {
                if !seen_links.insert(r.link.clone()) {
                    continue;
                }
                if r.is_rejected || ag_state.is_release_blacklisted(&item.id, &r.hash, &r.title) {
                    continue;
                }
                if matches_season_pack_release(
                    &r.title,
                    &item.title,
                    item.original_title.as_deref(),
                    item.year,
                    season_num,
                ) {
                    packs.push(r);
                }
            }
        }

        apply_strict_castilian_filter(&mut packs);
        packs.retain(|r| !r.is_rejected && r.score >= MIN_SEASON_PACK_SCORE);
        packs.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.sources.cmp(&a.sources))
        });
        packs
    }

    fn candidate_to_json(r: &ScoredRelease) -> Value {
        json!({
            "title": r.title,
            "protocol": r.protocol,
            "link": r.link,
            "hash": r.hash,
            "sizeBytes": r.size_bytes,
            "sources": r.sources,
            "score": r.score,
            "quality": r.quality,
            "language": r.language,
            "isRejected": r.is_rejected,
            "rejectionReason": r.rejection_reason,
            "indexerName": r.indexer_name,
            "castilianConfidence": castilian_confidence_level(r),
        })
    }

    /// Descarta la versión actual por idioma incorrecto y busca (o descarga automáticamente)
    /// la siguiente mejor recomendación en Castellano de España, manteniendo intacto el archivo
    /// en disco hasta que la nueva descarga finalice.
    pub async fn replace_wrong_language(
        state: &AppState,
        aggregator: &IndexerAggregator,
        media_id: &str,
        episode_id: Option<&str>,
        season_number: Option<u32>,
        auto_grab: bool,
        cancel_active_download: bool,
    ) -> Result<Value, String> {
        let _pass_guard = autograb_pass_mutex().lock().await;
        aggregator.invalidate_cache().await;
        let mut ag_state = Self::load_state(state).await;

        let blacklisted_count = Self::blacklist_current_for_target(
            state,
            &mut ag_state,
            media_id,
            episode_id,
            season_number,
            WRONG_LANGUAGE_REASON,
            cancel_active_download,
        )
        .await?;
        Self::save_state(state, &ag_state).await;

        let (item, episodes) = {
            let dl_svc = state.downloads.lock().await;
            let item = dl_svc
                .db()
                .get_media_item(media_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Media item '{media_id}' not found"))?;
            let eps = if item.media_type == "tv" {
                dl_svc
                    .db()
                    .get_episodes_for_media(media_id)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            (item, eps)
        };

        // Caso 1: Temporada completa
        if item.media_type == "tv" && episode_id.is_none() {
            let s_num = season_number.ok_or_else(|| {
                "Se requiere 'episodeId' o 'seasonNumber' para reemplazar idioma en una serie".to_string()
            })?;
            let season_eps: Vec<EpisodeItem> = episodes
                .into_iter()
                .filter(|e| e.season_number == s_num)
                .collect();
            if season_eps.is_empty() {
                return Err(format!("No se encontraron episodios para la temporada {s_num}"));
            }

            let pack_candidates =
                Self::search_strict_castilian_season_packs(state, aggregator, &ag_state, &item, s_num)
                    .await;
            let candidates_json: Vec<Value> =
                pack_candidates.iter().map(Self::candidate_to_json).collect();

            if !auto_grab {
                return Ok(json!({
                    "success": true,
                    "mode": "preview",
                    "recommendedIndex": if candidates_json.is_empty() { Value::Null } else { json!(0) },
                    "candidates": candidates_json,
                    "blacklistedCount": blacklisted_count,
                }));
            }

            if let Some(best_pack) = pack_candidates.into_iter().next() {
                let target_dest = {
                    let dl_svc = state.downloads.lock().await;
                    dl_svc
                        .get_setting("default_download_directory")
                        .ok()
                        .flatten()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| state.config.default_download_dir.clone())
                };
                let now = now_ms();
                let pack_dl_id = if best_pack.protocol == "ed2k" {
                    let mut dl_svc = state.downloads.lock().await;
                    dl_svc
                        .add_ed2k_link(
                            &best_pack.link,
                            &target_dest,
                            AllocationPolicy::SparseIfSupported,
                            Some("tv".to_string()),
                        )
                        .map_err(|e| format!("eD2k grab error: {e}"))?
                        .id
                        .to_string()
                } else {
                    let torrent_engine = state
                        .torrent_engine()
                        .await
                        .map_err(|e| format!("BitTorrent engine error: {e}"))?;
                    torrent_engine
                        .add_url(&best_pack.link, Some(target_dest), Some("tv".to_string()))
                        .await
                        .map_err(|e| format!("BitTorrent grab error: {e}"))?
                        .info_hash
                };

                let covered_nums = extract_tv_episode_numbers(&best_pack.title, s_num);
                let mut linked_eps = 0usize;
                {
                    let dl_svc = state.downloads.lock().await;
                    for sep in &season_eps {
                        if covered_nums.is_empty() || covered_nums.contains(&sep.episode_number) {
                            let _ = dl_svc.db().update_episode_status(
                                &sep.id,
                                "downloading",
                                None,
                                Some(&pack_dl_id),
                                now,
                            );
                            linked_eps += 1;
                        }
                    }
                    let _ = dl_svc
                        .db()
                        .update_media_status(&item.id, "downloading", Some(&pack_dl_id), now);
                }

                let pack_msg = format!(
                    "Reemplazo Castellano T.{s_num:02}: '{}' [{} · {} · {} pts]",
                    best_pack.title, best_pack.quality, best_pack.language, best_pack.score
                );
                let show_diag = ag_state.diagnostics.entry(item.id.clone()).or_default();
                show_diag.last_search_ms = now;
                show_diag.last_status = "language_failover".to_string();
                show_diag.summary_message = pack_msg;
                Self::save_state(state, &ag_state).await;

                return Ok(json!({
                    "success": true,
                    "mode": "auto",
                    "isSeasonPack": true,
                    "linkedEpisodes": linked_eps,
                    "grabbed": {
                        "title": best_pack.title,
                        "protocol": best_pack.protocol,
                        "quality": best_pack.quality,
                        "language": best_pack.language,
                        "score": best_pack.score,
                        "castilianConfidence": castilian_confidence_level(&best_pack),
                        "downloadId": pack_dl_id,
                    },
                    "candidates": candidates_json,
                    "blacklistedCount": blacklisted_count,
                }));
            }

            // Respaldo episodio a episodio en Castellano estricto si no hay pack de temporada
            let mut grabbed_eps = Vec::new();
            for ep in &season_eps {
                let ep_candidates = Self::search_strict_castilian_candidates(
                    state,
                    aggregator,
                    &ag_state,
                    &item,
                    Some(ep),
                )
                .await;
                if let Some(best_ep) = ep_candidates.into_iter().next() {
                    if let Ok(dl_id) =
                        Self::grab_release_for_target(state, media_id, Some(&ep.id), &best_ep).await
                    {
                        grabbed_eps.push(json!({
                            "episodeId": ep.id,
                            "season": ep.season_number,
                            "episode": ep.episode_number,
                            "title": best_ep.title,
                            "quality": best_ep.quality,
                            "language": best_ep.language,
                            "score": best_ep.score,
                            "downloadId": dl_id,
                        }));
                    }
                }
            }

            if !grabbed_eps.is_empty() {
                return Ok(json!({
                    "success": true,
                    "mode": "auto",
                    "isSeasonPack": false,
                    "grabbedEpisodes": grabbed_eps,
                    "blacklistedCount": blacklisted_count,
                }));
            }

            return Ok(json!({
                "success": false,
                "mode": "auto",
                "message": "No se encontró ninguna alternativa verificada en Castellano de España para esta temporada. Los archivos actuales se mantienen intactos.",
                "candidates": [],
                "blacklistedCount": blacklisted_count,
            }));
        }

        // Caso 2: Película o Episodio concreto
        let ep_opt = if let Some(ep_id) = episode_id {
            Some(
                episodes
                    .iter()
                    .find(|e| e.id == ep_id)
                    .cloned()
                    .ok_or_else(|| format!("Episode '{ep_id}' not found for media '{media_id}'"))?,
            )
        } else {
            None
        };

        let candidates = Self::search_strict_castilian_candidates(
            state,
            aggregator,
            &ag_state,
            &item,
            ep_opt.as_ref(),
        )
        .await;
        let candidates_json: Vec<Value> = candidates.iter().map(Self::candidate_to_json).collect();

        if !auto_grab {
            return Ok(json!({
                "success": true,
                "mode": "preview",
                "recommendedIndex": if candidates_json.is_empty() { Value::Null } else { json!(0) },
                "candidates": candidates_json,
                "blacklistedCount": blacklisted_count,
            }));
        }

        let target_id = ep_opt
            .as_ref()
            .map(|e| e.id.as_str())
            .unwrap_or(item.id.as_str());
        let now = now_ms();

        if let Some(best) = candidates.into_iter().next() {
            let dl_id =
                Self::grab_release_for_target(state, media_id, episode_id, &best).await?;
            ag_state = Self::load_state(state).await;
            if let Some(diag) = ag_state.diagnostics.get_mut(target_id) {
                diag.last_status = "language_failover".to_string();
                diag.summary_message = format!(
                    "Cambio a Castellano: '{}' [{} · {} · {} pts]",
                    best.title, best.quality, best.language, best.score
                );
            }
            Self::save_state(state, &ag_state).await;

            return Ok(json!({
                "success": true,
                "mode": "auto",
                "grabbed": {
                    "title": best.title,
                    "protocol": best.protocol,
                    "quality": best.quality,
                    "language": best.language,
                    "score": best.score,
                    "castilianConfidence": castilian_confidence_level(&best),
                    "downloadId": dl_id,
                },
                "candidates": candidates_json,
                "blacklistedCount": blacklisted_count,
            }));
        }

        let diag = ag_state.diagnostics.entry(target_id.to_string()).or_default();
        diag.last_search_ms = now;
        diag.last_status = "no_castilian_alternative".to_string();
        diag.summary_message =
            "Sin alternativa verificada en Castellano de España; se conserva el archivo actual."
                .to_string();
        Self::save_state(state, &ag_state).await;

        Ok(json!({
            "success": false,
            "mode": "auto",
            "message": "No se encontró ninguna alternativa verificada en Castellano de España. El archivo actual se mantiene intacto.",
            "candidates": [],
            "blacklistedCount": blacklisted_count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blacklist_release_matches_hash_and_normalized_title() {
        let mut ag_state = AutoGrabberState::default();
        let target_id = "media-123";
        let now = 1_700_000_000_000;

        ag_state.blacklist_release(
            target_id,
            Some("abc123def"),
            "Pelicula.Ejemplo.(2024).1080p.Latino.mkv",
            WRONG_LANGUAGE_REASON,
            now,
        );

        assert!(ag_state.is_release_blacklisted(target_id, "ABC123DEF", "Otro.Nombre.mkv"));
        assert!(ag_state.is_release_blacklisted(
            target_id,
            "otrohash999",
            "Pelicula Ejemplo (2024) 1080p Latino mkv"
        ));
        assert!(!ag_state.is_release_blacklisted(
            target_id,
            "otrohash999",
            "Pelicula.Ejemplo.(2024).1080p.Castellano.mkv"
        ));

        // Sin hash conocido: genera clave sintética y sigue bloqueando por título normalizado
        ag_state.blacklist_release(
            "ep-456",
            None,
            "Mi.Serie.S01E01.720p.English.mkv",
            WRONG_LANGUAGE_REASON,
            now,
        );
        assert!(ag_state.is_release_blacklisted(
            "ep-456",
            "",
            "Mi Serie S01E01 720p English mkv"
        ));
    }
}
