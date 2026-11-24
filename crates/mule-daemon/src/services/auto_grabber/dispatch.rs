// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::*;

impl AutoGrabber {
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
    pub(super) async fn inspect_and_heal_stalled_downloads(
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
    pub(super) async fn check_movie_upgrade_eligibility(
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

    pub(super) fn compute_next_backoff_ms(now: i64, consecutive_failures: u32) -> i64 {
        let idx = (consecutive_failures.saturating_sub(1) as usize)
            .min(BACKOFF_STEPS_MS.len().saturating_sub(1));
        now + BACKOFF_STEPS_MS[idx]
    }
}
