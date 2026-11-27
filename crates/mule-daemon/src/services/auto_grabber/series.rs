// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::*;

/// Número máximo de episodios buscados en una pasada forzada para evitar bloqueos largos en series extensas.
const MAX_FORCED_EPISODES_PER_PASS: usize = 24;
/// Número máximo de episodios buscados en una pasada automática en segundo plano.
const MAX_AUTO_EPISODES_PER_PASS: usize = 20;
/// Objetivo de episodios descargando simultáneamente por serie para mantener flujo continuo sin saturar la cola.
const TARGET_ACTIVE_SERIES_DOWNLOADS: usize = 15;
/// Concurrencia de búsquedas individuales por episodio dentro de una misma serie.
const EPISODE_SEARCH_CONCURRENCY: usize = 4;
/// Umbral mínimo de candidatos en el pool base de la serie antes de lanzar una segunda pasada de búsqueda.
const MIN_BASE_SERIES_POOL_SIZE: usize = 80;
/// Número máximo de packs de temporada candidatos intentados antes de pasar a episodios individuales.
const MAX_SEASON_PACK_ATTEMPTS: usize = 3;
/// Número mínimo de episodios HD distintos en el pool para preferirlos frente a un pack de temporada SD en perfiles HD.
const MIN_HD_EPISODES_TO_PREFER_OVER_SD_PACK: usize = 2;

fn is_hd_quality_tag(quality: &str) -> bool {
    let q = quality.trim().to_ascii_lowercase();
    q.contains("2160") || q.contains("4k") || q.contains("1080") || q.contains("720")
}

/// Comprueba si conviene omitir un pack de temporada SD en un perfil HD porque el pool ya contiene
/// suficientes episodios individuales en calidad HD con puntuación superior.
pub(super) fn should_skip_sd_pack_for_hd_pool(
    quality_profile: &str,
    pack: &ScoredRelease,
    pool: &[ScoredRelease],
    item: &MediaItem,
    alt_refs: &[&str],
    season_num: u32,
) -> usize {
    if !is_hd_quality_tag(quality_profile) || is_hd_quality_tag(&pack.quality) {
        return 0;
    }
    let mut hd_episodes: HashSet<u32> = HashSet::new();
    for r in pool {
        if r.is_rejected || r.score <= pack.score || !is_hd_quality_tag(&r.quality) {
            continue;
        }
        if !matches_release_with_options(
            &r.title,
            &item.title,
            item.original_title.as_deref(),
            alt_refs,
            item.year,
            false,
            None,
            None,
        ) {
            continue;
        }
        let nums = extract_tv_episode_numbers(&r.title, season_num);
        if nums.len() == 1 || (1..=3).contains(&nums.len()) {
            for ep_num in nums {
                if matches_release_with_options(
                    &r.title,
                    &item.title,
                    item.original_title.as_deref(),
                    alt_refs,
                    item.year,
                    false,
                    Some((season_num, ep_num)),
                    None,
                ) {
                    hd_episodes.insert(ep_num);
                }
            }
        }
    }
    hd_episodes.len()
}

impl AutoGrabber {
    /// Ejecuta la búsqueda y descarga automática para los episodios buscados de una serie de TV.
    pub(super) async fn process_series_item(
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

        let active_downloading = episodes
            .iter()
            .filter(|e| e.status == "downloading")
            .count();
        if only_episode_id.is_none()
            && !force
            && active_downloading >= TARGET_ACTIVE_SERIES_DOWNLOADS
        {
            return grabbed_episodes;
        }

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

        let season_in_progress: HashSet<u32> = episodes
            .iter()
            .filter(|e| e.status == "completed" || e.status == "downloading")
            .map(|e| e.season_number)
            .collect();

        // Priorizar episodios nunca buscados y temporadas ya iniciadas (`completed`/`downloading`)
        wanted_eps.sort_by_key(|ep| {
            let attempts = ag_state
                .diagnostics
                .get(&ep.id)
                .map(|d| d.consecutive_empty_searches)
                .unwrap_or(0);
            let not_started = !season_in_progress.contains(&ep.season_number);
            (attempts, not_started, ep.season_number, ep.episode_number)
        });

        let mut handled_ep_ids: HashSet<String> = HashSet::new();
        let search_title = strip_trailing_year_suffix(&item.title);
        let ext_details = super::load_extended_details_bounded(state, item).await;
        let alt_refs: Vec<&str> = ext_details
            .alternative_titles
            .iter()
            .map(|s| s.as_str())
            .collect();

        // 0. Búsqueda inteligente de paquetes de temporada completa si se buscan >= 3 episodios de una misma temporada
        let mut season_pack_pool: Vec<ScoredRelease> = Vec::new();
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
            candidate_seasons.sort_by_key(|&s| (!season_in_progress.contains(&s), s));

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
                    format!("{search_title} T{season_num}"),
                    format!("{search_title} castellano"),
                ];
                if let Some(ref orig) = item.original_title {
                    if !orig.is_empty() && !orig.eq_ignore_ascii_case(&item.title) {
                        let q = format!("{orig} S{season_num:02}");
                        if !pack_queries.iter().any(|p| p.eq_ignore_ascii_case(&q)) {
                            pack_queries.push(q);
                        }
                    }
                }
                for es_t in &es_variants {
                    for q in [
                        format!("{es_t} Temporada {season_num}"),
                        format!("{es_t} S{season_num:02}"),
                        format!("{es_t} T{season_num}"),
                    ] {
                        if !pack_queries.iter().any(|p| p.eq_ignore_ascii_case(&q)) {
                            pack_queries.push(q);
                        }
                    }
                }

                super::emit_autograb_progress(
                    state,
                    "searching",
                    &item.title,
                    &format!(
                        "Buscando packs de Temporada {season_num:02} en eD2k, Kad y BitTorrent..."
                    ),
                    10,
                    Some(json!({ "season": season_num, "queries": pack_queries })),
                );

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
                        season_pack_pool.push(r.clone());
                        if r.is_rejected || r.score < MIN_SEASON_PACK_SCORE {
                            continue;
                        }
                        if is_single_video_file_ed2k(&r.title, &r.protocol) {
                            continue;
                        }
                        if ag_state.is_release_blacklisted(&item.id, &r.hash, &r.title)
                            || season_eps
                                .iter()
                                .any(|ep| ag_state.is_release_blacklisted(&ep.id, &r.hash, &r.title))
                        {
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

                info!(
                    "[AUTO_GRAB] '{}' T{:02}: {} candidatos de pack analizados ({} válidos)",
                    item.title,
                    season_num,
                    seen_pack_links.len(),
                    valid_packs.len()
                );

                valid_packs.sort_by(|a, b| {
                    b.score
                        .cmp(&a.score)
                        .then_with(|| b.sources.cmp(&a.sources))
                });

                for best_pack in valid_packs.into_iter().take(MAX_SEASON_PACK_ATTEMPTS) {
                    let hd_ep_count = should_skip_sd_pack_for_hd_pool(
                        &item.quality_profile,
                        &best_pack,
                        &season_pack_pool,
                        item,
                        &alt_refs,
                        season_num,
                    );
                    if hd_ep_count >= MIN_HD_EPISODES_TO_PREFER_OVER_SD_PACK {
                        info!(
                            "[AUTO_GRAB] '{}' T{:02}: omitiendo pack SD '{}' ({} pts) porque hay {} episodios HD en el pool",
                            item.title, season_num, best_pack.title, best_pack.score, hd_ep_count
                        );
                        continue;
                    }
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
                        match dl_svc.add_ed2k_link(
                            &best_pack.link,
                            &target_dest,
                            AllocationPolicy::SparseIfSupported,
                            Some("tv".to_string()),
                        ) {
                            Ok(snap) => Some(snap.id.to_string()),
                            Err(e) => {
                                warn!(
                                    "[AUTO_GRAB] Fallo al añadir pack '{}' T{:02} (ed2k): {e}",
                                    best_pack.title, season_num
                                );
                                None
                            }
                        }
                    } else if let Ok(torrent_engine) = state.torrent_engine().await {
                        match torrent_engine
                            .add_url(&best_pack.link, Some(target_dest), Some("tv".to_string()))
                            .await
                        {
                            Ok(snap) => Some(snap.info_hash),
                            Err(e) => {
                                warn!(
                                    "[AUTO_GRAB] Fallo al añadir pack '{}' T{:02} ({}): {e}",
                                    best_pack.title, season_num, best_pack.protocol
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    if let Some(dl_id_str) = pack_dl_id {
                        let covered_nums = extract_tv_episode_numbers(&best_pack.title, season_num);
                        let mut season_linked_count = 0usize;
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
                                    if handled_ep_ids.insert(sep.id.clone()) {
                                        season_linked_count += 1;
                                    }
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
                            season_linked_count,
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
                        break;
                    }
                }
            }
        }

        let batch_limit = if only_episode_id.is_some() || force {
            MAX_FORCED_EPISODES_PER_PASS
        } else {
            MAX_AUTO_EPISODES_PER_PASS
        };
        let es_variants =
            spanish_title_search_variants(search_title, item.original_title.as_deref());
        let remaining_eps: Vec<EpisodeItem> = wanted_eps
            .into_iter()
            .filter(|e| !handled_ep_ids.contains(&e.id))
            .take(batch_limit)
            .collect();

        // Precargar los resultados de Kad/Torznab a nivel de serie (y sus variantes en español)
        // junto con el pool obtenido en las búsquedas de paquetes de temporada.
        let mut base_series_releases = Vec::new();
        let mut seen_base_links = HashSet::new();
        for r in season_pack_pool {
            if seen_base_links.insert(r.link.clone()) {
                base_series_releases.push(r);
            }
        }
        if !remaining_eps.is_empty() {
            let mut base_queries = vec![
                search_title.to_string(),
                format!("{search_title} castellano"),
                format!("{search_title} spanish"),
            ];
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
            super::emit_autograb_progress(
                state,
                "searching",
                &item.title,
                &format!(
                    "Buscando serie en eD2k, Kad, BT Público, DHT Local y Torznab ({} consultas base)...",
                    base_queries.len()
                ),
                20,
                Some(json!({ "queries": base_queries })),
            );
            let base_futs = base_queries
                .iter()
                .map(|q| aggregator.search_all(state, q, false, &item.quality_profile));
            let base_batches = join_all(base_futs).await;
            for rels in base_batches {
                for r in rels {
                    if seen_base_links.insert(r.link.clone()) {
                        base_series_releases.push(r);
                    }
                }
            }

            // Si el pool inicial es escaso (ej. pocos nodos Kad respondieron), realizar una segunda
            // pasada de fusión sobre la consulta principal para mejorar la cobertura.
            if base_series_releases.len() < MIN_BASE_SERIES_POOL_SIZE {
                aggregator.invalidate_cache().await;
                let retry_rels = aggregator
                    .search_all(state, search_title, false, &item.quality_profile)
                    .await;
                for r in retry_rels {
                    if seen_base_links.insert(r.link.clone()) {
                        base_series_releases.push(r);
                    }
                }
            }
        }

        // Ejecutar búsquedas individuales por lotes concurrentes (solo para episodios sin coincidencia previa en el pool)
        let pending_eps: Vec<EpisodeItem> = remaining_eps
            .into_iter()
            .filter(|ep| !handled_ep_ids.contains(&ep.id))
            .collect();

        for ep_chunk in pending_eps.chunks(EPISODE_SEARCH_CONCURRENCY) {
            if only_episode_id.is_none()
                && (active_downloading + grabbed_episodes.len()) >= TARGET_ACTIVE_SERIES_DOWNLOADS
            {
                break;
            }
            let chunk_futs = ep_chunk.iter().map(|ep| {
                let ep_meta_key = format!("S{:02}E{:02}", ep.season_number, ep.episode_number);
                let abs_num = if ext_details.is_anime {
                    ext_details
                        .episode_meta
                        .get(&ep_meta_key)
                        .and_then(|m| m.absolute_episode_number)
                } else {
                    None
                };
                let has_pool_hit = base_series_releases.iter().any(|r| {
                    !r.is_rejected
                        && r.score >= 15
                        && !ag_state.is_release_blacklisted(&ep.id, &r.hash, &r.title)
                        && matches_release_with_options(
                            &r.title,
                            &item.title,
                            item.original_title.as_deref(),
                            &alt_refs,
                            item.year,
                            false,
                            Some((ep.season_number, ep.episode_number)),
                            abs_num,
                        )
                });
                let es_vars = &es_variants;
                async move {
                    if has_pool_hit {
                        return Vec::new();
                    }
                    let mut ep_queries = vec![
                        format!("{search_title} S{:02}E{:02}", ep.season_number, ep.episode_number),
                        format!("{search_title} {}x{:02}", ep.season_number, ep.episode_number),
                    ];
                    if ep.episode_number >= 100 {
                        ep_queries.push(format!(
                            "{search_title} {:02}x{:03}",
                            ep.season_number, ep.episode_number
                        ));
                    }
                    if ep.season_number >= 1 && ep.episode_number < 100 {
                        let cap_code = ep.season_number * 100 + ep.episode_number;
                        ep_queries.push(format!("{search_title} Cap.{cap_code}"));
                    }
                    if let Some(abs) = abs_num {
                        ep_queries.push(format!("{search_title} {abs:03}"));
                        if abs != ep.episode_number || ep.season_number > 1 {
                            ep_queries.push(format!("{search_title} 01x{abs:03}"));
                            ep_queries.push(format!("{search_title} 1x{abs:02}"));
                        }
                    }
                    for es_t in es_vars {
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
                    join_all(ep_futs).await
                }
            });
            let chunk_batches = join_all(chunk_futs).await;

            for (ep, ep_batches) in ep_chunk.iter().zip(chunk_batches) {
                if handled_ep_ids.contains(&ep.id) {
                    continue;
                }
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
            let sources_summary = super::summarize_release_sources(&all_releases);
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
                let ep_meta_key = format!("S{:02}E{:02}", ep.season_number, ep.episode_number);
                let abs_num = if ext_details.is_anime {
                    ext_details
                        .episode_meta
                        .get(&ep_meta_key)
                        .and_then(|m| m.absolute_episode_number)
                } else {
                    None
                };
                if !matches_release_with_options(
                    &r.title,
                    &item.title,
                    item.original_title.as_deref(),
                    &alt_refs,
                    item.year,
                    false,
                    Some((ep.season_number, ep.episode_number)),
                    abs_num,
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

            let ep_label = format!("{} S{:02}E{:02}", item.title, ep.season_number, ep.episode_number);
            super::emit_autograb_progress(
                state,
                "evaluated",
                &ep_label,
                &format!(
                    "{total_candidates} candidatas analizadas → {valid_count} válidas{}",
                    if formatted_reasons.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", formatted_reasons.iter().take(2).cloned().collect::<Vec<_>>().join(" · "))
                    }
                ),
                65,
                Some(json!({
                    "sources": sources_summary,
                    "totalCandidates": total_candidates,
                    "validCandidates": valid_count,
                    "topRejections": formatted_reasons,
                })),
            );

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
                    super::emit_autograb_progress(
                        state,
                        "grabbed",
                        &ep_label,
                        &format!(
                            "Capturado '{}' [{} · {} · {} pts · {} fuentes]",
                            best.title, best.quality, best.language, best.score, best.sources
                        ),
                        90,
                        Some(json!({
                            "sources": sources_summary,
                            "protocol": best.protocol,
                            "release": best.title,
                            "quality": best.quality,
                            "language": best.language,
                            "score": best.score,
                            "peerSources": best.sources,
                            "indexerName": best.indexer_name,
                        })),
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
                show_diag.summary_message = ep_msg.clone();
                show_diag.top_rejection_reasons = formatted_reasons;
            }
            super::emit_autograb_progress(
                state,
                "no_match",
                &ep_label,
                &ep_msg,
                85,
                Some(json!({
                    "sources": sources_summary,
                    "totalCandidates": total_candidates,
                    "validCandidates": 0,
                })),
            );
            }
        }

        Self::save_state(state, ag_state).await;
        grabbed_episodes
    }
}
