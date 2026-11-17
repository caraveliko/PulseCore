// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::services::downloads::now_ms;
use crate::services::organizer::{clean_movie_title, extract_year};
use crate::state::AppState;
use mule_core::model::{EpisodeItem, MediaItem};

const VIDEO_EXTENSIONS: &[&str] = &[
    ".mkv", ".mp4", ".avi", ".m4v", ".mov", ".ts",
];
/// Pausa entre dos peticiones de enriquecimiento externo (en ms) para respetar los límites de tasa.
const METADATA_ENRICH_THROTTLE_MS: u64 = 80;

fn is_video_file(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    VIDEO_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Extrae tmdb_id del nombre de carpeta como "Abigail (2024) [tmdbid-1111873]"
fn extract_tmdb_id(name: &str) -> Option<u64> {
    let lower = name.to_lowercase();
    for needle in &["tmdbid-", "tmdb-", "tmdbid=", "tmdb="] {
        if let Some(pos) = lower.find(needle) {
            let start = pos + needle.len();
            let end = lower[start..]
                .find(|c: char| !c.is_numeric())
                .map(|idx| idx + start)
                .unwrap_or(lower.len());
            if let Ok(id) = lower[start..end].parse() {
                return Some(id);
            }
        }
    }
    None
}

/// Extrae tvdb_id del nombre de carpeta como "The Simpsons () [tvdbid-71663]"
fn extract_tvdb_id(name: &str) -> Option<u64> {
    let lower = name.to_lowercase();
    for needle in &["tvdbid-", "tvdb-", "tvdbid=", "tvdb="] {
        if let Some(pos) = lower.find(needle) {
            let start = pos + needle.len();
            let end = lower[start..]
                .find(|c: char| !c.is_numeric())
                .map(|idx| idx + start)
                .unwrap_or(lower.len());
            if let Ok(id) = lower[start..end].parse() {
                return Some(id);
            }
        }
    }
    None
}

/// Limpia el título eliminando (tag)/[tag] iniciales y (YYYY), [tmdbid-XXX], [tvdbid-XXX], etc. finales.
fn clean_folder_title(name: &str) -> String {
    clean_movie_title(name)
}

/// Extrae el número de temporada del nombre de directorio como "Season 01", "Temporada 2", "Season 3"
pub fn extract_season_from_folder(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    for prefix in &["season", "temporada", "temp", "s"] {
        if let Some(pos) = lower.find(prefix) {
            let rest = &lower[pos + prefix.len()..];
            let trimmed = rest.trim_start_matches([' ', '_', '-']);
            let end = trimmed.find(|c: char| !c.is_numeric()).unwrap_or(trimmed.len());
            if end > 0 {
                if let Ok(s) = trimmed[..end].parse::<u32>() {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Extrae (temporada, episodio) a partir del nombre de archivo y la pista opcional de temporada de la carpeta.
/// Gestiona caracteres Unicode adecuadamente y evita capturar letras 's' no relacionadas en títulos de series.
pub fn extract_season_episode(folder_season: Option<u32>, filename: &str) -> Option<(u32, u32)> {
    let lower = filename.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let n = chars.len();

    // 1. Escanear en busca de S(\d+)E(\d+) o S(\d+)[._-]E(\d+)
    let mut i = 0;
    while i < n {
        let is_s_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
        if chars[i] == 's' && is_s_boundary {
            let s_start = i + 1;
            let mut s_end = s_start;
            while s_end < n && chars[s_end].is_ascii_digit() {
                s_end += 1;
            }
            if s_end > s_start && (s_end - s_start) <= 4 {
                let mut e_idx = s_end;
                if e_idx < n && (chars[e_idx] == '.' || chars[e_idx] == '-' || chars[e_idx] == '_') {
                    e_idx += 1;
                }
                if e_idx < n && chars[e_idx] == 'e' {
                    let ep_start = e_idx + 1;
                    let mut ep_end = ep_start;
                    while ep_end < n && chars[ep_end].is_ascii_digit() {
                        ep_end += 1;
                    }
                    if ep_end > ep_start && (ep_end - ep_start) <= 4 {
                        let is_ep_boundary = ep_end == n || !chars[ep_end].is_alphanumeric();
                        if is_ep_boundary {
                            let s_str: String = chars[s_start..s_end].iter().collect();
                            let ep_str: String = chars[ep_start..ep_end].iter().collect();
                            if let (Ok(s), Ok(e)) = (s_str.parse::<u32>(), ep_str.parse::<u32>()) {
                                return Some((s, e));
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }

    // 2. Escanear en busca de (\d{1,2})x(\d{1,3}) ej.: "1x05", "02x08"
    i = 0;
    while i < n {
        if chars[i] == 'x' {
            let mut s_start = i;
            while s_start > 0 && chars[s_start - 1].is_ascii_digit() {
                s_start -= 1;
            }
            let s_len = i - s_start;
            let is_s_boundary = s_start == 0 || !chars[s_start - 1].is_alphanumeric();
            if (1..=2).contains(&s_len) && is_s_boundary {
                let ep_start = i + 1;
                let mut ep_end = ep_start;
                while ep_end < n && chars[ep_end].is_ascii_digit() {
                    ep_end += 1;
                }
                let ep_len = ep_end - ep_start;
                let is_ep_boundary = ep_end == n || !chars[ep_end].is_alphanumeric();
                if (1..=3).contains(&ep_len) && is_ep_boundary {
                    let s_str: String = chars[s_start..i].iter().collect();
                    let ep_str: String = chars[ep_start..ep_end].iter().collect();
                    if let (Ok(s), Ok(e)) = (s_str.parse::<u32>(), ep_str.parse::<u32>()) {
                        return Some((s, e));
                    }
                }
            }
        }
        i += 1;
    }

    // 3. Respaldo: si se conoce la temporada de la carpeta, comprobar "Episodio X", "Episode X", "Capitulo X", "Cap.X"
    if let Some(s) = folder_season {
        for marker in &["episodio", "episode", "capitulo", "cap"] {
            if let Some(pos) = lower.find(marker) {
                let rest = &lower[pos + marker.len()..];
                let trimmed = rest.trim_start_matches(['.', ' ', '_', '-']);
                let end = trimmed.find(|c: char| !c.is_numeric()).unwrap_or(trimmed.len());
                if end > 0 {
                    if let Ok(e) = trimmed[..end].parse::<u32>() {
                        return Some((s, e));
                    }
                }
            }
        }
    }

    None
}

struct ScannedMovieFolder {
    folder_name: String,
    tmdb_id: Option<u64>,
    folder_year: Option<u32>,
    clean_title: String,
    file_path_str: String,
}

struct ScannedSeriesFolder {
    folder_name: String,
    tvdb_id: Option<u64>,
    tmdb_id: Option<u64>,
    folder_year: Option<u32>,
    clean_title: String,
    episodes: Vec<(u32, u32, String, String)>,
}

fn scan_filesystem_blocking(
    movies_root: PathBuf,
    series_root: PathBuf,
) -> (Vec<ScannedMovieFolder>, Vec<ScannedSeriesFolder>) {
    let mut movie_folders = Vec::new();
    let mut series_folders = Vec::new();

    if movies_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&movies_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let folder_name = entry.file_name().to_string_lossy().to_string();
                let tmdb_id = extract_tmdb_id(&folder_name);
                let folder_year = extract_year(&folder_name);
                let clean_title = clean_folder_title(&folder_name);

                // Hallar el archivo de vídeo más grande dentro de la carpeta de la película (evita archivos pequeños de muestra/tráiler)
                let mut best_video_file: Option<(PathBuf, u64)> = None;
                if let Ok(files) = std::fs::read_dir(&path) {
                    for f in files.flatten() {
                        let fp = f.path();
                        if is_video_file(&fp) {
                            let size = fp.metadata().map(|m| m.len()).unwrap_or(0);
                            if best_video_file.as_ref().map(|(_, s)| size > *s).unwrap_or(true) {
                                best_video_file = Some((fp, size));
                            }
                        }
                    }
                }

                if let Some((file_path_buf, _)) = best_video_file {
                    movie_folders.push(ScannedMovieFolder {
                        folder_name,
                        tmdb_id,
                        folder_year,
                        clean_title,
                        file_path_str: file_path_buf.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }

    if series_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&series_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let folder_name = entry.file_name().to_string_lossy().to_string();
                let tvdb_id = extract_tvdb_id(&folder_name);
                let tmdb_id = extract_tmdb_id(&folder_name);
                let folder_year = extract_year(&folder_name);
                let clean_title = clean_folder_title(&folder_name);

                let mut episodes = Vec::new();
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub_entry in sub_entries.flatten() {
                        let sub_path = sub_entry.path();
                        if sub_path.is_dir() {
                            let dir_name = sub_entry.file_name().to_string_lossy().to_string();
                            let folder_season = extract_season_from_folder(&dir_name);
                            if let Ok(ep_files) = std::fs::read_dir(&sub_path) {
                                for ep_entry in ep_files.flatten() {
                                    let ep_path = ep_entry.path();
                                    if !is_video_file(&ep_path) {
                                        continue;
                                    }
                                    let ep_filename = ep_entry.file_name().to_string_lossy().to_string();
                                    if let Some((s_num, ep_num)) = extract_season_episode(folder_season, &ep_filename) {
                                        episodes.push((
                                            s_num,
                                            ep_num,
                                            ep_filename,
                                            ep_path.to_string_lossy().to_string(),
                                        ));
                                    }
                                }
                            }
                        } else if is_video_file(&sub_path) {
                            let ep_filename = sub_entry.file_name().to_string_lossy().to_string();
                            if let Some((s_num, ep_num)) = extract_season_episode(None, &ep_filename) {
                                episodes.push((
                                    s_num,
                                    ep_num,
                                    ep_filename,
                                    sub_path.to_string_lossy().to_string(),
                                ));
                            }
                        }
                    }
                }

                series_folders.push(ScannedSeriesFolder {
                    folder_name,
                    tvdb_id,
                    tmdb_id,
                    folder_year,
                    clean_title,
                    episodes,
                });
            }
        }
    }

    (movie_folders, series_folders)
}

pub struct DiskScanner;

impl DiskScanner {
    /// Escanea directorios de películas y series en disco (`spawn_blocking`) y sincroniza la base de datos en lote
    pub async fn scan_all(state: &AppState) -> Value {
        let now = now_ms();
        let movies_root = state.config.movies_dir.clone();
        let series_root = state.config.series_dir.clone();
        let movies_root_str = movies_root.to_string_lossy().to_string();
        let series_root_str = series_root.to_string_lossy().to_string();

        // 1. Recorrer el NAS / sistema de archivos en un hilo dedicado sin bloquear Tokio
        let (scanned_movies, scanned_series) = tokio::task::spawn_blocking(move || {
            scan_filesystem_blocking(movies_root, series_root)
        })
        .await
        .unwrap_or_default();

        let movies_scanned = scanned_movies.len();
        let mut episodes_scanned = 0usize;
        let mut updated = 0usize;

        // 2. Sincronización agrupada en la base SQLite (un único bloqueo para el escaneo completo)
        {
            let mut dl_svc = state.downloads.lock().await;
            let db = dl_svc.db_mut();
            let mut all_movies = db.list_media_items(Some("movie"), None).unwrap_or_default();

            for m_scan in scanned_movies {
                let mut existing = if let Some(tid) = m_scan.tmdb_id {
                    all_movies.iter().find(|m| m.tmdb_id == Some(tid)).cloned()
                } else {
                    None
                };

                if existing.is_none() {
                    existing = all_movies
                        .iter()
                        .find(|m| {
                            m.title.eq_ignore_ascii_case(&m_scan.clean_title)
                                || (m_scan.folder_year.is_some()
                                    && m.year == m_scan.folder_year
                                    && m.title.to_lowercase().contains(&m_scan.clean_title.to_lowercase()))
                        })
                        .cloned();
                }

                if let Some(mut item) = existing {
                    item.status = "completed".to_string();
                    item.file_path = Some(m_scan.file_path_str);
                    if item.year.is_none() {
                        item.year = m_scan.folder_year;
                    }
                    if item.tmdb_id.is_none() {
                        item.tmdb_id = m_scan.tmdb_id;
                    }
                    item.updated_at = now;
                    let _ = db.insert_media_item(&item);
                    if let Some(slot) = all_movies.iter_mut().find(|m| m.id == item.id) {
                        *slot = item;
                    }
                    updated += 1;
                } else {
                    let item = MediaItem {
                        id: Uuid::new_v4().to_string(),
                        tmdb_id: m_scan.tmdb_id,
                        imdb_id: None,
                        tvdb_id: None,
                        media_type: "movie".to_string(),
                        title: if m_scan.clean_title.is_empty() {
                            m_scan.folder_name
                        } else {
                            m_scan.clean_title
                        },
                        original_title: None,
                        year: m_scan.folder_year,
                        overview: None,
                        poster_path: None,
                        backdrop_path: None,
                        status: "completed".to_string(),
                        monitored: true,
                        quality_profile: "1080p-es".to_string(),
                        root_folder: movies_root_str.clone(),
                        file_path: Some(m_scan.file_path_str),
                        download_id: None,
                        created_at: now,
                        updated_at: now,
                    };
                    let _ = db.insert_media_item(&item);
                    all_movies.push(item);
                    updated += 1;
                }
            }

            let mut all_shows = db.list_media_items(Some("tv"), None).unwrap_or_default();

            for s_scan in scanned_series {
                let mut media_item = if let Some(tvid) = s_scan.tvdb_id {
                    all_shows.iter().find(|s| s.tvdb_id == Some(tvid)).cloned()
                } else if let Some(tid) = s_scan.tmdb_id {
                    all_shows.iter().find(|s| s.tmdb_id == Some(tid)).cloned()
                } else {
                    None
                };

                if media_item.is_none() {
                    media_item = all_shows
                        .iter()
                        .find(|s| {
                            s.title.eq_ignore_ascii_case(&s_scan.clean_title)
                                || s.title.to_lowercase().contains(&s_scan.clean_title.to_lowercase())
                                || s_scan.clean_title.to_lowercase().contains(&s.title.to_lowercase())
                        })
                        .cloned();
                }

                let media_id = if let Some(mut item) = media_item {
                    if item.year.is_none() {
                        item.year = s_scan.folder_year;
                    }
                    if item.tvdb_id.is_none() {
                        item.tvdb_id = s_scan.tvdb_id;
                    }
                    if item.tmdb_id.is_none() {
                        item.tmdb_id = s_scan.tmdb_id;
                    }
                    item.updated_at = now;
                    let _ = db.insert_media_item(&item);
                    let id = item.id.clone();
                    if let Some(slot) = all_shows.iter_mut().find(|s| s.id == id) {
                        *slot = item;
                    }
                    id
                } else {
                    let id = Uuid::new_v4().to_string();
                    let item = MediaItem {
                        id: id.clone(),
                        tmdb_id: s_scan.tmdb_id,
                        tvdb_id: s_scan.tvdb_id,
                        imdb_id: None,
                        media_type: "tv".to_string(),
                        title: if s_scan.clean_title.is_empty() {
                            s_scan.folder_name
                        } else {
                            s_scan.clean_title
                        },
                        original_title: None,
                        year: s_scan.folder_year,
                        overview: None,
                        poster_path: None,
                        backdrop_path: None,
                        status: "downloading".to_string(),
                        monitored: true,
                        quality_profile: "1080p-es".to_string(),
                        root_folder: series_root_str.clone(),
                        file_path: None,
                        download_id: None,
                        created_at: now,
                        updated_at: now,
                    };
                    let _ = db.insert_media_item(&item);
                    all_shows.push(item);
                    id
                };

                if !s_scan.episodes.is_empty() {
                    let mut existing_episodes = db.get_episodes_for_media(&media_id).unwrap_or_default();
                    let mut eps_to_batch: Vec<EpisodeItem> = Vec::new();
                    for (s_num, ep_num, ep_filename, file_path_str) in s_scan.episodes {
                        episodes_scanned += 1;
                        if let Some(existing) = existing_episodes
                            .iter_mut()
                            .find(|e| e.season_number == s_num && e.episode_number == ep_num)
                        {
                            existing.status = "completed".to_string();
                            existing.file_path = Some(file_path_str);
                            existing.updated_at = now;
                            eps_to_batch.push(existing.clone());
                            updated += 1;
                        } else {
                            let ep_item = EpisodeItem {
                                id: Uuid::new_v4().to_string(),
                                media_id: media_id.clone(),
                                season_number: s_num,
                                episode_number: ep_num,
                                title: ep_filename,
                                air_date: None,
                                status: "completed".to_string(),
                                monitored: true,
                                file_path: Some(file_path_str),
                                download_id: None,
                                created_at: now,
                                updated_at: now,
                            };
                            eps_to_batch.push(ep_item.clone());
                            existing_episodes.push(ep_item);
                            updated += 1;
                        }
                    }
                    if !eps_to_batch.is_empty() {
                        let _ = db.insert_episodes_batch(&eps_to_batch);
                    }

                    let all_completed = !existing_episodes.is_empty()
                        && existing_episodes.iter().all(|e| e.status == "completed");
                    if let Ok(Some(mut current_show)) = db.get_media_item(&media_id) {
                        let new_status = if all_completed { "completed" } else { "downloading" };
                        if current_show.status != new_status {
                            current_show.status = new_status.to_string();
                            current_show.updated_at = now;
                            let _ = db.insert_media_item(&current_show);
                        }
                    }
                }
            }
        }

        // 3. Ejecutar automáticamente un pase de enriquecimiento de metadatos en elementos sin póster, sinopsis o lista de episodios
        let enriched = Self::enrich_library(state).await;

        json!({
            "moviesScanned": movies_scanned,
            "episodesScanned": episodes_scanned,
            "itemsSynchronized": updated,
            "itemsEnriched": enriched
        })
    }

    /// Enriquece todos los elementos de la biblioteca a los que les falte poster_path, overview o la lista de episodios
    pub async fn enrich_library(state: &AppState) -> usize {
        let (items_to_enrich, show_episodes_map) = {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();
            let all_items = db.list_media_items(None, None).unwrap_or_default();
            let mut ep_map: HashMap<String, Vec<EpisodeItem>> = HashMap::new();
            let mut candidates: Vec<MediaItem> = Vec::new();

            for item in all_items {
                let needs_metadata = item.poster_path.as_deref().unwrap_or("").trim().is_empty()
                    || item.overview.as_deref().unwrap_or("").trim().is_empty()
                    || item.year.is_none();

                let mut needs_episodes = false;
                if item.media_type == "tv" {
                    let eps = db.get_episodes_for_media(&item.id).unwrap_or_default();
                    needs_episodes = eps.is_empty()
                        || eps.iter().any(|e| {
                            e.title.ends_with(".mkv")
                                || e.title.ends_with(".avi")
                                || e.title.ends_with(".mp4")
                                || e.title.is_empty()
                        });
                    if needs_episodes {
                        ep_map.insert(item.id.clone(), eps);
                    }
                }

                if needs_metadata || needs_episodes {
                    candidates.push(item);
                }
            }
            (candidates, ep_map)
        };

        let mut enriched_count = 0usize;
        for mut item in items_to_enrich {
            let needs_metadata = item.poster_path.as_deref().unwrap_or("").trim().is_empty()
                || item.overview.as_deref().unwrap_or("").trim().is_empty()
                || item.year.is_none();

            let needs_episodes = if item.media_type == "tv" {
                let existing_eps = show_episodes_map
                    .get(&item.id)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                existing_eps.is_empty()
                    || existing_eps.iter().any(|e| {
                        e.title.ends_with(".mkv")
                            || e.title.ends_with(".avi")
                            || e.title.ends_with(".mp4")
                            || e.title.is_empty()
                    })
            } else {
                false
            };

            if !needs_metadata && !needs_episodes {
                continue;
            }

            let mut fetched_meta = None;

            if item.media_type == "movie" {
                if let Some(tid) = item.tmdb_id {
                    if let Ok(meta) = state.media_metadata.get_movie(tid).await {
                        fetched_meta = Some(meta);
                    }
                }
                if fetched_meta.is_none() {
                    if let Ok(results) = state.media_metadata.search_movies(&item.title).await {
                        let matched = if let Some(y) = item.year {
                            results.into_iter().find(|r| {
                                r.year.map(|ry| (ry as i64 - y as i64).abs() <= 1).unwrap_or(false)
                            })
                        } else {
                            results.into_iter().next()
                        };
                        fetched_meta = matched;
                    }
                }
            } else {
                if let Some(tvid) = item.tvdb_id {
                    if let Ok(meta) = state.media_metadata.get_show(tvid).await {
                        fetched_meta = Some(meta);
                    }
                }
                if fetched_meta.is_none() {
                    if let Ok(results) = state.media_metadata.search_shows(&item.title).await {
                        let matched = if let Some(tid) = item.tmdb_id {
                            results.iter().find(|r| r.tmdb_id == Some(tid)).cloned()
                        } else if let Some(y) = item.year {
                            results.into_iter().find(|r| {
                                r.year.map(|ry| (ry as i64 - y as i64).abs() <= 1).unwrap_or(false)
                            })
                        } else {
                            results.into_iter().next()
                        };
                        fetched_meta = matched;
                    }
                }
            }

            if let Some(meta) = fetched_meta {
                if item.tmdb_id.is_none() {
                    item.tmdb_id = meta.tmdb_id;
                }
                if item.tvdb_id.is_none() {
                    item.tvdb_id = meta.tvdb_id;
                }
                if item.imdb_id.is_none() {
                    item.imdb_id = meta.imdb_id;
                }
                if item.year.is_none() {
                    item.year = meta.year;
                }
                if item.overview.as_deref().unwrap_or("").trim().is_empty() {
                    item.overview = meta.overview;
                }
                if item.poster_path.as_deref().unwrap_or("").trim().is_empty() {
                    item.poster_path = meta.poster_url;
                }
                if item.backdrop_path.as_deref().unwrap_or("").trim().is_empty() {
                    item.backdrop_path = meta.backdrop_url;
                }
                if item.original_title.is_none() {
                    item.original_title = meta.original_title;
                }
                item.updated_at = now_ms();

                let episodes_to_insert = if item.media_type == "tv" {
                    if let Some(tvid) = item.tvdb_id {
                        state.media_metadata.get_show_episodes(tvid).await.ok()
                    } else {
                        None
                    }
                } else {
                    None
                };

                let mut dl_svc = state.downloads.lock().await;
                let db = dl_svc.db_mut();
                let _ = db.insert_media_item(&item);

                if let Some(eps) = episodes_to_insert {
                    let mut existing_eps = db.get_episodes_for_media(&item.id).unwrap_or_default();
                    let mut eps_to_batch: Vec<EpisodeItem> = Vec::new();
                    for (s_num, ep_num, ep_title, air_date) in eps {
                        if let Some(existing) = existing_eps
                            .iter_mut()
                            .find(|e| e.season_number == s_num && e.episode_number == ep_num)
                        {
                            let is_generic_title = existing.title.ends_with(".mkv")
                                || existing.title.ends_with(".avi")
                                || existing.title.ends_with(".mp4")
                                || existing.title.is_empty();
                            let mut changed = false;
                            if is_generic_title && !ep_title.is_empty() {
                                existing.title = ep_title;
                                changed = true;
                            }
                            if existing.air_date.is_none() && air_date.is_some() {
                                existing.air_date = air_date;
                                changed = true;
                            }
                            if changed {
                                existing.updated_at = now_ms();
                                eps_to_batch.push(existing.clone());
                            }
                        } else {
                            let ep_item = EpisodeItem {
                                id: Uuid::new_v4().to_string(),
                                media_id: item.id.clone(),
                                season_number: s_num,
                                episode_number: ep_num,
                                title: ep_title,
                                air_date,
                                status: "wanted".to_string(),
                                monitored: true,
                                file_path: None,
                                download_id: None,
                                created_at: item.updated_at,
                                updated_at: item.updated_at,
                            };
                            eps_to_batch.push(ep_item.clone());
                            existing_eps.push(ep_item);
                        }
                    }
                    if !eps_to_batch.is_empty() {
                        let _ = db.insert_episodes_batch(&eps_to_batch);
                    }

                    let has_wanted = existing_eps.iter().any(|e| e.status == "wanted");
                    let new_status = if has_wanted { "downloading" } else { "completed" };
                    if item.status != new_status {
                        item.status = new_status.to_string();
                        let _ = db.insert_media_item(&item);
                    }
                }
                enriched_count += 1;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(METADATA_ENRICH_THROTTLE_MS)).await;
        }

        enriched_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_season_episode_standard() {
        assert_eq!(
            extract_season_episode(
                None,
                "Linternas (2026) - S01E07 - El legado de los Jordan [2160p][H.265].mkv"
            ),
            Some((1, 7))
        );
        assert_eq!(
            extract_season_episode(
                None,
                "VisionQuest (2026) - S01E01 - Sin ataduras [1080p][H.265].mkv"
            ),
            Some((1, 1))
        );
        assert_eq!(
            extract_season_episode(
                None,
                "Cyberpunk- Edgerunners (2022) - S01E05 - Todas las miradas [1080p][H.264].mkv"
            ),
            Some((1, 5))
        );
        assert_eq!(
            extract_season_episode(
                None,
                "La casa del dragón (2022) - S01E08 - El señor de las mareas [1080p][H.265].mkv"
            ),
            Some((1, 8))
        );
        assert_eq!(
            extract_season_episode(None, "Silo (2023) - S03E10 - Troy.avi"),
            Some((3, 10))
        );
    }

    #[test]
    fn test_extract_season_episode_x_notation() {
        assert_eq!(
            extract_season_episode(
                None,
                "La.casa.del.dragón.1x05.Iluminamos.el.camino.(Spanish.English.Subs).BDRip.1080p.x265-EAC3.by.Legan.mkv"
            ),
            Some((1, 5))
        );
        assert_eq!(
            extract_season_episode(
                None,
                "MobLand 2025 2x05 1080p - Birland (V.O.S. Spa-Eng) - Spanish subs integrados.mkv"
            ),
            Some((2, 5))
        );
    }

    #[test]
    fn test_extract_season_from_folder_and_fallback() {
        assert_eq!(extract_season_from_folder("Season 01"), Some(1));
        assert_eq!(extract_season_from_folder("Season 2"), Some(2));
        assert_eq!(extract_season_from_folder("Temporada 3"), Some(3));
        assert_eq!(
            extract_season_episode(Some(2), "Episodio 04 - El retorno.mkv"),
            Some((2, 4))
        );
    }

    #[test]
    fn test_movie_names_not_matched_as_episodes() {
        assert_eq!(
            extract_season_episode(None, "Oddity (2024) [1080p][H.264].mkv"),
            None
        );
        assert_eq!(
            extract_season_episode(None, "Llaman a la puerta (2023).avi"),
            None
        );
    }

    #[test]
    fn test_extract_metadata_tags() {
        assert_eq!(
            extract_tmdb_id("Llaman a la puerta (2023) [tmdbid-631842]"),
            Some(631842)
        );
        assert_eq!(
            extract_tvdb_id("House of the Dragon () [tvdbid-371572]"),
            Some(371572)
        );
        assert_eq!(extract_year("Oddity (2024)"), Some(2024));
        assert_eq!(extract_year("House of the Dragon ()"), None);
    }
}
