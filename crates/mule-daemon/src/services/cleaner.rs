// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use crate::services::downloads::now_ms;
use crate::services::media_probe::{probe_video_file, MediaProbeInfo};
use crate::services::organizer::{clean_movie_title, remove_companion_subtitles_sync};
use crate::state::AppState;

/// Extensiones de archivos residuales o no deseados a eliminar en las carpetas de descargas entrantes.
/// NOTA: "nfo", las imágenes y los subtítulos NUNCA están en esta lista porque son datos legítimos de Jellyfin/Kodi.
pub const JUNK_EXTENSIONS: &[&str] = &[
    "url", "html", "htm", "exe", "bat", "cmd", "scr", "lnk",
    "sfv", "md5", "srr", "srs", "nzb", "torrent", "ds_store", "db",
];

/// Nombres de imágenes de carátula estándar de Jellyfin/Kodi a preservar en las carpetas raíz de títulos.
const PRESERVED_ARTWORK_STEMS: &[&str] = &[
    "poster", "fanart", "backdrop", "banner", "clearlogo", "landscape", "logo",
    "season-specials-poster",
];

/// Comprueba si una imagen (.jpg, .png...) es un archivo basura/residual
fn is_junk_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif") {
        return false;
    }

    let fname = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();

    // Toda imagen ubicada en una subcarpeta de temporada ("Season 01", etc.) es un residuo P2P
    if let Some(parent_name) = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        let parent_lower = parent_name.to_lowercase();
        if parent_lower.starts_with("season") || parent_lower.starts_with("temporada") {
            return true;
        }
    }

    // Imágenes nombradas explícitamente thumbnail / proof / screen / rarbg
    if fname.contains("thumb") || fname.contains("sample") || fname.contains("proof") || fname.contains("screen") || fname.contains("rarbg") {
        return true;
    }

    // Los pósteres de temporada en la raíz (ej. season01-poster.jpg) se preservan
    if stem.starts_with("season") && stem.ends_with("-poster") {
        return false;
    }

    // Si el nombre coincide con una carátula oficial de Jellyfin/Kodi, preservar
    if PRESERVED_ARTWORK_STEMS.contains(&stem.as_str()) {
        return false;
    }

    // De lo contrario, imagen huérfana/desconocida
    true
}

/// Extensiones de vídeo reconocidas por la política de contenido.
pub const VIDEO_EXTENSIONS: &[&str] = mule_network_policy::content_policy::ALLOWED_VIDEO_EXTENSIONS;

/// Extensiones de subtítulos complementarios reconocidas por la política de contenido.
pub const SUBTITLE_EXTENSIONS: &[&str] =
    mule_network_policy::content_policy::COMPANION_SUBTITLE_EXTENSIONS;

/// Tamaño máximo de un extracto de vista previa «sample / trailer» (85 MiB).
pub const SAMPLE_MAX_BYTES: u64 = 85 * 1024 * 1024;

/// Antigüedad mínima (en segundos) de un archivo part temporal huérfano antes de su eliminación (1 hora).
pub const ORPHAN_TEMP_MIN_AGE_SECS: u64 = 3600;

/// Par de archivos duplicados detectado (ganador conservado frente a copia eliminada).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateMatch {
    pub kind: String,
    pub group_label: String,
    pub kept_path: String,
    pub kept_score: i32,
    pub kept_size: u64,
    pub removed_path: String,
    pub removed_score: i32,
    pub removed_size: u64,
}

#[derive(Default, Debug)]
pub struct CleanReport {
    pub dry_run: bool,
    pub duplicate_episodes_removed: Vec<String>,
    pub duplicate_movies_removed: Vec<String>,
    pub duplicate_pairs: Vec<DuplicateMatch>,
    pub junk_files_removed: Vec<String>,
    pub orphan_subtitles_removed: Vec<String>,
    pub incoming_cleaned: Vec<String>,
    pub empty_dirs_removed: Vec<String>,
    pub redundant_downloads_cancelled: Vec<String>,
    pub bytes_freed: u64,
}

impl CleanReport {
    pub fn to_json(&self) -> Value {
        json!({
            "dryRun": self.dry_run,
            "duplicateEpisodesRemoved": self.duplicate_episodes_removed,
            "duplicateMoviesRemoved": self.duplicate_movies_removed,
            "duplicatePairs": self.duplicate_pairs,
            "junkFilesRemoved": self.junk_files_removed,
            "orphanSubtitlesRemoved": self.orphan_subtitles_removed,
            "incomingCleaned": self.incoming_cleaned,
            "emptyDirsRemoved": self.empty_dirs_removed,
            "redundantDownloadsCancelled": self.redundant_downloads_cancelled,
            "bytesFreed": self.bytes_freed,
            "summary": format!(
                "Limpieza {}: {} episodios duplicados, {} películas duplicadas, {} archivos residuales, {} subtítulos huérfanos, {} carpetas vacías ({} MB liberados)",
                if self.dry_run { "[Simulación]" } else { "[Ejecutada]" },
                self.duplicate_episodes_removed.len(),
                self.duplicate_movies_removed.len(),
                self.junk_files_removed.len() + self.incoming_cleaned.len(),
                self.orphan_subtitles_removed.len(),
                self.empty_dirs_removed.len(),
                self.bytes_freed / (1024 * 1024)
            )
        })
    }
}

pub struct MediaCleaner;

impl MediaCleaner {
    /// Puntúa un archivo de vídeo existente a partir de su sonda `ffprobe`.
    fn score_candidate_from_probe(path: &Path, file_size: u64, probe: MediaProbeInfo) -> i32 {
        let mut score: i32 = 0;

        // 1. Puntuación de idioma
        if probe.has_spanish_audio {
            score += 100;
        } else if probe.has_spanish_subtitles {
            score += 50;
        } else if probe.detected_language_profile == "English" {
            score += 10;
        }

        // 2. Puntuación de resolución
        match probe.quality_tag.as_str() {
            "2160p" => score += 80,
            "1080p" => score += 60,
            "720p" => score += 40,
            _ => score += 10,
        }

        // 3. Códecs modernos
        if let Some(ref codec) = probe.video_codec {
            let lower = codec.to_lowercase();
            if lower.contains("hevc") || lower.contains("h265") || lower.contains("av1") {
                score += 15;
            }
        }

        // 4. Bonificación de metadatos del nombre de archivo (título propio de episodio frente a genérico "Episode 1")
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let lower = stem.to_lowercase();
            if !lower.contains("sample") && !lower.contains("trailer") {
                score += 20;
            }
            if stem.contains(" - ") {
                score += 10;
            }
        }

        // 5. Bonificación por nivel de tamaño (ligera preferencia por mayor tasa de bits / flujo sin comprimir)
        let size_mb = (file_size / (1024 * 1024)) as i32;
        score += size_mb.min(50);

        score
    }

    /// Puntúa un archivo de vídeo existente de forma síncrona (tests).
    #[cfg(test)]
    fn score_candidate(path: &Path, file_size: u64) -> i32 {
        Self::score_candidate_from_probe(
            path,
            file_size,
            crate::services::media_probe::probe_video_file_sync(path),
        )
    }

    /// Puntúa un archivo de vídeo existente de forma asíncrona sin bloquear el ejecutor Tokio.
    async fn score_candidate_async(path: &Path, file_size: u64) -> i32 {
        let probe = probe_video_file(path).await;
        Self::score_candidate_from_probe(path, file_size, probe)
    }

    /// Comprueba si un archivo es un vídeo de muestra / tráiler
    pub fn is_sample_or_trailer(path: &Path, file_size: u64) -> bool {
        if file_size > SAMPLE_MAX_BYTES {
            return false;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            return false;
        };
        let lower = stem.to_lowercase();
        lower.contains("sample") || lower.contains("trailer") || lower.contains("rarbg.com")
    }

    /// Elimina recursivamente directorios vacíos bajo la raíz, omitiendo directorios protegidos
    fn purge_empty_dirs_recursive(
        dir: &Path,
        protected_dirs: &[PathBuf],
        dry_run: bool,
        report: &mut CleanReport,
    ) {
        if protected_dirs.iter().any(|p| p == dir) {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut subdirs = Vec::new();
        let mut has_entries = false;

        for entry in entries.flatten() {
            has_entries = true;
            let p = entry.path();
            if p.is_dir() {
                subdirs.push(p);
            }
        }

        for sub in subdirs {
            Self::purge_empty_dirs_recursive(&sub, protected_dirs, dry_run, report);
        }

        if protected_dirs.iter().any(|p| p == dir) {
            return;
        }

        // Volver a comprobar tras la posible eliminación de subdirectorios
        if let Ok(mut remaining) = std::fs::read_dir(dir) {
            if remaining.next().is_none() {
                report.empty_dirs_removed.push(dir.to_string_lossy().to_string());
                if !dry_run {
                    let _ = std::fs::remove_dir(dir);
                    info!("[CLEANER] Removed empty directory: '{}'", dir.display());
                }
            }
        } else if !has_entries {
            report.empty_dirs_removed.push(dir.to_string_lossy().to_string());
            if !dry_run {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }

    /// Escanea el directorio y elimina archivos basura y subtítulos complementarios huérfanos.
    /// En las carpetas de biblioteca (movies_dir / series_dir), las imágenes de portada (.jpg, .png...), metadatos (.nfo, .xml)
    /// y subtítulos (.srt, .vtt) NUNCA se eliminan porque pertenecen a Jellyfin/Kodi.
    fn clean_junk_and_orphan_subtitles(
        dir: &Path,
        is_library_folder: bool,
        dry_run: bool,
        report: &mut CleanReport,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        let mut video_stems = Vec::new();
        let mut subtitle_candidates = Vec::new();

        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                Self::clean_junk_and_orphan_subtitles(&p, is_library_folder, dry_run, report);
                continue;
            }
            if !p.is_file() {
                continue;
            }

            let fname = entry.file_name().to_string_lossy().to_string();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);

            // En las carpetas de biblioteca (movies_dir y series_dir), nunca eliminar archivos de portada o metadatos:
            // .nfo, .xml, .json, .jpg, .jpeg, .png, .webp, .tbn, .gif, .bif, .srt, .sub, .vtt, .ass, .ssa, .idx
            // y archivos de puntos (.ignore) son recursos oficiales de Jellyfin/Kodi.
            let is_artwork_or_meta = matches!(
                ext.as_str(),
                "nfo" | "xml" | "json" | "jpg" | "jpeg" | "png" | "webp" | "tbn" | "gif" | "bif" | "srt" | "sub" | "vtt" | "ass" | "ssa" | "idx"
            ) || fname.starts_with('.');

            if is_library_folder && is_artwork_or_meta {
                continue;
            }

            // 1. Extensiones basura y archivos de imagen basura
            let is_junk = if is_library_folder {
                // En carpetas de biblioteca: solo eliminar binarios peligrosos, accesos directos, enlaces web o marcadores temporales residuales
                matches!(
                    ext.as_str(),
                    "url" | "html" | "htm" | "exe" | "bat" | "cmd" | "scr" | "lnk" | "ds_store"
                ) || fname.ends_with(".part.met.keep")
            } else {
                // En carpetas de descargas entrantes: basura de la escena + imágenes basura de prueba/muestra
                JUNK_EXTENSIONS.contains(&ext.as_str())
                    || fname.ends_with(".part.met.keep")
                    || is_junk_image(&p)
            };

            if is_junk {
                report.junk_files_removed.push(p.to_string_lossy().to_string());
                report.bytes_freed += size;
                if !dry_run {
                    let _ = std::fs::remove_file(&p);
                    info!("[CLEANER] Removed junk file: '{}' ({} bytes)", p.display(), size);
                }
                continue;
            }

            // 2. Vídeos de muestra / tráiler
            if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
                if Self::is_sample_or_trailer(&p, size) {
                    report.junk_files_removed.push(p.to_string_lossy().to_string());
                    report.bytes_freed += size;
                    if !dry_run {
                        let _ = std::fs::remove_file(&p);
                        info!("[CLEANER] Removed sample video: '{}' ({} bytes)", p.display(), size);
                    }
                    continue;
                }
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    video_stems.push(stem.to_string());
                }
            }

            // 3. Subtítulos (¡solo limpiar subtítulos huérfanos en carpetas entrantes, NUNCA en la biblioteca organizada!)
            if !is_library_folder && SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
                subtitle_candidates.push((p, size));
            }
        }

        // Verificar subtítulos huérfanos en carpetas de descargas entrantes
        for (sub_path, size) in subtitle_candidates {
            let Some(stem) = sub_path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let clean_sub_stem = stem
                .trim_end_matches(".es")
                .trim_end_matches(".spa")
                .trim_end_matches(".en");

            let matches_video = video_stems.iter().any(|vstem| {
                vstem == stem || vstem == clean_sub_stem || vstem.starts_with(clean_sub_stem)
            });

            if !matches_video {
                report.orphan_subtitles_removed.push(sub_path.to_string_lossy().to_string());
                report.bytes_freed += size;
                if !dry_run {
                    let _ = std::fs::remove_file(&sub_path);
                    info!("[CLEANER] Removed orphan subtitle: '{}'", sub_path.display());
                }
            }
        }
    }

    /// Deduplica episodios de series de TV en series_dir
    async fn deduplicate_series(state: &AppState, dry_run: bool, report: &mut CleanReport) {
        let series_dir = state.effective_series_dir().await;
        if !series_dir.is_dir() {
            return;
        }

        let Ok(series_entries) = std::fs::read_dir(&series_dir) else {
            return;
        };

        let shows = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().list_media_items(Some("tv"), None).unwrap_or_default()
        };

        for s_entry in series_entries.flatten() {
            let s_path = s_entry.path();
            if !s_path.is_dir() {
                continue;
            }

            let s_folder_name = s_entry.file_name().to_string_lossy().to_string();

            // Hallar la serie de medios coincidente en SQLite
            let series_item_opt = shows.iter().find(|s| {
                if let Some(tvdb_id) = s.tvdb_id {
                    if s_folder_name.contains(&format!("tvdbid-{tvdb_id}")) {
                        return true;
                    }
                }
                clean_movie_title(&s.title).to_lowercase() == clean_movie_title(&s_folder_name).to_lowercase()
            });

            // Recopilar todos los archivos de vídeo a través de las carpetas de temporada y la raíz de la serie
            let mut all_video_files = Vec::new();
            let mut to_visit = vec![s_path.clone()];

            while let Some(current) = to_visit.pop() {
                if let Ok(entries) = std::fs::read_dir(&current) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            to_visit.push(p);
                        } else if p.is_file() {
                            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                            if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
                                let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                                if !Self::is_sample_or_trailer(&p, size) {
                                    all_video_files.push((p, size));
                                }
                            }
                        }
                    }
                }
            }

            // Agrupar archivos de vídeo por (season_num, ep_num)
            let mut episodes_map: HashMap<(u32, u32), Vec<(PathBuf, u64)>> = HashMap::new();

            for (vpath, size) in all_video_files {
                let fname = vpath.file_name().unwrap_or_default().to_string_lossy().to_string();
                // Determinar la pista de temporada a partir de la carpeta contenedora (ej. "Season 01")
                let season_hint = vpath
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .and_then(|name| {
                        let lower = name.to_lowercase();
                        if lower.starts_with("season") || lower.starts_with("temporada") {
                            name.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().ok()
                        } else {
                            None
                        }
                    });

                if let Some((s_num, ep_num)) = crate::services::disk_scanner::extract_season_episode(season_hint, &fname) {
                    episodes_map.entry((s_num, ep_num)).or_default().push((vpath, size));
                }
            }

            // Para cada (temporada, episodio), si hay más de 1 archivo, encontrar el ganador y eliminar duplicados
            for ((s_num, ep_num), files) in episodes_map {
                if files.len() <= 1 {
                    continue;
                }

                info!(
                    "[CLEANER] Found {} duplicate files for series '{}' S{:02}E{:02}",
                    files.len(),
                    s_folder_name,
                    s_num,
                    ep_num
                );

                // Puntuar candidatos de forma asíncrona
                let mut scored: Vec<(i32, PathBuf, u64)> = Vec::with_capacity(files.len());
                for (p, sz) in files {
                    let sc = Self::score_candidate_async(&p, sz).await;
                    scored.push((sc, p, sz));
                }

                // Mayor puntuación primero; en caso de empate, mayor tamaño de archivo primero
                scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.cmp(&a.2)));

                let (winner_score, winner, winner_size) = (&scored[0].0, &scored[0].1, &scored[0].2);
                let winner_str = winner.to_string_lossy().to_string();

                for (loser_score, loser_path, loser_size) in &scored[1..] {
                    let loser_str = loser_path.to_string_lossy().to_string();
                    report.duplicate_episodes_removed.push(loser_str.clone());
                    report.duplicate_pairs.push(DuplicateMatch {
                        kind: "episode".to_string(),
                        group_label: format!("{s_folder_name} S{s_num:02}E{ep_num:02}"),
                        kept_path: winner_str.clone(),
                        kept_score: *winner_score,
                        kept_size: *winner_size,
                        removed_path: loser_str.clone(),
                        removed_score: *loser_score,
                        removed_size: *loser_size,
                    });
                    report.bytes_freed += loser_size;
                    if !dry_run {
                        remove_companion_subtitles_sync(loser_path);
                        let _ = tokio::fs::remove_file(loser_path).await;
                        Self::purge_shared_file_by_path(state, &loser_str).await;
                        info!(
                            "[CLEANER] Deleted duplicate episode file: '{}' (kept '{}')",
                            loser_path.display(),
                            winner.display()
                        );
                    }
                }

                // Actualizar ruta ganadora en la base de datos SQLite
                if !dry_run {
                    if let Some(series) = series_item_opt {
                        let dl_svc = state.downloads.lock().await;
                        let db = dl_svc.db();
                        if let Ok(eps) = db.get_episodes_for_media(&series.id) {
                            if let Some(ep) = eps.into_iter().find(|e| e.season_number == s_num && e.episode_number == ep_num) {
                                let _ = db.update_episode_status(
                                    &ep.id,
                                    "completed",
                                    Some(&winner_str),
                                    None,
                                    now_ms(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Elimina de `shared_files` cualquier entrada huérfana cuya ruta coincida con el archivo borrado.
    async fn purge_shared_file_by_path(state: &AppState, removed_path: &str) {
        let hashes: Vec<String> = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .list_all_shared_files()
                .unwrap_or_default()
                .into_iter()
                .filter(|sf| sf.file_path == removed_path)
                .map(|sf| sf.file_hash)
                .collect()
        };
        if hashes.is_empty() {
            return;
        }
        let mut dl_svc = state.downloads.lock().await;
        for hash in hashes {
            let _ = dl_svc.remove_shared_file(&hash, false);
        }
    }

    /// Calcula la identidad canónica de una carpeta de película para agrupar duplicados
    /// incluso cuando existen carpetas distintas (con o sin `[tmdbid-...]`).
    fn canonical_movie_key(folder_name: &str, movies: &[mule_core::model::MediaItem]) -> (String, String) {
        if let Some(pos) = folder_name.find("tmdbid-") {
            let digits: String = folder_name[pos + 7..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                let label = movies
                    .iter()
                    .find(|m| m.tmdb_id.map(|id| id.to_string()) == Some(digits.clone()))
                    .map(|m| m.title.clone())
                    .unwrap_or_else(|| clean_movie_title(folder_name));
                return (format!("tmdb:{digits}"), label);
            }
        }

        let clean_folder = clean_movie_title(folder_name).to_lowercase();
        if let Some(m) = movies
            .iter()
            .find(|m| clean_movie_title(&m.title).to_lowercase() == clean_folder)
        {
            if let Some(tmdb_id) = m.tmdb_id {
                return (format!("tmdb:{tmdb_id}"), m.title.clone());
            }
            return (format!("title:{clean_folder}"), m.title.clone());
        }

        (format!("title:{clean_folder}"), clean_movie_title(folder_name))
    }

    /// Deduplica películas en `movies_dir`, detectando copias repetidas tanto dentro de la misma
    /// carpeta como entre carpetas distintas que correspondan a la misma película.
    async fn deduplicate_movies(state: &AppState, dry_run: bool, report: &mut CleanReport) {
        let movies_dir = state.effective_movies_dir().await;
        if !movies_dir.is_dir() {
            return;
        }

        let Ok(movie_entries) = std::fs::read_dir(&movies_dir) else {
            return;
        };

        let movies = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().list_media_items(Some("movie"), None).unwrap_or_default()
        };

        struct MovieGroup {
            label: String,
            folder_names: Vec<String>,
            videos: Vec<(PathBuf, u64)>,
        }

        let mut grouped: HashMap<String, MovieGroup> = HashMap::new();

        for m_entry in movie_entries.flatten() {
            let m_path = m_entry.path();
            if !m_path.is_dir() {
                continue;
            }

            let m_folder_name = m_entry.file_name().to_string_lossy().to_string();
            let (key, label) = Self::canonical_movie_key(&m_folder_name, &movies);

            let entry_group = grouped.entry(key).or_insert_with(|| MovieGroup {
                label,
                folder_names: Vec::new(),
                videos: Vec::new(),
            });
            entry_group.folder_names.push(m_folder_name);

            if let Ok(entries) = std::fs::read_dir(&m_path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                        if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
                            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                            if !Self::is_sample_or_trailer(&p, size) {
                                entry_group.videos.push((p, size));
                            }
                        }
                    }
                }
            }
        }

        for (key, group) in grouped {
            let MovieGroup {
                label: group_label,
                folder_names,
                videos: video_files,
            } = group;
            if video_files.len() <= 1 {
                continue;
            }

            info!(
                "[CLEANER] Found {} duplicate video files for movie '{}' ({})",
                video_files.len(),
                group_label,
                key
            );

            let mut scored: Vec<(i32, PathBuf, u64)> = Vec::with_capacity(video_files.len());
            for (p, sz) in video_files {
                let sc = Self::score_candidate_async(&p, sz).await;
                scored.push((sc, p, sz));
            }

            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.cmp(&a.2)));

            let (winner_score, winner, winner_size) = (scored[0].0, &scored[0].1, scored[0].2);
            let winner_str = winner.to_string_lossy().to_string();

            for (loser_score, loser_path, loser_size) in &scored[1..] {
                let loser_str = loser_path.to_string_lossy().to_string();
                report.duplicate_movies_removed.push(loser_str.clone());
                report.duplicate_pairs.push(DuplicateMatch {
                    kind: "movie".to_string(),
                    group_label: group_label.clone(),
                    kept_path: winner_str.clone(),
                    kept_score: winner_score,
                    kept_size: winner_size,
                    removed_path: loser_str.clone(),
                    removed_score: *loser_score,
                    removed_size: *loser_size,
                });
                report.bytes_freed += loser_size;
                if !dry_run {
                    remove_companion_subtitles_sync(loser_path);
                    let _ = tokio::fs::remove_file(loser_path).await;
                    Self::purge_shared_file_by_path(state, &loser_str).await;
                    info!(
                        "[CLEANER] Deleted duplicate movie file: '{}' (kept '{}')",
                        loser_path.display(),
                        winner.display()
                    );
                }
            }

            if !dry_run {
                let tmdb_opt = key.strip_prefix("tmdb:").and_then(|s| s.parse::<u64>().ok());
                if let Some(m) = movies.iter().find(|m| {
                    if let (Some(tid), Some(mtid)) = (tmdb_opt, m.tmdb_id) {
                        if tid == mtid {
                            return true;
                        }
                    }
                    let m_clean = clean_movie_title(&m.title).to_lowercase();
                    folder_names
                        .iter()
                        .any(|fnm| clean_movie_title(fnm).to_lowercase() == m_clean)
                }) {
                    let dl_svc = state.downloads.lock().await;
                    let _ = dl_svc.db().update_media_completed(&m.id, &winner_str, now_ms());
                }
            }
        }
    }

    /// Limpia directorios entrantes (default_download_dir + extra_incoming_dirs + temp_dir)
    async fn clean_incoming(state: &AppState, dry_run: bool, report: &mut CleanReport) {
        let effective_dl_dir = state.effective_download_dir().await;
        let mut incoming_dirs = vec![effective_dl_dir, state.config.default_download_dir.clone()];
        for d in &state.config.extra_incoming_dirs {
            if !incoming_dirs.contains(d) {
                incoming_dirs.push(d.clone());
            }
        }

        // Identificadores de descargas activas y archivos temporales
        let active_temp_paths = {
            let dl_svc = state.downloads.lock().await;
            let downloads = dl_svc.list_downloads();
            let mut temps = Vec::with_capacity(downloads.len() * 4);
            for d in downloads {
                // Ruta temporal basada en UUID
                temps.push(format!("{}.part", d.id));
                temps.push(format!("{}.part.met", d.id));
                temps.push(format!("{}.part.met.bak", d.id));
                temps.push(format!("{}.part.met.keep", d.id));
            }
            temps
        };

        let effective_movies = state.effective_movies_dir().await;
        let effective_series = state.effective_series_dir().await;

        // 1. Limpiar basura y muestras de las carpetas entrantes (no carpetas de biblioteca)
        for inc_dir in &incoming_dirs {
            if !inc_dir.is_dir() {
                continue;
            }
            if inc_dir == &state.config.movies_dir
                || inc_dir == &state.config.series_dir
                || inc_dir == &effective_movies
                || inc_dir == &effective_series
                || crate::services::organizer::is_already_in_organized_library(state, inc_dir).await
            {
                continue;
            }
            Self::clean_junk_and_orphan_subtitles(inc_dir, false, dry_run, report);
        }

        // 2. Limpiar archivos temporales .part huérfanos en temp_dir más antiguos que ORPHAN_TEMP_MIN_AGE_SECS
        let temp_dir = state.config.data_dir.join("temp");
        if temp_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(temp_dir) {
                let now_system = std::time::SystemTime::now();
                for entry in entries.flatten() {
                    let p = entry.path();
                    if !p.is_file() {
                        continue;
                    }
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if fname.ends_with(".part") || fname.ends_with(".part.met") || fname.ends_with(".part.met.keep") {
                        if active_temp_paths.contains(&fname) {
                            continue;
                        }
                        let metadata = p.metadata().ok();
                        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                        let age_secs = metadata
                            .and_then(|m| m.modified().ok())
                            .and_then(|mtime| now_system.duration_since(mtime).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);

                        if age_secs >= ORPHAN_TEMP_MIN_AGE_SECS {
                            report.incoming_cleaned.push(p.to_string_lossy().to_string());
                            report.bytes_freed += size;
                            if !dry_run {
                                let _ = tokio::fs::remove_file(&p).await;
                                info!("[CLEANER] Removed orphaned temporary file: '{}' ({} bytes)", p.display(), size);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Cancela descargas en vuelo redundantes para episodios/películas que ya están completados en disco
    async fn cancel_redundant_downloads(state: &AppState, dry_run: bool, report: &mut CleanReport) {
        let mut ag_state = crate::services::AutoGrabber::load_state(state).await;

        let active_grabs = ag_state.active_grabs.clone();
        let mut to_cancel = Vec::new();

        {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();

            for (dl_id_str, grab_item) in active_grabs {
                if grab_item.is_upgrade {
                    // Mejora legítima de calidad iniciada intencionadamente
                    continue;
                }

                // Comprobar si el episodio o película correspondiente ya está completado en disco
                let already_completed = if let Ok(Some(ep)) = db.find_episode_by_download_id(&dl_id_str) {
                    ep.status == "completed" && ep.file_path.as_ref().map(|p| Path::new(p).exists()).unwrap_or(false)
                } else if let Ok(Some(m)) = db.find_media_by_download_id(&dl_id_str) {
                    m.status == "completed" && m.file_path.as_ref().map(|p| Path::new(p).exists()).unwrap_or(false)
                } else {
                    false
                };

                if already_completed {
                    to_cancel.push((dl_id_str, grab_item.release_title));
                }
            }
        }

        for (dl_id_str, release_title) in to_cancel {
            report.redundant_downloads_cancelled.push(format!("{} ('{}')", dl_id_str, release_title));
            if !dry_run {
                if let Ok(dl_uuid) = Uuid::parse_str(&dl_id_str) {
                    let mut dl_svc_mut = state.downloads.lock().await;
                    // CRÍTICO: ¡Pasar siempre delete_files = false para NO eliminar NUNCA
                    // el archivo multimedia final que el sistema acaba de descargar y organizar!
                    let _ = dl_svc_mut.remove_download(&dl_uuid, false);
                }
                ag_state.active_grabs.remove(&dl_id_str);
                info!(
                    "[CLEANER] Removed completed download task '{}' from active queue",
                    release_title
                );
            }
        }

        if !dry_run {
            crate::services::AutoGrabber::save_state(state, &ag_state).await;
        }
    }

    /// Ejecuta la limpieza completa de duplicados y residuos en todo el ecosistema de medios
    pub async fn clean_all(state: &AppState, dry_run: bool) -> Value {
        let mut report = CleanReport {
            dry_run,
            ..Default::default()
        };

        info!(
            "[CLEANER] Starting media cleanup (dry_run: {})...",
            dry_run
        );

        // 1. Deduplicar episodios de series de TV
        Self::deduplicate_series(state, dry_run, &mut report).await;

        // 2. Deduplicar películas
        Self::deduplicate_movies(state, dry_run, &mut report).await;

        let effective_series = state.effective_series_dir().await;
        let effective_movies = state.effective_movies_dir().await;

        // 3. Purgar basura, tráileres/muestras de carpetas de medios (preservando estrictamente portadas, metadatos nfo y subtítulos)
        Self::clean_junk_and_orphan_subtitles(&effective_series, true, dry_run, &mut report);
        Self::clean_junk_and_orphan_subtitles(&effective_movies, true, dry_run, &mut report);

        // 4. Limpiar carpetas entrantes y archivos temporales huérfanos
        Self::clean_incoming(state, dry_run, &mut report).await;

        // 5. Cancelar descargas activas redundantes
        Self::cancel_redundant_downloads(state, dry_run, &mut report).await;

        // 6. Purgar directorios vacíos, preservando carpetas para medios rastreados de la biblioteca
        let protected_dirs: Vec<PathBuf> = {
            let dl_svc = state.downloads.lock().await;
            let items = dl_svc.db().list_media_items(None, None).unwrap_or_default();
            drop(dl_svc);
            let mut list = Vec::new();
            for m in items {
                if let Some(ref fp) = m.file_path {
                    if let Some(p) = PathBuf::from(fp).parent() {
                        list.push(p.to_path_buf());
                    }
                }
                let clean_t = clean_movie_title(&m.title).to_lowercase();
                let norm_t = crate::services::scorer::normalize_accents_and_symbols(&clean_t);
                if m.media_type == "movie" {
                    if let Ok(entries) = std::fs::read_dir(&effective_movies) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.is_dir() {
                                let name = e.file_name().to_string_lossy().to_lowercase();
                                let norm_n = crate::services::scorer::normalize_accents_and_symbols(&name);
                                if name.contains(&clean_t) || norm_n.contains(&norm_t) {
                                    list.push(p);
                                }
                            }
                        }
                    }
                } else if m.media_type == "tv" {
                    if let Ok(entries) = std::fs::read_dir(&effective_series) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.is_dir() {
                                let name = e.file_name().to_string_lossy().to_lowercase();
                                let norm_n = crate::services::scorer::normalize_accents_and_symbols(&name);
                                if name.contains(&clean_t) || norm_n.contains(&norm_t) {
                                    list.push(p);
                                }
                            }
                        }
                    }
                }
            }
            list
        };

        Self::purge_empty_dirs_recursive(&effective_series, &protected_dirs, dry_run, &mut report);
        Self::purge_empty_dirs_recursive(&effective_movies, &protected_dirs, dry_run, &mut report);

        info!(
            "[CLEANER] Finished media cleanup: {} duplicate eps, {} duplicate movies, {} junk files, {} orphan subs, {} bytes freed",
            report.duplicate_episodes_removed.len(),
            report.duplicate_movies_removed.len(),
            report.junk_files_removed.len() + report.incoming_cleaned.len(),
            report.orphan_subtitles_removed.len(),
            report.bytes_freed
        );

        report.to_json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_sample_or_trailer() {
        assert!(MediaCleaner::is_sample_or_trailer(
            Path::new("Show.S01E01.sample.mkv"),
            10 * 1024 * 1024
        ));
        assert!(MediaCleaner::is_sample_or_trailer(
            Path::new("Movie.trailer.mp4"),
            50 * 1024 * 1024
        ));
        assert!(!MediaCleaner::is_sample_or_trailer(
            Path::new("Show.S01E01.sample.mkv"),
            100 * 1024 * 1024
        ));
        assert!(!MediaCleaner::is_sample_or_trailer(
            Path::new("Show.S01E01.mkv"),
            10 * 1024 * 1024
        ));
    }

    #[test]
    fn test_clean_junk_and_orphan_subtitles() {
        let dir = tempdir().unwrap();
        let p = dir.path();

        let video1 = p.join("Movie.1080p.mkv");
        let valid_sub = p.join("Movie.1080p.es.srt");
        let nfo = p.join("movie.nfo");
        let sample = p.join("sample.mkv");
        let thumb = p.join("Movie-thumb.jpg");
        let poster = p.join("poster.jpg");
        let malware = p.join("setup.exe");

        std::fs::write(&video1, b"main video content").unwrap();
        std::fs::write(&valid_sub, b"subtitle content").unwrap();
        std::fs::write(&nfo, b"<movie><title>Test</title></movie>").unwrap();
        std::fs::write(&sample, b"small sample clip").unwrap();
        std::fs::write(&thumb, b"episode thumbnail").unwrap();
        std::fs::write(&poster, b"wanted poster art").unwrap();
        std::fs::write(&malware, b"dangerous binary").unwrap();

        let mut report = CleanReport::default();
        // Modo biblioteca de prueba: ¡las portadas, metadatos nfo y subtítulos se preservan!
        MediaCleaner::clean_junk_and_orphan_subtitles(p, true, false, &mut report);

        assert!(video1.exists(), "Main video must be kept");
        assert!(valid_sub.exists(), "Companion subtitle must be kept in library");
        assert!(poster.exists(), "Jellyfin poster art must be kept in library");
        assert!(thumb.exists(), "Episode thumbnail must be kept in library");
        assert!(nfo.exists(), "Jellyfin .nfo metadata must be kept in library");
        assert!(!malware.exists(), "Dangerous executable malware must be removed");
        assert!(!sample.exists(), "Sample video clip must be removed");
        assert_eq!(report.junk_files_removed.len(), 2);
    }

    #[test]
    fn test_purge_empty_dirs() {
        let dir = tempdir().unwrap();
        let p = dir.path();

        let empty_sub = p.join("EmptySeries").join("Season 01");
        std::fs::create_dir_all(&empty_sub).unwrap();

        let protected_sub = p.join("ProtectedWantedMovie (2024)");
        std::fs::create_dir_all(&protected_sub).unwrap();

        let non_empty = p.join("ValidSeries").join("Season 01");
        std::fs::create_dir_all(&non_empty).unwrap();
        std::fs::write(non_empty.join("Episode.mkv"), b"video").unwrap();

        let mut report = CleanReport::default();
        let protected = vec![protected_sub.clone()];
        MediaCleaner::purge_empty_dirs_recursive(p, &protected, false, &mut report);

        assert!(!empty_sub.exists(), "Unprotected empty directory must be purged");
        assert!(protected_sub.exists(), "Protected empty directory must NOT be purged");
        assert!(non_empty.exists(), "Non-empty directory must be kept");
    }

    #[test]
    fn test_canonical_movie_key_and_scoring() {
        let movies = vec![mule_core::model::MediaItem {
            id: "m1".to_string(),
            media_type: "movie".to_string(),
            title: "The Matrix".to_string(),
            original_title: None,
            year: Some(1999),
            tmdb_id: Some(603),
            tvdb_id: None,
            imdb_id: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            quality_profile: "1080p-es".to_string(),
            monitored: true,
            status: "completed".to_string(),
            root_folder: "/movies".to_string(),
            file_path: None,
            original_release_name: None,
            release_date: None,
            digital_release_date: None,
            release_status: None,
            download_id: None,
            created_at: 0,
            updated_at: 0,
        }];

        let (k1, _) = MediaCleaner::canonical_movie_key("The Matrix (1999)", &movies);
        let (k2, _) = MediaCleaner::canonical_movie_key("The Matrix (1999) [tmdbid-603]", &movies);
        assert_eq!(k1, "tmdb:603");
        assert_eq!(k1, k2);

        let hd = Path::new("/movies/The Matrix (1999)/The Matrix (1999) [1080p] Castellano.mkv");
        let sd = Path::new("/movies/The Matrix (1999)/The Matrix (1999) [720p].avi");
        assert!(MediaCleaner::score_candidate(hd, 3_000_000_000) > MediaCleaner::score_candidate(sd, 1_500_000_000));
    }
}
