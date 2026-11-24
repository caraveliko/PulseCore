// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::*;

impl AutoGrabber {
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
                    "Sustitución verificada T.{s_num:02}: '{}' [{} · {} · {} pts]",
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
                "message": "No se encontró ninguna alternativa con audio verificado para esta temporada. Los archivos actuales se mantienen intactos.",
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
                    "Sustitución verificada: '{}' [{} · {} · {} pts]",
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
            "Sin alternativa con audio verificado; se conserva el archivo actual."
                .to_string();
        Self::save_state(state, &ag_state).await;

        Ok(json!({
            "success": false,
            "mode": "auto",
            "message": "No se encontró ninguna alternativa con audio verificado. El archivo actual se mantiene intacto.",
            "candidates": [],
            "blacklistedCount": blacklisted_count,
        }))
    }
}
