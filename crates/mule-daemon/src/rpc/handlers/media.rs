// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use mule_core::model::{EpisodeItem, MediaItem};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::JsonRpcError;
use crate::services::downloads::now_ms;
use crate::state::AppState;

/// Busca películas o series de TV en los proveedores de metadatos (proxies Radarr/Sonarr/TMDB)
pub async fn handle_media_search(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let query = params["query"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'query' parameter"))?;
    let media_type = params["type"].as_str().unwrap_or("movie");

    let results = match media_type {
        "tv" | "series" | "show" => {
            let mut res = state.media_metadata.search_shows(query).await;
            if res.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
                if let Ok(movies) = state.media_metadata.search_movies(query).await {
                    if !movies.is_empty() {
                        res = Ok(movies);
                    }
                }
            }
            res
        }
        _ => {
            let mut res = state.media_metadata.search_movies(query).await;
            if res.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
                if let Ok(shows) = state.media_metadata.search_shows(query).await {
                    if !shows.is_empty() {
                        res = Ok(shows);
                    }
                }
            }
            res
        }
    };

    match results {
        Ok(items) => serde_json::to_value(items)
            .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}"))),
        Err(e) => Err(JsonRpcError::business(format!("Metadata search error: {e}"))),
    }
}

/// Lista los elementos de la biblioteca multimedia local (películas y series)
pub async fn handle_media_list(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let filter_type = params.as_ref().and_then(|p| p["type"].as_str());
    let filter_status = params.as_ref().and_then(|p| p["status"].as_str());

    let dl_svc = state.downloads.lock().await;
    let items = dl_svc
        .db()
        .list_media_items(filter_type, filter_status)
        .map_err(|e| JsonRpcError::business(format!("Database error: {e}")))?;

    serde_json::to_value(items)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Obtiene un único elemento multimedia con su lista completa de episodios (para series) y créditos/metadatos detallados
pub async fn handle_media_get(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;

    let (mut item, episodes) = {
        let dl_svc = state.downloads.lock().await;
        let item = dl_svc
            .db()
            .get_media_item(id)
            .map_err(|e| JsonRpcError::business(format!("Database error: {e}")))?
            .ok_or_else(|| JsonRpcError::business(format!("Media item '{id}' not found")))?;

        let episodes = dl_svc
            .db()
            .get_episodes_for_media(id)
            .unwrap_or_default();
        (item, episodes)
    };

    let details = state.media_metadata.get_extended_details(&item).await;

    let mut needs_db_update = false;
    if item.backdrop_path.is_none() && details.backdrop_url.is_some() {
        item.backdrop_path = details.backdrop_url.clone();
        needs_db_update = true;
    }
    if item.poster_path.is_none() && details.poster_url.is_some() {
        item.poster_path = details.poster_url.clone();
        needs_db_update = true;
    }
    if item.tmdb_id.is_none() && details.tmdb_id.is_some() {
        item.tmdb_id = details.tmdb_id;
        needs_db_update = true;
    }
    if item.tvdb_id.is_none() && details.tvdb_id.is_some() {
        item.tvdb_id = details.tvdb_id;
        needs_db_update = true;
    }
    if item.imdb_id.is_none() && details.imdb_id.is_some() {
        item.imdb_id = details.imdb_id.clone();
        needs_db_update = true;
    }
    // Si el título registrado quedó en inglés y existe una traducción oficial en español
    if item.title.eq_ignore_ascii_case(item.original_title.as_deref().unwrap_or("")) {
        let es_variants = crate::services::scorer::spanish_title_search_variants(
            &item.title,
            item.original_title.as_deref(),
        );
        if let Some(es_title) = es_variants.first() {
            item.title = es_title.clone();
            needs_db_update = true;
        } else if let Some(alt) = details.alternative_titles.iter().find(|t| {
            let lower = t.to_lowercase();
            lower.starts_with("el ") || lower.starts_with("la ") || lower.starts_with("los ") || lower.starts_with("las ")
        }) {
            item.title = alt.clone();
            needs_db_update = true;
        }
    }
    if needs_db_update {
        let dl_svc = state.downloads.lock().await;
        let _ = dl_svc.db().insert_media_item(&item);
    }

    let (subtitles, media_probe) = if let Some(fp) = item.file_path.as_deref() {
        let path = std::path::Path::new(fp);
        let mut subs = crate::services::SubtitleService::detect_existing_subtitles_async(path).await;
        let probe = crate::services::media_probe::probe_video_file(path).await;
        if probe.has_spanish_subtitles && !subs.iter().any(|s| s == "es") {
            subs.push("es".to_string());
        }
        (subs, Some(probe))
    } else {
        (Vec::new(), None)
    };

    let auto_grab = crate::services::AutoGrabber::get_diagnostics_for_media(state, id, &episodes).await;

    Ok(json!({
        "item": item,
        "episodes": episodes,
        "details": details,
        "subtitles": subtitles,
        "mediaProbe": media_probe,
        "autoGrab": auto_grab
    }))
}

/// Añade una película o serie de TV a la biblioteca (marcada como 'buscada' y monitorizada)
pub async fn handle_media_add(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let title = params["title"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'title' parameter"))?;
    let media_type = params["mediaType"]
        .as_str()
        .unwrap_or("movie")
        .to_string();

    let tmdb_id = params["tmdbId"].as_u64();
    let tvdb_id = params["tvdbId"].as_u64();
    let imdb_id = params["imdbId"].as_str().map(String::from);
    let original_title = params["originalTitle"].as_str().map(String::from);
    let year = params["year"].as_u64().map(|y| y as u32);
    let overview = params["overview"].as_str().map(String::from);
    let poster_path = params["posterPath"]
        .as_str()
        .or_else(|| params["posterUrl"].as_str())
        .map(String::from);
    let backdrop_path = params["backdropPath"]
        .as_str()
        .or_else(|| params["backdropUrl"].as_str())
        .map(String::from);
    let quality_profile = params["qualityProfile"]
        .as_str()
        .unwrap_or("1080p-es")
        .to_string();

    let default_root = if media_type == "tv" {
        state.config.series_dir.to_string_lossy().to_string()
    } else {
        state.config.movies_dir.to_string_lossy().to_string()
    };
    let root_folder = params["rootFolder"]
        .as_str()
        .map(String::from)
        .unwrap_or(default_root);

    let id = Uuid::new_v4().to_string();
    let now = now_ms();

    // Obtener episodios antes de adquirir el bloqueo si es una serie de TV
    let episodes_to_insert = if media_type == "tv" {
        if let Some(tv_id) = tvdb_id {
            state.media_metadata.get_show_episodes(tv_id).await.ok()
        } else {
            None
        }
    } else {
        None
    };

    let item = MediaItem {
        id: id.clone(),
        tmdb_id,
        tvdb_id,
        imdb_id,
        media_type: media_type.clone(),
        title: title.to_string(),
        original_title,
        year,
        overview,
        poster_path,
        backdrop_path,
        status: "wanted".to_string(),
        monitored: params["monitored"].as_bool().unwrap_or(true),
        quality_profile,
        root_folder,
        file_path: None,
        download_id: None,
        created_at: now,
        updated_at: now,
    };

    let mut dl_svc = state.downloads.lock().await;
    dl_svc
        .db()
        .insert_media_item(&item)
        .map_err(|e| JsonRpcError::business(format!("Database insert error: {e}")))?;

    if let Some(episodes) = episodes_to_insert {
        let ep_items: Vec<EpisodeItem> = episodes
            .into_iter()
            .map(|(s, ep, ep_title, air_date)| EpisodeItem {
                id: Uuid::new_v4().to_string(),
                media_id: id.clone(),
                season_number: s,
                episode_number: ep,
                title: ep_title,
                air_date,
                status: "wanted".to_string(),
                monitored: true,
                file_path: None,
                download_id: None,
                created_at: now,
                updated_at: now,
            })
            .collect();
        let _ = dl_svc.insert_episodes_batch(&ep_items);
    }

    serde_json::to_value(&item)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Elimina una película o serie de TV de la biblioteca (con cancelación opcional de descargas y limpieza de archivos)
pub async fn handle_media_delete(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;

    let cancel_downloads = params["cancelDownloads"].as_bool().unwrap_or(true);
    let delete_files = params["deleteFiles"].as_bool().unwrap_or(false);

    let mut files_to_delete = Vec::new();
    let mut downloads_to_remove = Vec::new();
    let mut item_info = None;

    {
        let dl_svc = state.downloads.lock().await;
        if let Ok(Some(item)) = dl_svc.db().get_media_item(id) {
            item_info = Some((
                item.media_type.clone(),
                item.title.clone(),
                item.tvdb_id,
                item.tmdb_id,
                item.root_folder.clone(),
            ));

            if let Some(ref fp) = item.file_path {
                files_to_delete.push(std::path::PathBuf::from(fp));
            }
            if let Some(ref dl_id) = item.download_id {
                if let Ok(uuid) = uuid::Uuid::parse_str(dl_id) {
                    downloads_to_remove.push(uuid);
                }
            }
            if let Ok(episodes) = dl_svc.db().get_episodes_for_media(id) {
                for ep in episodes {
                    if let Some(ref fp) = ep.file_path {
                        files_to_delete.push(std::path::PathBuf::from(fp));
                    }
                    if let Some(ref dl_id) = ep.download_id {
                        if let Ok(uuid) = uuid::Uuid::parse_str(dl_id) {
                            downloads_to_remove.push(uuid);
                        }
                    }
                }
            }
        }
    }

    if cancel_downloads && !downloads_to_remove.is_empty() {
        let mut dl_svc = state.downloads.lock().await;
        for dl_id in downloads_to_remove {
            let _ = dl_svc.remove_download(&dl_id, delete_files);
        }
    }

    if delete_files {
        if let Some((_, ref title, _, _, ref root_folder)) = item_info {
            let root_p = std::path::PathBuf::from(root_folder);
            if root_p.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&root_p) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if name.to_lowercase().contains(&title.to_lowercase())
                                || title.to_lowercase().contains(&name.to_lowercase())
                            {
                                files_to_delete.push(path);
                            }
                        }
                    }
                }
            }
        }
    }

    if delete_files && !files_to_delete.is_empty() {
        tokio::task::spawn_blocking(move || {
            for f in files_to_delete {
                if f.is_file() {
                    let _ = std::fs::remove_file(&f);
                } else if f.is_dir() {
                    let _ = std::fs::remove_dir_all(&f);
                }
            }
        })
        .await
        .ok();
    }

    let dl_svc = state.downloads.lock().await;
    let deleted = dl_svc
        .db()
        .delete_media_item(id)
        .map_err(|e| JsonRpcError::business(format!("Database error: {e}")))?;

    // Sincronizar eliminación con Sonarr / Radarr para evitar nuevas descargas
    if let Some((mtype, title, tvdb, tmdb, _)) = item_info {
        sync_delete_with_servarr(state, &mtype, &title, tvdb, tmdb, delete_files).await;
    }

    Ok(json!({ "success": deleted }))
}

async fn sync_delete_with_servarr(
    state: &AppState,
    media_type: &str,
    title: &str,
    tvdb_id: Option<u64>,
    tmdb_id: Option<u64>,
    delete_files: bool,
) {
    if state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
        return;
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    if media_type == "tv" {
        let Some(ref api_key) = state.config.sonarr_api_key else {
            return;
        };
        let url = format!(
            "{}/api/v3/series",
            state.config.sonarr_url.trim_end_matches('/')
        );
        if let Ok(resp) = client.get(&url).header("X-Api-Key", api_key).send().await {
            if let Ok(bytes) = resp.bytes().await {
                if let Ok(series_list) = serde_json::from_slice::<Vec<Value>>(&bytes) {
                    for s in series_list {
                        let s_id = s["id"].as_i64();
                        let s_tvdb = s["tvdbId"].as_u64();
                        let s_title = s["title"].as_str().unwrap_or("");
                        let matches_tvdb = tvdb_id.is_some() && tvdb_id == s_tvdb;
                        let matches_title = !title.is_empty()
                            && (s_title.eq_ignore_ascii_case(title)
                                || title.to_lowercase().contains(&s_title.to_lowercase())
                                || s_title.to_lowercase().contains(&title.to_lowercase()));
                        if matches_tvdb || matches_title {
                            if let Some(series_id) = s_id {
                                let del_url = format!(
                                    "{url}/{series_id}?deleteFiles={delete_files}&addImportListExclusion=false"
                                );
                                let _ = client
                                    .delete(&del_url)
                                    .header("X-Api-Key", api_key)
                                    .send()
                                    .await;
                                tracing::info!(
                                    "Synced deletion of TV series '{title}' (id={series_id}) with Sonarr"
                                );
                            }
                        }
                    }
                }
            }
        }
    } else {
        let Some(ref api_key) = state.config.radarr_api_key else {
            return;
        };
        let url = format!(
            "{}/api/v3/movie",
            state.config.radarr_url.trim_end_matches('/')
        );
        if let Ok(resp) = client.get(&url).header("X-Api-Key", api_key).send().await {
            if let Ok(bytes) = resp.bytes().await {
                if let Ok(movie_list) = serde_json::from_slice::<Vec<Value>>(&bytes) {
                    for m in movie_list {
                        let m_id = m["id"].as_i64();
                        let m_tmdb = m["tmdbId"].as_u64();
                        let m_title = m["title"].as_str().unwrap_or("");
                        let matches_tmdb = tmdb_id.is_some() && tmdb_id == m_tmdb;
                        let matches_title = !title.is_empty()
                            && (m_title.eq_ignore_ascii_case(title)
                                || title.to_lowercase().contains(&m_title.to_lowercase())
                                || m_title.to_lowercase().contains(&title.to_lowercase()));
                        if matches_tmdb || matches_title {
                            if let Some(movie_id) = m_id {
                                let del_url = format!(
                                    "{url}/{movie_id}?deleteFiles={delete_files}&addImportExclusion=false"
                                );
                                let _ = client
                                    .delete(&del_url)
                                    .header("X-Api-Key", api_key)
                                    .send()
                                    .await;
                                tracing::info!(
                                    "Synced deletion of movie '{title}' (id={movie_id}) with Radarr"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Estadísticas de la biblioteca: buscados, descargando, completados
pub async fn handle_media_stats(state: &AppState) -> Result<Value, JsonRpcError> {
    let dl_svc = state.downloads.lock().await;
    let items = dl_svc
        .db()
        .list_media_items(None, None)
        .unwrap_or_default();

    let mut wanted = 0usize;
    let mut downloading = 0usize;
    let mut completed = 0usize;
    let mut movies = 0usize;
    let mut tv = 0usize;

    for item in &items {
        if item.media_type == "tv" {
            tv += 1;
        } else {
            movies += 1;
        }

        match item.status.as_str() {
            "wanted" => wanted += 1,
            "downloading" => downloading += 1,
            "completed" => completed += 1,
            _ => {}
        }
    }

    Ok(json!({
        "total": items.len(),
        "movies": movies,
        "tvShows": tv,
        "wanted": wanted,
        "downloading": downloading,
        "completed": completed
    }))
}

/// Busca todos los lanzamientos en Kad y en indexadores Torznab externos para un elemento de la biblioteca
pub async fn handle_media_search_releases(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let media_id = params["mediaId"].as_str().unwrap_or("");
    let query_param = params["query"].as_str().unwrap_or("");

    let (queries, is_movie, quality_profile, item_ctx) = if !media_id.is_empty() {
        let dl_svc = state.downloads.lock().await;
        let item = dl_svc
            .db()
            .get_media_item(media_id)
            .map_err(|e| JsonRpcError::business(e.to_string()))?
            .ok_or_else(|| JsonRpcError::business(format!("Media item '{media_id}' not found")))?;

        let ext_details = state.media_metadata.get_extended_details(&item).await;

        let mut q_list = Vec::new();
        if !query_param.is_empty() {
            q_list.push(query_param.to_string());
        } else {
            if let Some(year) = item.year {
                q_list.push(format!("{} {year}", item.title));
            }
            q_list.push(item.title.clone());
            if let Some(ref orig) = item.original_title {
                if !orig.is_empty() && !orig.eq_ignore_ascii_case(&item.title) {
                    if let Some(year) = item.year {
                        q_list.push(format!("{orig} {year}"));
                    }
                    q_list.push(orig.clone());
                }
            }
            for es_t in crate::services::scorer::spanish_title_search_variants(
                &item.title,
                item.original_title.as_deref(),
            ) {
                if let Some(year) = item.year {
                    q_list.push(format!("{es_t} {year}"));
                }
                q_list.push(es_t);
            }
            for alt_title in &ext_details.alternative_titles {
                if let Some(year) = item.year {
                    q_list.push(format!("{alt_title} {year}"));
                }
                q_list.push(alt_title.clone());
            }
            if item.title.contains(':') {
                let parts: Vec<&str> = item.title.split(':').collect();
                if parts.len() >= 2 {
                    let prefix = parts[0].trim();
                    let suffix = parts[1].trim();
                    if !prefix.is_empty() {
                        if let Some(year) = item.year {
                            q_list.push(format!("{prefix} {year}"));
                        }
                        q_list.push(prefix.to_string());
                    }
                    if !suffix.is_empty() {
                        if let Some(year) = item.year {
                            q_list.push(format!("{suffix} {year}"));
                        }
                        q_list.push(suffix.to_string());
                    }
                }
            }
        }
        let ctx = Some((item.title, item.original_title, ext_details.alternative_titles, item.year));
        (q_list, item.media_type == "movie", item.quality_profile, ctx)
    } else if !query_param.is_empty() {
        let is_m = params["isMovie"].as_bool().unwrap_or(true);
        (vec![query_param.to_string()], is_m, "1080p-es".to_string(), None)
    } else {
        return Err(JsonRpcError::invalid_params("Must provide either 'mediaId' or 'query'"));
    };

    let aggregator = crate::services::IndexerAggregator::new();
    let mut all_releases = Vec::new();
    let mut seen_links = std::collections::HashSet::new();

    for q in queries {
        let rels = aggregator.search_all(state, &q, is_movie, &quality_profile).await;
        for mut r in rels {
            if seen_links.insert(r.link.clone()) {
                if !r.is_rejected {
                    if let Some((ref exp_title, ref exp_orig, ref alts, exp_year)) = item_ctx {
                        let alt_refs: Vec<&str> = alts.iter().map(|s| s.as_str()).collect();
                        if is_movie
                            && !crate::services::scorer::matches_release_with_alts(
                                &r.title,
                                exp_title,
                                exp_orig.as_deref(),
                                &alt_refs,
                                exp_year,
                                true,
                                None,
                            )
                        {
                            r.is_rejected = true;
                            r.score = -500;
                            r.rejection_reason = Some(format!(
                                "No coincide con '{}'{}",
                                exp_title,
                                exp_year.map(|y| format!(" ({y})")).unwrap_or_default()
                            ));
                        }
                    }
                }
                all_releases.push(r);
            }
        }
    }

    all_releases.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| b.sources.cmp(&a.sources)));

    serde_json::to_value(all_releases)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Descarga manualmente un candidato de lanzamiento específico para un elemento multimedia
pub async fn handle_media_grab_release(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let media_id = params["mediaId"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'mediaId' parameter"))?;
    let release_val = params["release"].clone();
    let release: crate::services::scorer::ScoredRelease = serde_json::from_value(release_val)
        .map_err(|e| JsonRpcError::invalid_params(&format!("Invalid release object: {e}")))?;

    match crate::services::AutoGrabber::grab_release(state, media_id, &release).await {
        Ok(dl_id) => Ok(json!({ "success": true, "downloadId": dl_id })),
        Err(e) => Err(JsonRpcError::business(e)),
    }
}

/// Desencadena un escaneo bajo demanda de directorios locales (/cineseries/Peliculas y /Series)
pub async fn handle_media_scan_disk(state: &AppState) -> Result<Value, JsonRpcError> {
    let summary = crate::services::DiskScanner::scan_all(state).await;
    Ok(summary)
}

/// Desencadena una pasada bajo demanda del programador de auto-descarga (global o para un medio/episodio/lista negra específico)
pub async fn handle_media_auto_grab(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    if let Some(ref p) = params {
        if let Some(clear_id) = p["clearBlacklistId"].as_str() {
            if !clear_id.is_empty() {
                return Ok(crate::services::AutoGrabber::clear_blacklist(state, clear_id).await);
            }
        }
        if let Some(media_id) = p["mediaId"].as_str() {
            if !media_id.is_empty() {
                let ep_id = p["episodeId"].as_str().filter(|s| !s.is_empty());
                let aggregator = crate::services::IndexerAggregator::new();
                return crate::services::AutoGrabber::grab_for_media_id(
                    state,
                    &aggregator,
                    media_id,
                    ep_id,
                )
                .await
                .map_err(JsonRpcError::business);
            }
        }
    }
    let force = params
        .as_ref()
        .and_then(|p| p.get("force").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let aggregator = crate::services::IndexerAggregator::new();
    let res = crate::services::AutoGrabber::run_auto_grab_round(state, &aggregator, force).await;
    Ok(res)
}

/// Lista todos los indexadores Torznab externos configurados
pub async fn handle_indexer_list(state: &AppState) -> Result<Value, JsonRpcError> {
    let dl_svc = state.downloads.lock().await;
    let list = dl_svc
        .db()
        .list_external_indexers()
        .map_err(|e| JsonRpcError::business(e.to_string()))?;
    serde_json::to_value(list)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Añade un indexador Torznab externo (ej.: feed de Jackett / Prowlarr)
pub async fn handle_indexer_add(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let name = params["name"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'name' parameter"))?;
    let base_url = params["baseUrl"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'baseUrl' parameter"))?;
    let api_key = params["apiKey"].as_str().unwrap_or("");
    let enabled = params["enabled"].as_bool().unwrap_or(true);

    let indexer = mule_core::model::ExternalIndexer {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        enabled,
        created_at: now_ms(),
    };

    let dl_svc = state.downloads.lock().await;
    dl_svc
        .db()
        .insert_external_indexer(&indexer)
        .map_err(|e| JsonRpcError::business(e.to_string()))?;

    serde_json::to_value(&indexer)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Elimina un indexador externo
pub async fn handle_indexer_delete(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let id = params["id"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'id' parameter"))?;

    let dl_svc = state.downloads.lock().await;
    let ok = dl_svc
        .db()
        .delete_external_indexer(id)
        .map_err(|e| JsonRpcError::business(e.to_string()))?;

    Ok(json!({ "success": ok }))
}

/// Desencadena la pasada de organización para enlazar (hardlink) descargas completadas en la biblioteca multimedia
pub async fn handle_media_organize(
    state: &AppState,
    _params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let result = crate::services::MediaOrganizer::sweep_and_organize(state).await;
    Ok(result)
}

/// Busca candidatos de subtítulos para un elemento multimedia (funcionalidad estilo Bazarr)
pub async fn handle_subtitles_search(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let lang = params["lang"].as_str().unwrap_or("spa");

    let (video_path, title, year, imdb_id, is_movie, season_ep) = if let Some(media_id) = params["mediaId"].as_str() {
        let dl_svc = state.downloads.lock().await;
        let item = dl_svc
            .db()
            .get_media_item(media_id)
            .map_err(|e| JsonRpcError::business(e.to_string()))?
            .ok_or_else(|| JsonRpcError::invalid_params("Media item not found"))?;

        let vp = item.file_path.map(std::path::PathBuf::from);
        (vp, item.title, item.year, item.imdb_id, item.media_type == "movie", None)
    } else {
        let title = params["title"]
            .as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("Missing 'mediaId' or 'title'"))?
            .to_string();
        let year = params["year"].as_u64().map(|y| y as u32);
        let imdb = params["imdbId"].as_str().map(|s| s.to_string());
        let is_movie = params["isMovie"].as_bool().unwrap_or(true);
        let season_ep = match (params["season"].as_u64(), params["episode"].as_u64()) {
            (Some(s), Some(e)) => Some((s as u32, e as u32)),
            _ => None,
        };
        let vp = params["path"].as_str().map(std::path::PathBuf::from);
        (vp, title, year, imdb, is_movie, season_ep)
    };

    let candidates = crate::services::SubtitleService::search_subtitles(
        video_path.as_deref(),
        &title,
        year,
        imdb_id.as_deref(),
        is_movie,
        season_ep,
        lang,
    )
    .await
    .map_err(JsonRpcError::business)?;

    serde_json::to_value(candidates)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Descarga un candidato de subtítulo específico
pub async fn handle_subtitles_download(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let download_link = params["downloadLink"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'downloadLink'"))?;
    let lang = params["lang"].as_str().unwrap_or("es");

    let target_path = if let Some(target) = params["targetPath"].as_str() {
        std::path::PathBuf::from(target)
    } else if let Some(media_id) = params["mediaId"].as_str() {
        let dl_svc = state.downloads.lock().await;
        let item = dl_svc
            .db()
            .get_media_item(media_id)
            .map_err(|e| JsonRpcError::business(e.to_string()))?
            .ok_or_else(|| JsonRpcError::invalid_params("Media item not found"))?;

        let vp = item
            .file_path
            .ok_or_else(|| JsonRpcError::business("Media item has no local file path yet"))?;
        let video_p = std::path::PathBuf::from(vp);
        video_p.with_extension(format!("{lang}.srt"))
    } else {
        return Err(JsonRpcError::invalid_params("Missing 'targetPath' or 'mediaId'"));
    };

    crate::services::SubtitleService::download_and_save_subtitle(download_link, &target_path)
        .await
        .map_err(JsonRpcError::business)?;

    crate::services::MediaOrganizer::notify_media_servers(state);

    Ok(json!({
        "success": true,
        "savedPath": target_path.to_string_lossy()
    }))
}

/// Comprueba subtítulos acompañantes existentes para un elemento multimedia
pub async fn handle_subtitles_check(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params object"))?;
    let media_id = params["mediaId"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Missing 'mediaId'"))?;

    let dl_svc = state.downloads.lock().await;
    let item = dl_svc
        .db()
        .get_media_item(media_id)
        .map_err(|e| JsonRpcError::business(e.to_string()))?
        .ok_or_else(|| JsonRpcError::invalid_params("Media item not found"))?;

    let langs = if let Some(ref fp) = item.file_path {
        crate::services::SubtitleService::detect_existing_subtitles(std::path::Path::new(fp))
    } else {
        Vec::new()
    };

    Ok(json!({
        "mediaId": media_id,
        "subtitles": langs
    }))
}

/// Obtiene automáticamente todos los subtítulos faltantes para elementos de la biblioteca
pub async fn handle_subtitles_auto_fetch(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let lang = params
        .as_ref()
        .and_then(|p| p["lang"].as_str())
        .unwrap_or("spa");

    let result = crate::services::SubtitleService::auto_fetch_all_missing(state, lang).await;
    Ok(result)
}

/// Actualiza y enriquece manual o automáticamente metadatos y pósteres de elementos de la biblioteca
pub async fn handle_media_refresh_metadata(state: &AppState) -> Result<Value, JsonRpcError> {
    let count = crate::services::DiskScanner::enrich_library(state).await;
    Ok(json!({ "itemsEnriched": count }))
}

/// Obtiene el calendario de episodios filtrado por rango de fechas
pub async fn handle_media_calendar(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let start_date = params.as_ref().and_then(|p| p["startDate"].as_str());
    let end_date = params.as_ref().and_then(|p| p["endDate"].as_str());

    let dl_svc = state.downloads.lock().await;
    let episodes = dl_svc
        .db()
        .get_calendar_episodes(start_date, end_date)
        .map_err(|e| JsonRpcError::business(format!("Database error: {e}")))?;

    serde_json::to_value(episodes)
        .map_err(|e| JsonRpcError::business(format!("Serialization error: {e}")))
}

/// Ejecuta el limpiador de duplicados y residuos en la biblioteca y en los directorios de entrada
pub async fn handle_media_clean(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let dry_run = params
        .as_ref()
        .and_then(|p| p["dryRun"].as_bool())
        .unwrap_or(false);

    // Ejecutar barrido y organización primero para que las descargas entrantes queden enlazadas
    let org_res = crate::services::MediaOrganizer::sweep_and_organize(state).await;

    // Ejecutar deduplicación profunda y limpiador de residuos/basura
    let mut clean_res = crate::services::MediaCleaner::clean_all(state, dry_run).await;
    if let Some(obj) = clean_res.as_object_mut() {
        obj.insert("organizedBeforeClean".to_string(), org_res["organized"].clone());
    }

    Ok(clean_res)
}



