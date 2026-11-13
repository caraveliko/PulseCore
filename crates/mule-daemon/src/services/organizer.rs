// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::path::{Path, PathBuf};
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use crate::state::AppState;
use mule_core::model::{EpisodeItem, MediaItem};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "mov", "wmv", "flv", "ts",
];
/// Taille minimale d'un fichier vidéo autonome dans un dossier incoming supplémentaire (50 Mio).
const MIN_STANDALONE_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
/// Taille maximale considérée como extrait « sample / trailer » (85 Mio).
const MAX_SAMPLE_VIDEO_BYTES: u64 = 85 * 1024 * 1024;

/// Fix Latin-1 / UTF-8 Mojibake in filenames (e.g. "La cabaÃ±a" -> "La cabaña")
pub fn fix_mojibake(s: &str) -> String {
    mule_format::fix_mojibake(s)
}

/// Extract year from string e.g. "La cabaña (2017)" or "Oddity.2024.1080p"
pub fn extract_year(name: &str) -> Option<u32> {
    if let Some(open) = name.find('(') {
        let rest = &name[open + 1..];
        if let Some(close) = rest.find(')') {
            let inside = rest[..close].trim();
            if inside.len() == 4 {
                if let Ok(y) = inside.parse::<u32>() {
                    if (1900..=2050).contains(&y) {
                        return Some(y);
                    }
                }
            }
        }
    }
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    for i in 0..n {
        if i + 4 <= n && chars[i..i + 4].iter().all(|c| c.is_ascii_digit()) {
            let prev_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
            let next_boundary = (i + 4 == n) || !chars[i + 4].is_alphanumeric();
            if prev_boundary && next_boundary {
                let s: String = chars[i..i + 4].iter().collect();
                if let Ok(y) = s.parse::<u32>() {
                    if (1900..=2050).contains(&y) {
                        return Some(y);
                    }
                }
            }
        }
    }
    None
}

/// Clean movie title from filename, stripping year and common tags
pub fn clean_movie_title(name: &str) -> String {
    let mut title = name.split('(').next().unwrap_or(name);
    if let Some(pos) = title.find('[') {
        title = &title[..pos];
    }
    let lower = title.to_lowercase();
    let mut end = title.len();
    for tag in &[
        "1080p", "720p", "2160p", "4k", "bluray", "bdrip", "webrip", "hdrip", "x264", "x265",
        "h264", "h265", "dvdrip", "ac3", "dts", "spanish", "castellano", "english", "ingles",
        "vose", "dual", "latino", "subs", "subtitulado", "aac", "eac3", "hevc",
    ] {
        if let Some(pos) = lower.find(tag) {
            if pos < end && pos > 0 {
                end = pos;
            }
        }
    }
    let sub = &title[..end];
    let mut cleaned = sub
        .replace(['.', '_', '-'], " ")
        .trim()
        .to_string();

    // If title ends with a 4-digit year e.g. "Oddity 2024", strip the year
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if let Some(last) = words.last() {
        if last.len() == 4 {
            if let Ok(y) = last.parse::<u32>() {
                if (1900..=2050).contains(&y) {
                    cleaned = words[..words.len() - 1].join(" ");
                }
            }
        }
    }

    cleaned.trim().to_string()
}

/// Sanitize filename characters forbidden in Linux / Unix filesystems or standard media players
pub fn sanitize_name(name: &str) -> String {
    name.replace(':', " -")
        .replace(['/', '\\'], "-")
        .replace(['*', '?', '"', '<', '>'], "")
        .replace('|', "-")
        .trim()
        .to_string()
}

/// Resolve existing series folder on disk if one already exists, otherwise construct canonical name
pub fn resolve_series_folder_name(
    root_folder: &Path,
    series_title: &str,
    year: Option<u32>,
    tvdb_id: Option<u64>,
) -> String {
    let clean_target = clean_movie_title(series_title).to_lowercase();
    let norm_target = crate::services::scorer::normalize_accents_and_symbols(&clean_target);

    if let Ok(entries) = std::fs::read_dir(root_folder) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(tid) = tvdb_id {
                if fname.contains(&format!("tvdbid-{tid}")) || fname.contains(&format!("tvdb-{tid}")) {
                    return fname;
                }
            }
            let clean_fname = clean_movie_title(&fname).to_lowercase();
            let norm_fname = crate::services::scorer::normalize_accents_and_symbols(&clean_fname);
            if clean_fname == clean_target || norm_fname == norm_target {
                if let Some(y) = year {
                    if fname.contains(&y.to_string()) {
                        return fname;
                    }
                } else {
                    return fname;
                }
            }
        }
    }

    let sanitized_title = sanitize_name(series_title);
    let year_str = year.map(|y| format!(" ({y})")).unwrap_or_default();
    if let Some(tid) = tvdb_id {
        format!("{}{year_str} [tvdbid-{tid}]", sanitized_title)
    } else {
        format!("{}{year_str}", sanitized_title)
    }
}

/// Detect quality tag from filename
pub fn detect_quality_tag(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.contains("2160p") || lower.contains("4k") || lower.contains("uhd") {
        "2160p"
    } else if lower.contains("1080p") || lower.contains("fhd") || lower.contains("bluray") || lower.contains("bdrip") {
        "1080p"
    } else if lower.contains("720p") || lower.contains("hdrip") || lower.contains("hdtv") {
        "720p"
    } else {
        "SD"
    }
}

/// Perform hardlink from source to target (or copy on EXDEV) and remove source from incoming
/// so zero duplicate files or residues remain in incoming.
pub async fn hardlink_or_copy(source: &Path, target: &Path) -> std::io::Result<()> {
    let src = source.to_path_buf();
    let tgt = target.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if src == tgt {
            return Ok(());
        }
        if tgt.exists() {
            if let (Ok(src_meta), Ok(tgt_meta)) = (std::fs::metadata(&src), std::fs::metadata(&tgt)) {
                if src_meta.len() == tgt_meta.len() {
                    // Target already has the organized file; remove source from incoming
                    if src.exists() {
                        let _ = std::fs::remove_file(&src);
                        info!(
                            "[ORGANIZER] Removed duplicate source '{}' (target '{}' already exists)",
                            src.display(),
                            tgt.display()
                        );
                    }
                    return Ok(());
                }
                if src_meta.len() > 0 {
                    let _ = std::fs::remove_file(&tgt);
                }
            }
        }

        if let Some(parent) = tgt.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match std::fs::hard_link(&src, &tgt) {
            Ok(_) => {
                if src.exists() {
                    let _ = std::fs::remove_file(&src);
                }
                info!(
                    "[ORGANIZER] Moved (hardlink + unlink source) successfully: '{}' -> '{}'",
                    src.display(),
                    tgt.display()
                );
                Ok(())
            }
            Err(e) if e.raw_os_error() == Some(18) => {
                // EXDEV: Cross-device link, incoming and media library are on separate mounts/filesystems
                info!(
                    "[ORGANIZER] Cross-device boundary (EXDEV); falling back to optimized NAS/local transfer and removing source to leave zero residues on host: '{}' -> '{}'",
                    src.display(),
                    tgt.display()
                );
                crate::services::nas_transfer::transfer_to_destination(
                    &src,
                    &tgt,
                    crate::services::nas_transfer::DEFAULT_NAS_TRANSFER_RATE_MBPS,
                    None::<fn(&crate::services::nas_transfer::NasTransferProgress)>,
                )?;
                if src.exists() {
                    let _ = std::fs::remove_file(&src);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

/// Remove companion subtitle files (.srt, .sub, .vtt) associated with a superseded video file
pub fn remove_companion_subtitles_sync(video_path: &Path) {
    let Some(stem) = video_path.file_stem().and_then(|s| s.to_str()) else {
        return;
    };
    let Some(parent) = video_path.parent() else {
        return;
    };
    for sub_ext in &["srt", "sub", "vtt"] {
        for pattern in &[
            format!("{stem}.{sub_ext}"),
            format!("{stem}.es.{sub_ext}"),
            format!("{stem}.spa.{sub_ext}"),
            format!("{stem}.en.{sub_ext}"),
        ] {
            let cand = parent.join(pattern);
            if cand.is_file() {
                let _ = std::fs::remove_file(&cand);
            }
        }
    }
}

/// Remove any superseded video files in `season_dir` that match the same `(season_num, ep_num)`
/// as `dest_file` (or match `old_file_path`), along with their companion subtitles.
fn purge_superseded_episode_files(
    season_dir: &Path,
    season_num: u32,
    ep_num: u32,
    dest_file: &Path,
    old_file_path: Option<&str>,
) {
    if let Some(old_fp) = old_file_path {
        let old_p = Path::new(old_fp);
        if !old_fp.is_empty() && old_p != dest_file && old_p.is_file() {
            info!(
                "[ORGANIZER] Removing superseded episode file '{}' (replaced by '{}')",
                old_p.display(),
                dest_file.display()
            );
            remove_companion_subtitles_sync(old_p);
            let _ = std::fs::remove_file(old_p);
        }
    }

    if let Ok(entries) = std::fs::read_dir(season_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() || p == dest_file {
                continue;
            }
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some((s, e)) = crate::services::disk_scanner::extract_season_episode(Some(season_num), &fname) {
                if s == season_num && e == ep_num {
                    info!(
                        "[ORGANIZER] Removing duplicate episode file '{}' in '{}' for S{:02}E{:02}",
                        p.display(),
                        season_dir.display(),
                        season_num,
                        ep_num
                    );
                    remove_companion_subtitles_sync(&p);
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
}

/// Remove any superseded video files in `movie_dir` other than `dest_file` (and `old_file_path`),
/// along with their companion subtitles.
fn purge_superseded_movie_files(
    movie_dir: &Path,
    dest_file: &Path,
    old_file_path: Option<&str>,
) {
    if let Some(old_fp) = old_file_path {
        let old_p = Path::new(old_fp);
        if !old_fp.is_empty() && old_p != dest_file && old_p.is_file() {
            info!(
                "[ORGANIZER] Removing superseded movie file '{}' (replaced by '{}')",
                old_p.display(),
                dest_file.display()
            );
            remove_companion_subtitles_sync(old_p);
            let _ = std::fs::remove_file(old_p);
        }
    }

    if let Ok(entries) = std::fs::read_dir(movie_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() || p == dest_file {
                continue;
            }
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                continue;
            }
            info!(
                "[ORGANIZER] Removing duplicate movie file '{}' in '{}'",
                p.display(),
                movie_dir.display()
            );
            remove_companion_subtitles_sync(&p);
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Returns true if `path` is already organized inside a subdirectory of `movies_dir` or `series_dir`.
pub fn is_already_in_organized_library(state: &AppState, path: &Path) -> bool {
    for lib_root in [&state.config.movies_dir, &state.config.series_dir] {
        if path.starts_with(lib_root) && path.parent() != Some(lib_root.as_path()) {
            return true;
        }
    }
    false
}

/// Copies or hardlinks companion subtitle files (.srt, .sub, .vtt) if present alongside video
async fn copy_companion_subtitles(source_video: &Path, dest_video: &Path) {
    let src_stem = match source_video.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return,
    };
    let parent = match source_video.parent() {
        Some(p) => p,
        None => return,
    };

    for sub_ext in &["srt", "sub", "vtt"] {
        for pattern in &[
            format!("{src_stem}.{sub_ext}"),
            format!("{src_stem}.es.{sub_ext}"),
            format!("{src_stem}.spa.{sub_ext}"),
        ] {
            let cand_sub = parent.join(pattern);
            if cand_sub.is_file() {
                let dest_sub = dest_video.with_extension(format!("es.{sub_ext}"));
                let _ = hardlink_or_copy(&cand_sub, &dest_sub).await;
                break;
            }
        }
    }
}

pub struct MediaOrganizer;

impl MediaOrganizer {
    /// Notify local media servers (Jellyfin, Plex) to refresh their libraries
    pub fn notify_media_servers(state: &AppState) {
        if state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
            return;
        }
        let jellyfin_url = state.config.jellyfin_url.clone();
        let jellyfin_key = state.config.jellyfin_api_key.clone();
        let plex_url = state.config.plex_url.clone();

        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build();

            if let Ok(client) = client {
                // 1. Jellyfin / Emby
                let token = jellyfin_key.unwrap_or_else(|| "aac2dc7a7dd13af864a06776ca806fe8".to_string());
                let url = format!("{}/Library/Refresh", jellyfin_url.trim_end_matches('/'));
                let _ = client
                    .post(&url)
                    .header("Authorization", format!("MediaBrowser Token=\"{token}\""))
                    .send()
                    .await;

                // 2. Plex Media Server
                let p_url = plex_url.unwrap_or_else(|| "http://127.0.0.1:32400/library/sections/all/refresh".to_string());
                let _ = client.get(&p_url).send().await;

                info!("[ORGANIZER] Dispatched library refresh signal to media servers (Jellyfin/Plex)");
            }
        });
    }

    /// Organize a single completed file by its download ID
    pub async fn organize_download(
        state: &AppState,
        download_id: &str,
        completed_file_path: &Path,
    ) -> Result<Option<PathBuf>, String> {
        if !completed_file_path.is_file() {
            return Ok(None);
        }

        // Skip files that are already organized inside a movie or series subfolder
        if is_already_in_organized_library(state, completed_file_path) {
            return Ok(None);
        }

        let ext = completed_file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mkv");

        if !VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
            return Ok(None);
        }

        // Reject sample / trailer clips from being organized as full episodes or movies
        if let Some(fname) = completed_file_path.file_name().and_then(|n| n.to_str()) {
            let lower = fname.to_lowercase();
            let size = completed_file_path.metadata().map(|m| m.len()).unwrap_or(0);
            if size > 0
                && size < MAX_SAMPLE_VIDEO_BYTES
                && (lower.contains("sample") || lower.contains("trailer") || lower.contains("rarbg.com"))
            {
                let _ = std::fs::remove_file(completed_file_path);
                info!(
                    "[ORGANIZER] Removed sample/trailer video clip '{}' ({} bytes)",
                    completed_file_path.display(),
                    size
                );
                return Ok(None);
            }
        }

        let now = now_ms();
        let quality = detect_quality_tag(&completed_file_path.to_string_lossy());

        // 1. Check if download corresponds to a TV Episode
        let ep_match = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().find_episode_by_download_id(download_id).ok().flatten()
        };

        if let Some(ep) = ep_match {
            let series_item = {
                let dl_svc = state.downloads.lock().await;
                dl_svc.db().get_media_item(&ep.media_id).ok().flatten()
            };

            if let Some(series) = series_item {
                let series_title = sanitize_name(&series.title);
                let series_folder = resolve_series_folder_name(
                    Path::new(&series.root_folder),
                    &series.title,
                    series.year,
                    series.tvdb_id,
                );

                let season_folder = format!("Season {:02}", ep.season_number);

                let ep_title_clean = sanitize_name(&ep.title);
                let ep_file_name = if !ep_title_clean.is_empty() && !ep_title_clean.to_lowercase().starts_with("episode") {
                    format!(
                        "{} - S{:02}E{:02} - {} [{}].{}",
                        series_title, ep.season_number, ep.episode_number, ep_title_clean, quality, ext
                    )
                } else {
                    format!(
                        "{} - S{:02}E{:02} [{}].{}",
                        series_title, ep.season_number, ep.episode_number, quality, ext
                    )
                };

                let dest_dir = PathBuf::from(&series.root_folder)
                    .join(&series_folder)
                    .join(&season_folder);
                let dest_file = dest_dir.join(&ep_file_name);

                copy_companion_subtitles(completed_file_path, &dest_file).await;

                hardlink_or_copy(completed_file_path, &dest_file)
                    .await
                    .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

                purge_superseded_episode_files(
                    &dest_dir,
                    ep.season_number,
                    ep.episode_number,
                    &dest_file,
                    ep.file_path.as_deref(),
                );

                let dest_str = dest_file.to_string_lossy().to_string();
                {
                    let mut dl_svc = state.downloads.lock().await;
                    if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                        let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                    }
                    let _ = dl_svc.db().update_episode_status(
                        &ep.id,
                        "completed",
                        Some(&dest_str),
                        None,
                        now,
                    );

                    // If all monitored episodes are now completed, mark series as completed
                    let all_eps = dl_svc.db().get_episodes_for_media(&series.id).unwrap_or_default();
                    let remaining = all_eps.iter().filter(|e| e.monitored && e.status != "completed").count();
                    if remaining == 0 {
                        let _ = dl_svc.db().update_media_status(&series.id, "completed", None, now);
                        info!("[ORGANIZER] All episodes completed for series '{}'; marked completed!", series.title);
                    }
                }

                info!(
                    "[ORGANIZER] Organized episode '{} S{:02}E{:02}' -> '{}'",
                    series.title, ep.season_number, ep.episode_number, dest_str
                );

                // Auto-fetch Spanish subtitles in background (Bazarr feature)
                let sub_target = dest_file.clone();
                let sub_title = series.title.clone();
                let sub_year = series.year;
                let sub_imdb = series.imdb_id.clone();
                let s_num = ep.season_number;
                let e_num = ep.episode_number;
                tokio::spawn(async move {
                    let _ = crate::services::SubtitleService::auto_fetch_for_video(
                        &sub_target,
                        &sub_title,
                        sub_year,
                        sub_imdb.as_deref(),
                        false,
                        Some((s_num, e_num)),
                        "spa",
                    ).await;
                });

                Self::notify_media_servers(state);
                return Ok(Some(dest_file));
            }
        }

        // 2. Check if download corresponds to a Movie
        let movie_match = {
            let dl_svc = state.downloads.lock().await;
            dl_svc.db().find_media_by_download_id(download_id).ok().flatten().filter(|m| m.media_type == "movie")
        };

        if let Some(item) = movie_match {
            let year_str = item.year.map(|y| y.to_string()).unwrap_or_else(|| "".to_string());
            let title_clean = sanitize_name(&item.title);

            let folder_name = if let Some(tmdb_id) = item.tmdb_id {
                if !year_str.is_empty() {
                    format!("{} ({}) [tmdbid-{}]", title_clean, year_str, tmdb_id)
                } else {
                    format!("{} [tmdbid-{}]", title_clean, tmdb_id)
                }
            } else if !year_str.is_empty() {
                format!("{} ({})", title_clean, year_str)
            } else {
                title_clean.clone()
            };

            let file_name = if !year_str.is_empty() {
                format!("{} ({}) [{}].{}", title_clean, year_str, quality, ext)
            } else {
                format!("{} [{}].{}", title_clean, quality, ext)
            };

            let dest_dir = PathBuf::from(&item.root_folder).join(&folder_name);
            let dest_file = dest_dir.join(&file_name);

            copy_companion_subtitles(completed_file_path, &dest_file).await;

            hardlink_or_copy(completed_file_path, &dest_file)
                .await
                .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

            purge_superseded_movie_files(&dest_dir, &dest_file, item.file_path.as_deref());

            let dest_str = dest_file.to_string_lossy().to_string();
            {
                let mut dl_svc = state.downloads.lock().await;
                if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                    let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                }
                let _ = dl_svc.db().update_media_completed(&item.id, &dest_str, now);
            }

            info!(
                "[ORGANIZER] Organized movie '{}' -> '{}'",
                item.title, dest_str
            );

            // Auto-fetch Spanish subtitles in background (Bazarr feature)
            let sub_target = dest_file.clone();
            let sub_title = item.title.clone();
            let sub_year = item.year;
            let sub_imdb = item.imdb_id.clone();
            tokio::spawn(async move {
                let _ = crate::services::SubtitleService::auto_fetch_for_video(
                    &sub_target,
                    &sub_title,
                    sub_year,
                    sub_imdb.as_deref(),
                    true,
                    None,
                    "spa",
                ).await;
            });

            Self::notify_media_servers(state);
            return Ok(Some(dest_file));
        }

        // 3. Fallback: Automatic identification for manual eMule/Kad/Torrent downloads
        let file_stem = completed_file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let raw_filename = completed_file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let clean_filename = fix_mojibake(raw_filename);
        let clean_stem = fix_mojibake(file_stem);

        // A. Is it an episode of a known series?
        if let Some((s_num, ep_num)) = crate::services::disk_scanner::extract_season_episode(None, &clean_filename) {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();
            if let Ok(shows) = db.list_media_items(Some("tv"), None) {
                let matched_show = shows.into_iter().find(|s| {
                    let st = s.title.to_lowercase();
                    let cf = clean_filename.to_lowercase();
                    cf.contains(&st) || (s.original_title.as_ref().map(|ot| cf.contains(&ot.to_lowercase())).unwrap_or(false))
                });

                if let Some(series) = matched_show {
                    let series_title = sanitize_name(&series.title);
                    let series_folder = resolve_series_folder_name(
                        Path::new(&series.root_folder),
                        &series.title,
                        series.year,
                        series.tvdb_id,
                    );
                    let season_folder = format!("Season {:02}", s_num);
                    let dest_dir = PathBuf::from(&series.root_folder)
                        .join(&series_folder)
                        .join(&season_folder);

                    let existing_ep = db.get_episodes_for_media(&series.id).ok().and_then(|eps| {
                        eps.into_iter().find(|e| e.season_number == s_num && e.episode_number == ep_num)
                    });

                    let ep_title_clean = existing_ep.as_ref().map(|e| sanitize_name(&e.title)).unwrap_or_default();
                    let ep_file_name = if !ep_title_clean.is_empty() && !ep_title_clean.to_lowercase().starts_with("episode") && !ep_title_clean.ends_with(".mkv") {
                        format!("{} - S{:02}E{:02} - {} [{}].{}", series_title, s_num, ep_num, ep_title_clean, quality, ext)
                    } else {
                        format!("{} - S{:02}E{:02} [{}].{}", series_title, s_num, ep_num, quality, ext)
                    };

                    let dest_file = dest_dir.join(&ep_file_name);
                    let old_ep_fp = existing_ep.as_ref().and_then(|e| e.file_path.clone());
                    drop(dl_svc);

                    copy_companion_subtitles(completed_file_path, &dest_file).await;

                    hardlink_or_copy(completed_file_path, &dest_file)
                        .await
                        .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

                    purge_superseded_episode_files(
                        &dest_dir,
                        s_num,
                        ep_num,
                        &dest_file,
                        old_ep_fp.as_deref(),
                    );

                    let dest_str = dest_file.to_string_lossy().to_string();
                    {
                        let mut dl_svc = state.downloads.lock().await;
                        if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                            let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                        }
                        if let Some(mut ep) = existing_ep {
                            ep.file_path = Some(dest_str.clone());
                            ep.status = "completed".to_string();
                            ep.download_id = Some(download_id.to_string());
                            ep.updated_at = now;
                            let _ = dl_svc.db().insert_episode(&ep);
                        } else {
                            let ep_id = Uuid::new_v4().to_string();
                            let ep_item = EpisodeItem {
                                id: ep_id,
                                media_id: series.id.clone(),
                                season_number: s_num,
                                episode_number: ep_num,
                                title: ep_file_name.clone(),
                                air_date: None,
                                status: "completed".to_string(),
                                monitored: true,
                                file_path: Some(dest_str.clone()),
                                download_id: Some(download_id.to_string()),
                                created_at: now,
                                updated_at: now,
                            };
                            let _ = dl_svc.db().insert_episode(&ep_item);
                        }
                    }

                    info!("[ORGANIZER] Auto-organized manual download episode '{} S{:02}E{:02}' -> '{}'", series.title, s_num, ep_num, dest_str);

                    let sub_target = dest_file.clone();
                    let sub_title = series.title.clone();
                    let sub_year = series.year;
                    let sub_imdb = series.imdb_id.clone();
                    tokio::spawn(async move {
                        let _ = crate::services::SubtitleService::auto_fetch_for_video(
                            &sub_target, &sub_title, sub_year, sub_imdb.as_deref(), false, Some((s_num, ep_num)), "spa"
                        ).await;
                    });

                    Self::notify_media_servers(state);
                    return Ok(Some(dest_file));
                }
            }
        }

        // B. Is it a Movie?
        let parsed_year = extract_year(&clean_stem);
        let parsed_title = clean_movie_title(&clean_stem);

        if !parsed_title.is_empty() {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();

            let mut matched_movie = None;
            if let Ok(movies) = db.list_media_items(Some("movie"), None) {
                let norm_pt = crate::services::scorer::normalize_accents_and_symbols(&parsed_title);
                matched_movie = movies.into_iter().find(|m| {
                    let mt = m.title.to_lowercase();
                    let pt = parsed_title.to_lowercase();
                    let norm_mt = crate::services::scorer::normalize_accents_and_symbols(&m.title);
                    let norm_orig = m.original_title.as_ref().map(|o| crate::services::scorer::normalize_accents_and_symbols(o)).unwrap_or_default();
                    (mt == pt || mt.contains(&pt) || pt.contains(&mt)
                        || norm_mt == norm_pt || norm_mt.contains(&norm_pt) || norm_pt.contains(&norm_mt)
                        || (!norm_orig.is_empty() && (norm_orig == norm_pt || norm_orig.contains(&norm_pt) || norm_pt.contains(&norm_orig))))
                        && (parsed_year.is_none() || m.year == parsed_year || m.year.is_none())
                });
            }
            drop(dl_svc);

            if matched_movie.is_none() {
                if let Ok(results) = state.media_metadata.search_movies(&parsed_title).await {
                    let best = if let Some(y) = parsed_year {
                        results.into_iter().find(|r| r.year == Some(y))
                    } else {
                        results.into_iter().next()
                    };

                    if let Some(meta) = best {
                        let id = Uuid::new_v4().to_string();
                        let item = MediaItem {
                            id: id.clone(),
                            tmdb_id: meta.tmdb_id,
                            imdb_id: meta.imdb_id,
                            tvdb_id: None,
                            media_type: "movie".to_string(),
                            title: meta.title,
                            original_title: meta.original_title,
                            year: meta.year.or(parsed_year),
                            overview: meta.overview,
                            poster_path: meta.poster_url,
                            backdrop_path: meta.backdrop_url,
                            status: "completed".to_string(),
                            monitored: true,
                            quality_profile: "1080p-es".to_string(),
                            root_folder: state.config.movies_dir.to_string_lossy().to_string(),
                            file_path: None,
                            download_id: Some(download_id.to_string()),
                            created_at: now,
                            updated_at: now,
                        };
                        let dl_svc = state.downloads.lock().await;
                        let _ = dl_svc.db().insert_media_item(&item);
                        matched_movie = Some(item);
                    }
                }
            }

            let final_movie = matched_movie.unwrap_or_else(|| {
                MediaItem {
                    id: Uuid::new_v4().to_string(),
                    tmdb_id: None,
                    imdb_id: None,
                    tvdb_id: None,
                    media_type: "movie".to_string(),
                    title: parsed_title.clone(),
                    original_title: None,
                    year: parsed_year,
                    overview: None,
                    poster_path: None,
                    backdrop_path: None,
                    status: "completed".to_string(),
                    monitored: true,
                    quality_profile: "1080p-es".to_string(),
                    root_folder: state.config.movies_dir.to_string_lossy().to_string(),
                    file_path: None,
                    download_id: Some(download_id.to_string()),
                    created_at: now,
                    updated_at: now,
                }
            });

            let year_str = final_movie.year.map(|y| y.to_string()).unwrap_or_default();
            let title_clean = sanitize_name(&final_movie.title);

            let folder_name = if let Some(tmdb_id) = final_movie.tmdb_id {
                if !year_str.is_empty() {
                    format!("{} ({}) [tmdbid-{}]", title_clean, year_str, tmdb_id)
                } else {
                    format!("{} [tmdbid-{}]", title_clean, tmdb_id)
                }
            } else if !year_str.is_empty() {
                format!("{} ({})", title_clean, year_str)
            } else {
                title_clean.clone()
            };

            let file_name = if !year_str.is_empty() {
                format!("{} ({}) [{}].{}", title_clean, year_str, quality, ext)
            } else {
                format!("{} [{}].{}", title_clean, quality, ext)
            };

            let dest_dir = PathBuf::from(&final_movie.root_folder).join(&folder_name);
            let dest_file = dest_dir.join(&file_name);

            copy_companion_subtitles(completed_file_path, &dest_file).await;

            hardlink_or_copy(completed_file_path, &dest_file)
                .await
                .map_err(|e| format!("Hardlink/copy failed: {e}"))?;

            purge_superseded_movie_files(&dest_dir, &dest_file, final_movie.file_path.as_deref());

            let dest_str = dest_file.to_string_lossy().to_string();
            {
                let mut dl_svc = state.downloads.lock().await;
                if let Ok(dl_uuid) = Uuid::parse_str(download_id) {
                    let _ = dl_svc.update_target_path(&dl_uuid, &dest_file);
                }
                let _ = dl_svc.db().update_media_completed(&final_movie.id, &dest_str, now);
            }

            info!("[ORGANIZER] Auto-organized manual download movie '{}' -> '{}'", final_movie.title, dest_str);

            let sub_target = dest_file.clone();
            let sub_title = final_movie.title.clone();
            let sub_year = final_movie.year;
            let sub_imdb = final_movie.imdb_id.clone();
            tokio::spawn(async move {
                let _ = crate::services::SubtitleService::auto_fetch_for_video(
                    &sub_target, &sub_title, sub_year, sub_imdb.as_deref(), true, None, "spa"
                ).await;
            });

            Self::notify_media_servers(state);
            return Ok(Some(dest_file));
        }

        Ok(None)
    }

    /// Full sweep over active eMule downloads and BitTorrent torrents to organize any finished files
    pub async fn sweep_and_organize(state: &AppState) -> Value {
        let mut organized_files = Vec::new();

        // 1. Check eMule downloads
        let completed_ed2k: Vec<(String, PathBuf)> = {
            let dl_svc = state.downloads.lock().await;
            let list = dl_svc.list_download_details();
            list.into_iter()
                .filter(|d| d.snapshot.status == "completed")
                .map(|d| (d.snapshot.id.to_string(), PathBuf::from(d.target_path)))
                .collect()
        };

        for (dl_id, path) in completed_ed2k {
            if path.is_file() && !is_already_in_organized_library(state, &path) {
                if let Ok(Some(dest)) = Self::organize_download(state, &dl_id, &path).await {
                    organized_files.push(json!({
                        "protocol": "ed2k",
                        "downloadId": dl_id,
                        "source": path.to_string_lossy(),
                        "target": dest.to_string_lossy()
                    }));
                }
            }
        }

        // 2. Check BitTorrent torrents
        if let Ok(torrent_engine) = state.torrent_engine().await {
            let torrents = torrent_engine.list();
            for tor in torrents {
                if tor.status == "completed" {
                    let out_dir = PathBuf::from(&tor.output_dir);
                    // Single file or directory
                    let candidate = out_dir.join(&tor.name);
                    let mut torrent_organized_any = false;
                    if candidate.is_file() {
                        if let Ok(Some(dest)) = Self::organize_download(state, &tor.info_hash, &candidate).await {
                            torrent_organized_any = true;
                            organized_files.push(json!({
                                "protocol": "torrent",
                                "downloadId": tor.info_hash,
                                "source": candidate.to_string_lossy(),
                                "target": dest.to_string_lossy()
                            }));
                        }
                    } else if candidate.is_dir() {
                        // Scan directory for video files
                        if let Ok(entries) = std::fs::read_dir(&candidate) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_file() {
                                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                                    if VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                                        if let Ok(Some(dest)) = Self::organize_download(state, &tor.info_hash, &p).await {
                                            torrent_organized_any = true;
                                            organized_files.push(json!({
                                                "protocol": "torrent",
                                                "downloadId": tor.info_hash,
                                                "source": p.to_string_lossy(),
                                                "target": dest.to_string_lossy()
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Once all video files in the completed torrent have been organized (or candidate no longer exists),
                    // remove the torrent from the engine and delete any leftover incoming folder/files.
                    if torrent_organized_any || !candidate.exists() {
                        let _ = torrent_engine.delete(tor.id, true).await;
                        if candidate.is_dir() {
                            let _ = std::fs::remove_dir_all(&candidate);
                        } else if candidate.is_file() {
                            let _ = std::fs::remove_file(&candidate);
                        }
                        info!(
                            "[ORGANIZER] Cleaned up completed torrent '{}' from incoming after organization",
                            tor.name
                        );
                    }
                }
            }
        }

        // 3. Scan default_download_dir and configured extra incoming directories for standalone video files
        let mut incoming_dirs = vec![state.config.default_download_dir.clone()];
        for d in &state.config.extra_incoming_dirs {
            if !incoming_dirs.contains(d) {
                incoming_dirs.push(d.clone());
            }
        }

        for inc_dir in &incoming_dirs {
            if inc_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(inc_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() && !is_already_in_organized_library(state, &p) {
                            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                            if size < MIN_STANDALONE_VIDEO_BYTES {
                                continue;
                            }
                            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                                let pseudo_id = format!("manual-{}", p.file_name().unwrap_or_default().to_string_lossy());
                                if let Ok(Some(dest)) = Self::organize_download(state, &pseudo_id, &p).await {
                                    organized_files.push(json!({
                                        "protocol": "manual",
                                        "downloadId": pseudo_id,
                                        "source": p.to_string_lossy(),
                                        "target": dest.to_string_lossy()
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }

        json!({
            "organized": organized_files
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_mojibake() {
        assert_eq!(fix_mojibake("La cabaÃ±a (2017)"), "La cabaña (2017)");
        assert_eq!(fix_mojibake("Normal Title (2020)"), "Normal Title (2020)");
    }

    #[test]
    fn test_extract_year_organizer() {
        assert_eq!(extract_year("La cabaña (2017)"), Some(2017));
        assert_eq!(extract_year("The.Matrix.1999.1080p"), Some(1999));
        assert_eq!(extract_year("Title Without Year"), None);
    }

    #[test]
    fn test_clean_movie_title() {
        assert_eq!(clean_movie_title("La cabaña (2017)"), "La cabaña");
        assert_eq!(clean_movie_title("The Matrix (1999) [1080p]"), "The Matrix");
        assert_eq!(clean_movie_title("Oddity.2024.1080p.x264"), "Oddity");
    }

    #[tokio::test]
    async fn test_hardlink_or_copy_removes_source_from_incoming() {
        use tempfile::tempdir;
        let incoming_dir = tempdir().unwrap();
        let library_dir = tempdir().unwrap();

        let src_file = incoming_dir.path().join("Episode.S01E01.mkv");
        std::fs::write(&src_file, b"test video data payload").unwrap();

        let tgt_file = library_dir.path().join("Series").join("Season 01").join("Series - S01E01 [1080p].mkv");

        hardlink_or_copy(&src_file, &tgt_file).await.unwrap();

        assert!(tgt_file.exists(), "Target file must exist in organized library");
        assert!(!src_file.exists(), "Source file must be unlinked from incoming directory");
        assert_eq!(std::fs::read(&tgt_file).unwrap(), b"test video data payload");
    }
}
