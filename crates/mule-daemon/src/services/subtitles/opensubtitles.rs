// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Cliente HTTP para OpenSubtitles (API REST heredada VLSub y API REST v1 opcional).

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;
use serde::Deserialize;
use tracing::{info, warn};

use super::compute_opensubtitles_hash_async;
use super::scorer::detect_spanish_variant;

/// Tiempo de espera HTTP para peticiones a OpenSubtitles (en segundos).
const OPENSUBTITLES_HTTP_TIMEOUT_SECS: u64 = 12;

static SUBTITLE_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn subtitle_http_client() -> &'static reqwest::Client {
    SUBTITLE_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(OPENSUBTITLES_HTTP_TIMEOUT_SECS))
            .user_agent("VLSub 0.10.2")
            .build()
            .unwrap_or_default()
    })
}

#[derive(Deserialize)]
struct OpenSubtitlesRawItem {
    #[serde(rename = "IDSubtitleFile")]
    id_subtitle_file: Option<String>,
    #[serde(rename = "SubFileName")]
    sub_file_name: Option<String>,
    #[serde(rename = "MovieReleaseName")]
    movie_release_name: Option<String>,
    #[serde(rename = "MovieFPS")]
    movie_fps: Option<String>,
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
    #[serde(rename = "SubBad")]
    sub_bad: Option<String>,
    #[serde(rename = "SubFromTrusted")]
    sub_from_trusted: Option<String>,
    #[serde(rename = "SubAutoTranslation")]
    sub_auto_translation: Option<String>,
    #[serde(rename = "SeriesSeason")]
    series_season: Option<String>,
    #[serde(rename = "SeriesEpisode")]
    series_episode: Option<String>,
}

/// Elemento crudo normalizado procedente de un proveedor de subtítulos.
#[derive(Debug, Clone)]
pub struct RawSubtitleItem {
    pub provider: String,
    pub id: String,
    pub title: String,
    pub release_name: Option<String>,
    pub language: String,
    pub format: String,
    pub download_link: String,
    pub downloads: u64,
    pub rating: String,
    pub matched_by: String,
    pub is_hi: bool,
    pub is_bad: bool,
    pub is_trusted: bool,
    pub is_auto_translated: bool,
    pub fps_milli: Option<u32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub variant: Option<String>,
}

/// Normaliza un código de idioma para la API REST heredada de OpenSubtitles (ISO 639-2 de 3 letras).
pub fn to_opensubtitles_lang_code(lang: &str) -> &str {
    match lang.trim().to_lowercase().as_str() {
        "es" | "spa" | "spanish" | "castellano" | "es-es" | "es-la" => "spa",
        "en" | "eng" | "english" => "eng",
        _ => lang,
    }
}

/// Construye la URL de búsqueda por hash incluyendo obligatoriamente `moviebytesize`.
pub fn build_hash_search_url(size: u64, hash: &str, lang_code: &str) -> String {
    format!(
        "https://rest.opensubtitles.org/search/moviebytesize-{size}/moviehash-{hash}/sublanguageid-{lang_code}"
    )
}

/// Construye la URL de búsqueda por IMDb ID, incluyendo `season`/`episode` en series.
pub fn build_imdb_search_url(
    clean_imdb: &str,
    is_movie: bool,
    season_ep: Option<(u32, u32)>,
    lang_code: &str,
) -> String {
    if !is_movie {
        if let Some((s, e)) = season_ep {
            return format!(
                "https://rest.opensubtitles.org/search/imdbid-{clean_imdb}/season-{s}/episode-{e}/sublanguageid-{lang_code}"
            );
        }
    }
    format!("https://rest.opensubtitles.org/search/imdbid-{clean_imdb}/sublanguageid-{lang_code}")
}

/// Construye la URL de búsqueda textual en OpenSubtitles.
pub fn build_query_search_url(
    title: &str,
    year: Option<u32>,
    is_movie: bool,
    season_ep: Option<(u32, u32)>,
    lang_code: &str,
) -> String {
    let clean_title = title
        .replace([':', '/', '\\', '(', ')', '[', ']', '.', '_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("+");

    if !is_movie {
        if let Some((s, e)) = season_ep {
            return format!(
                "https://rest.opensubtitles.org/search/query-{clean_title}/season-{s}/episode-{e}/sublanguageid-{lang_code}"
            );
        }
        return format!(
            "https://rest.opensubtitles.org/search/query-{clean_title}/sublanguageid-{lang_code}"
        );
    }

    if let Some(y) = year {
        format!(
            "https://rest.opensubtitles.org/search/query-{clean_title}+{y}/sublanguageid-{lang_code}"
        )
    } else {
        format!(
            "https://rest.opensubtitles.org/search/query-{clean_title}/sublanguageid-{lang_code}"
        )
    }
}

fn parse_fps_str_to_milli(raw: Option<&str>) -> Option<u32> {
    let s = raw?.trim();
    if s.is_empty() || s == "0" || s == "0.0" || s == "0.000" {
        return None;
    }
    let val: f64 = s.parse().ok()?;
    if !val.is_finite() || val < 10.0 || val > 240.0 {
        return None;
    }
    Some((val * 1000.0).round() as u32)
}

async fn fetch_legacy_items(url: &str) -> Vec<RawSubtitleItem> {
    let client = subtitle_http_client();
    let Ok(resp) = client.get(url).send().await else {
        return Vec::new();
    };
    let Ok(bytes) = resp.bytes().await else {
        return Vec::new();
    };
    match serde_json::from_slice::<Vec<OpenSubtitlesRawItem>>(&bytes) {
        Ok(items) => items
            .into_iter()
            .filter_map(|it| {
                let link = it.sub_download_link?;
                let id = it
                    .id_subtitle_file
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let title = it
                    .sub_file_name
                    .unwrap_or_else(|| "subtitle.srt".to_string());
                let release_name = it.movie_release_name.filter(|s| !s.trim().is_empty());
                let combined_lower = format!(
                    "{} {}",
                    title.to_lowercase(),
                    release_name.as_deref().unwrap_or("").to_lowercase()
                );
                let variant = detect_spanish_variant(&combined_lower);
                Some(RawSubtitleItem {
                    provider: "opensubtitles".to_string(),
                    id,
                    title,
                    release_name,
                    language: it.sub_language_id.unwrap_or_else(|| "spa".to_string()),
                    format: it.sub_format.unwrap_or_else(|| "srt".to_string()),
                    download_link: link,
                    downloads: it
                        .sub_downloads_cnt
                        .and_then(|d| d.parse::<u64>().ok())
                        .unwrap_or(0),
                    rating: it.sub_rating.unwrap_or_else(|| "0.0".to_string()),
                    matched_by: it.matched_by.unwrap_or_else(|| "query".to_string()),
                    is_hi: it.sub_hearing_impaired.as_deref() == Some("1"),
                    is_bad: it.sub_bad.as_deref() == Some("1"),
                    is_trusted: it.sub_from_trusted.as_deref() == Some("1"),
                    is_auto_translated: it.sub_auto_translation.as_deref() == Some("1"),
                    fps_milli: parse_fps_str_to_milli(it.movie_fps.as_deref()),
                    season: it.series_season.and_then(|s| s.parse::<u32>().ok()),
                    episode: it.series_episode.and_then(|e| e.parse::<u32>().ok()),
                    variant,
                })
            })
            .collect(),
        Err(e) => {
            warn!("[SUBTITLES] Error analizando respuesta de OpenSubtitles ({url}): {e}");
            Vec::new()
        }
    }
}

/// Consulta OpenSubtitles combinando hash, IMDb ID (con episodio en series) y búsqueda textual.
pub async fn search_opensubtitles_combined(
    video_path: Option<&Path>,
    title: &str,
    year: Option<u32>,
    imdb_id: Option<&str>,
    is_movie: bool,
    season_ep: Option<(u32, u32)>,
    lang: &str,
) -> Vec<RawSubtitleItem> {
    let lang_code = to_opensubtitles_lang_code(lang);
    let mut all_items = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Estrategia 1: Hash + tamaño exacto de archivo
    if let Some(p) = video_path {
        if let Ok((hash, size)) = compute_opensubtitles_hash_async(p).await {
            let url = build_hash_search_url(size, &hash, lang_code);
            let hash_items = fetch_legacy_items(&url).await;
            if !hash_items.is_empty() {
                info!(
                    "[SUBTITLES] Encontradas {} coincidencias por hash exacto en OpenSubtitles",
                    hash_items.len()
                );
            }
            for item in hash_items {
                if seen_ids.insert(item.id.clone()) {
                    all_items.push(item);
                }
            }
        }
    }

    // Estrategia 2: IMDb ID (con season/episode obligatorio en series)
    if let Some(imdb) = imdb_id {
        let clean_imdb = imdb.trim_start_matches("tt");
        if !clean_imdb.is_empty() {
            let url = build_imdb_search_url(clean_imdb, is_movie, season_ep, lang_code);
            let imdb_items = fetch_legacy_items(&url).await;
            if !imdb_items.is_empty() {
                info!(
                    "[SUBTITLES] Encontradas {} coincidencias por IMDb '{}' en OpenSubtitles",
                    imdb_items.len(),
                    imdb
                );
            }
            for item in imdb_items {
                if seen_ids.insert(item.id.clone()) {
                    all_items.push(item);
                }
            }
        }
    }

    // Estrategia 3: Consulta textual (si aún hay pocos candidatos o no hubo hash)
    let has_hash = all_items.iter().any(|i| i.matched_by == "moviehash");
    if !has_hash || all_items.len() < 5 {
        let url = build_query_search_url(title, year, is_movie, season_ep, lang_code);
        let query_items = fetch_legacy_items(&url).await;
        if !query_items.is_empty() {
            info!(
                "[SUBTITLES] Encontradas {} coincidencias por texto para '{}' en OpenSubtitles",
                query_items.len(),
                title
            );
        }
        for item in query_items {
            if seen_ids.insert(item.id.clone()) {
                all_items.push(item);
            }
        }
    }

    all_items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opensubtitles_urls_include_size_and_series_episode() {
        let hash_url = build_hash_search_url(1_234_567_890, "0123456789abcdef", "spa");
        assert!(hash_url.contains("moviebytesize-1234567890"));
        assert!(hash_url.contains("moviehash-0123456789abcdef"));

        let series_imdb = build_imdb_search_url("0944947", false, Some((3, 9)), "spa");
        assert!(series_imdb.contains("imdbid-0944947"));
        assert!(series_imdb.contains("season-3/episode-9"));

        let movie_imdb = build_imdb_search_url("0133093", true, None, "spa");
        assert!(!movie_imdb.contains("season-"));
    }
}
