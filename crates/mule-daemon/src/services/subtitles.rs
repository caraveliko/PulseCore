// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::services::media_probe::probe_video_file;
use crate::state::AppState;

/// Taille minimale de fichier requise pour le hash OpenSubtitles 64-bit (128 Ko).
const MIN_OPENSUBTITLES_HASH_FILE_SIZE: u64 = 131_072;
/// Nombre de mots 64-bit lus au début et à la fin du fichier (64 Ko = 8192 * 8 octets).
const OPENSUBTITLES_CHUNK_WORDS: usize = 8192;
/// Décalage de fin pour les 64 derniers Ko.
const OPENSUBTITLES_END_OFFSET: i64 = -65_536;
/// Timeout HTTP pour OpenSubtitles (en secondes).
const OPENSUBTITLES_HTTP_TIMEOUT_SECS: u64 = 10;

static SUBTITLE_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn subtitle_http_client() -> &'static reqwest::Client {
    SUBTITLE_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(OPENSUBTITLES_HTTP_TIMEOUT_SECS))
            .user_agent("VLSub 0.10.2")
            .build()
            .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCandidate {
    pub id: String,
    pub title: String,
    pub language: String,
    pub format: String,
    pub download_link: String,
    pub downloads: u64,
    pub rating: String,
    pub matched_by: String,
    pub is_hi: bool,
}

#[derive(Deserialize)]
struct OpenSubtitlesRawItem {
    #[serde(rename = "IDSubtitleFile")]
    id_subtitle_file: Option<String>,
    #[serde(rename = "SubFileName")]
    sub_file_name: Option<String>,
    #[serde(rename = "SubLanguageID")]
    sub_language_id: Option<String>,
    #[serde(rename = "SubFormat")]
    sub_format: Option<String>,
    #[serde(rename = "SubDownloadLink")]
    sub_download_link: Option<String>,
    #[serde(rename = "SubDownloadsCnt")]
    sub_downloads_cnt: Option<String>,
    #[serde(rename = "SubRating")]
    sub_rating: Option<String>,
    #[serde(rename = "MatchedBy")]
    matched_by: Option<String>,
    #[serde(rename = "SubHearingImpaired")]
    sub_hearing_impaired: Option<String>,
}

/// Computes the standard OpenSubtitles 64-bit hash (first 64KB + last 64KB + filesize)
pub fn compute_opensubtitles_hash(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    if file_size < MIN_OPENSUBTITLES_HASH_FILE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Video file too small for OpenSubtitles hash (<128KB)",
        ));
    }

    let mut hash = file_size;
    let mut buf = [0u8; 8];

    // Read first 64 KB (8192 64-bit words)
    for _ in 0..OPENSUBTITLES_CHUNK_WORDS {
        file.read_exact(&mut buf)?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    // Seek to last 64 KB
    file.seek(SeekFrom::End(OPENSUBTITLES_END_OFFSET))?;
    for _ in 0..OPENSUBTITLES_CHUNK_WORDS {
        file.read_exact(&mut buf)?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    Ok((format!("{:016x}", hash), file_size))
}

/// Version asynchrone non-bloquante (`spawn_blocking`) du calcul de hash OpenSubtitles.
pub async fn compute_opensubtitles_hash_async(path: &Path) -> std::io::Result<(String, u64)> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || compute_opensubtitles_hash(&p))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

/// Decode subtitle bytes converting legacy Windows-1252 / ISO-8859-1 encodings into clean UTF-8
pub fn decode_subtitle_bytes(raw: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.strip_prefix('\u{feff}').unwrap_or(s).to_string();
    }

    // Windows-1252 / ISO-8859-1 character table mapping
    raw.iter()
        .map(|&b| match b {
            0x80 => '€',
            0x82 => '‚',
            0x83 => 'ƒ',
            0x84 => '„',
            0x85 => '…',
            0x86 => '†',
            0x87 => '‡',
            0x88 => 'ˆ',
            0x89 => '‰',
            0x8A => 'Š',
            0x8B => '‹',
            0x8C => 'Œ',
            0x8E => 'Ž',
            0x91 => '‘',
            0x92 => '’',
            0x93 => '“',
            0x94 => '”',
            0x95 => '•',
            0x96 => '–',
            0x97 => '—',
            0x98 => '˜',
            0x99 => '™',
            0x9A => 'š',
            0x9B => '›',
            0x9C => 'œ',
            0x9E => 'ž',
            0x9F => 'Ÿ',
            _ => b as char,
        })
        .collect()
}

pub struct SubtitleService;

impl SubtitleService {
    /// Detect which companion subtitle files already exist alongside the video file
    pub fn detect_existing_subtitles(video_path: &Path) -> Vec<String> {
        let parent = match video_path.parent() {
            Some(p) => p,
            None => return Vec::new(),
        };

        let stem = match video_path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => return Vec::new(),
        };

        let mut langs = Vec::new();

        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(stem) {
                    let lower = name.to_lowercase();
                    if lower.ends_with(".srt") || lower.ends_with(".vtt") || lower.ends_with(".sub") {
                        if lower.contains(".es.") || lower.contains(".spa.") || lower.contains(".spanish.") {
                            if !langs.contains(&"es".to_string()) {
                                langs.push("es".to_string());
                            }
                        } else if lower.contains(".en.") || lower.contains(".eng.") || lower.contains(".english.") {
                            if !langs.contains(&"en".to_string()) {
                                langs.push("en".to_string());
                            }
                        } else if lower == format!("{}.srt", stem.to_lowercase())
                            && !langs.contains(&"es".to_string())
                        {
                            langs.push("es".to_string()); // Default generic .srt
                        }
                    }
                }
            }
        }

        langs
    }

    pub async fn detect_existing_subtitles_async(video_path: &Path) -> Vec<String> {
        let p = video_path.to_path_buf();
        tokio::task::spawn_blocking(move || Self::detect_existing_subtitles(&p))
            .await
            .unwrap_or_default()
    }

    /// Search subtitle candidates on OpenSubtitles REST API
    pub async fn search_subtitles(
        video_path: Option<&Path>,
        title: &str,
        year: Option<u32>,
        imdb_id: Option<&str>,
        is_movie: bool,
        season_ep: Option<(u32, u32)>,
        lang: &str,
    ) -> Result<Vec<SubtitleCandidate>, String> {
        let lang_code = if lang == "es" || lang == "spanish" || lang == "castellano" {
            "spa"
        } else {
            lang
        };

        let client = subtitle_http_client();

        // Strategy 1: Hash match (100% sync, non-blocking I/O)
        if let Some(p) = video_path {
            if let Ok((hash, _size)) = compute_opensubtitles_hash_async(p).await {
                let url = format!(
                    "https://rest.opensubtitles.org/search/moviehash-{}/sublanguageid-{}",
                    hash, lang_code
                );
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Ok(items) = serde_json::from_slice::<Vec<OpenSubtitlesRawItem>>(&bytes) {
                            if !items.is_empty() {
                                info!("[SUBTITLES] Found {} exact hash matches on OpenSubtitles!", items.len());
                                return Ok(Self::convert_raw_items(items));
                            }
                        }
                    }
                }
            }
        }

        // Strategy 2: IMDb ID match
        if let Some(imdb) = imdb_id {
            let clean_imdb = imdb.trim_start_matches("tt");
            if !clean_imdb.is_empty() {
                let url = format!(
                    "https://rest.opensubtitles.org/search/imdbid-{}/sublanguageid-{}",
                    clean_imdb, lang_code
                );
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Ok(items) = serde_json::from_slice::<Vec<OpenSubtitlesRawItem>>(&bytes) {
                            if !items.is_empty() {
                                info!("[SUBTITLES] Found {} IMDb matches on OpenSubtitles for ID '{}'", items.len(), imdb);
                                return Ok(Self::convert_raw_items(items));
                            }
                        }
                    }
                }
            }
        }

        // Strategy 3: Text query match
        let clean_title = title.replace([':', '/', '\\', '(', ')', '[', ']', '.', '_', '-'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("+");

        let query_url = if !is_movie {
            if let Some((s, e)) = season_ep {
                format!(
                    "https://rest.opensubtitles.org/search/query-{}/season-{}/episode-{}/sublanguageid-{}",
                    clean_title, s, e, lang_code
                )
            } else {
                format!(
                    "https://rest.opensubtitles.org/search/query-{}/sublanguageid-{}",
                    clean_title, lang_code
                )
            }
        } else if let Some(y) = year {
            format!(
                "https://rest.opensubtitles.org/search/query-{}+{}/sublanguageid-{}",
                clean_title, y, lang_code
            )
        } else {
            format!(
                "https://rest.opensubtitles.org/search/query-{}/sublanguageid-{}",
                clean_title, lang_code
            )
        };

        let resp = client.get(&query_url).send().await.map_err(|e| e.to_string())?;
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;

        match serde_json::from_slice::<Vec<OpenSubtitlesRawItem>>(&bytes) {
            Ok(items) => {
                info!("[SUBTITLES] Found {} query matches for '{}' on OpenSubtitles", items.len(), title);
                Ok(Self::convert_raw_items(items))
            }
            Err(e) => {
                warn!("[SUBTITLES] OpenSubtitles response parsing failed: {}", e);
                Ok(Vec::new())
            }
        }
    }

    fn convert_raw_items(items: Vec<OpenSubtitlesRawItem>) -> Vec<SubtitleCandidate> {
        items
            .into_iter()
            .filter_map(|it| {
                let link = it.sub_download_link?;
                let title = it.sub_file_name.unwrap_or_else(|| "subtitle.srt".to_string());
                let downloads = it.sub_downloads_cnt.and_then(|d| d.parse::<u64>().ok()).unwrap_or(0);
                let is_hi = it.sub_hearing_impaired.as_deref() == Some("1");

                Some(SubtitleCandidate {
                    id: it.id_subtitle_file.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    title,
                    language: it.sub_language_id.unwrap_or_else(|| "spa".to_string()),
                    format: it.sub_format.unwrap_or_else(|| "srt".to_string()),
                    download_link: link,
                    downloads,
                    rating: it.sub_rating.unwrap_or_else(|| "0.0".to_string()),
                    matched_by: it.matched_by.unwrap_or_else(|| "query".to_string()),
                    is_hi,
                })
            })
            .collect()
    }

    /// Download a subtitle from a URL, decompress gzip if needed, decode to UTF-8, and save to target path
    pub async fn download_and_save_subtitle(
        download_url: &str,
        target_srt_path: &Path,
    ) -> Result<(), String> {
        let client = subtitle_http_client();
        let resp = client.get(download_url).send().await.map_err(|e| e.to_string())?;
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;

        let target_buf = target_srt_path.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            // Gzip decompression if required
            let raw_data = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
                let mut gz = GzDecoder::new(&bytes[..]);
                let mut out = Vec::new();
                gz.read_to_end(&mut out)
                    .map_err(|e| format!("Gzip decompression failed: {e}"))?;
                out
            } else {
                bytes.to_vec()
            };

            // Normalize character encoding (Windows-1252/ISO-8859-1 -> UTF-8)
            let utf8_text = decode_subtitle_bytes(&raw_data);

            // Ensure parent directory exists
            if let Some(parent) = target_buf.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create subtitle dir: {e}"))?;
            }

            std::fs::write(&target_buf, utf8_text.as_bytes())
                .map_err(|e| format!("Failed to write subtitle file: {e}"))?;

            Ok(())
        })
        .await
        .map_err(|e| format!("Subtitle write task error: {e}"))??;

        info!(
            "[SUBTITLES] Saved subtitle successfully: '{}'",
            target_srt_path.display()
        );
        Ok(())
    }

    /// Auto-fetch companion subtitle for a newly organized video file
    pub async fn auto_fetch_for_video(
        video_path: &Path,
        title: &str,
        year: Option<u32>,
        imdb_id: Option<&str>,
        is_movie: bool,
        season_ep: Option<(u32, u32)>,
        lang: &str,
    ) -> Result<Option<PathBuf>, String> {
        let existing = Self::detect_existing_subtitles_async(video_path).await;
        let normalized_lang = if lang == "spa" || lang == "spanish" || lang == "castellano" {
            "es"
        } else {
            lang
        };

        if existing.contains(&normalized_lang.to_string()) {
            return Ok(None);
        }

        // Vérifier via MediaProbe si le conteneur embarque déjà des sous-titres espagnols ET de l'audio espagnol
        if normalized_lang == "es" {
            let probe = probe_video_file(video_path).await;
            if probe.has_spanish_subtitles && probe.has_spanish_audio {
                info!(
                    "[SUBTITLES] Container '{}' already embeds Spanish audio & subtitles; skipping external .srt fetch.",
                    video_path.display()
                );
                return Ok(None);
            }
        }

        let candidates = Self::search_subtitles(
            Some(video_path),
            title,
            year,
            imdb_id,
            is_movie,
            season_ep,
            lang,
        )
        .await?;

        if candidates.is_empty() {
            info!("[SUBTITLES] No subtitle candidates found on OpenSubtitles for '{}'", title);
            return Ok(None);
        }

        // Pick the best candidate:
        // 1. Matched by moviehash (perfect sync)
        // 2. Otherwise sort by highest download count
        let best = candidates
            .iter()
            .find(|c| c.matched_by == "moviehash")
            .or_else(|| candidates.iter().max_by_key(|c| c.downloads))
            .ok_or_else(|| "No suitable subtitle candidate".to_string())?;

        let target_srt = video_path.with_extension(format!("{normalized_lang}.srt"));
        Self::download_and_save_subtitle(&best.download_link, &target_srt).await?;

        Ok(Some(target_srt))
    }

    /// Sweep entire media library (movies + TV episodes) and auto-fetch missing Spanish subtitles
    pub async fn auto_fetch_all_missing(state: &AppState, lang: &str) -> Value {
        let mut downloaded = Vec::new();

        // 1. Snapshot unique de la base SQLite (movies + shows + episodes complétés)
        let (movies, completed_episodes) = {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();
            let movies = db
                .list_media_items(Some("movie"), Some("completed"))
                .unwrap_or_default();
            let shows = db
                .list_media_items(Some("tv"), None)
                .unwrap_or_default();
            let mut ep_targets = Vec::new();
            for show in shows {
                if let Ok(eps) = db.get_episodes_for_media(&show.id) {
                    for ep in eps {
                        if ep.status == "completed" {
                            if let Some(fp) = ep.file_path {
                                if !fp.is_empty() {
                                    ep_targets.push((
                                        show.title.clone(),
                                        show.year,
                                        show.imdb_id.clone(),
                                        ep.season_number,
                                        ep.episode_number,
                                        fp,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            (movies, ep_targets)
        };

        for m in movies {
            if let Some(ref fp) = m.file_path {
                let p = Path::new(fp);
                if p.is_file() {
                    if let Ok(Some(sub_file)) = Self::auto_fetch_for_video(
                        p,
                        &m.title,
                        m.year,
                        m.imdb_id.as_deref(),
                        true,
                        None,
                        lang,
                    )
                    .await
                    {
                        downloaded.push(json!({
                            "type": "movie",
                            "title": m.title,
                            "subtitle": sub_file.to_string_lossy()
                        }));
                    }
                }
            }
        }

        for (show_title, show_year, show_imdb, s_num, e_num, fp) in completed_episodes {
            let p = Path::new(&fp);
            if p.is_file() {
                if let Ok(Some(sub_file)) = Self::auto_fetch_for_video(
                    p,
                    &show_title,
                    show_year,
                    show_imdb.as_deref(),
                    false,
                    Some((s_num, e_num)),
                    lang,
                )
                .await
                {
                    downloaded.push(json!({
                        "type": "episode",
                        "title": format!("{show_title} S{s_num:02}E{e_num:02}"),
                        "subtitle": sub_file.to_string_lossy()
                    }));
                }
            }
        }

        json!({
            "downloaded": downloaded
        })
    }
}
