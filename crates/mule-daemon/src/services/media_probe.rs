// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use serde::{Deserialize, Serialize};

/// Taille maximale de l'en-tête binaire inspecté en fallback sans `ffprobe` (256 Ko).
const HEADER_PROBE_BYTES: usize = 256 * 1024;
/// Nombre maximal d'entrées dans le cache d'inspection média en mémoire.
const PROBE_CACHE_MAX_ENTRIES: usize = 1024;
/// Seuils de résolution vidéo (largeur / hauteur).
const UHD_MIN_WIDTH: u32 = 3200;
const UHD_MIN_HEIGHT: u32 = 1800;
const FHD_MIN_WIDTH: u32 = 1700;
const FHD_MIN_HEIGHT: u32 = 900;
const HD_MIN_WIDTH: u32 = 1150;
const HD_MIN_HEIGHT: u32 = 640;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbeInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub quality_tag: String,
    pub video_codec: Option<String>,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub detected_language_profile: String,
    pub has_spanish_audio: bool,
    pub has_spanish_subtitles: bool,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct ProbeCacheKey {
    path: String,
    size: u64,
    mtime_secs: u64,
}

static PROBE_CACHE: OnceLock<Mutex<HashMap<ProbeCacheKey, MediaProbeInfo>>> = OnceLock::new();

fn probe_cache() -> &'static Mutex<HashMap<ProbeCacheKey, MediaProbeInfo>> {
    PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    tags: Option<FfprobeTags>,
}

#[derive(Deserialize)]
struct FfprobeTags {
    language: Option<String>,
    title: Option<String>,
}

/// Inspecte de manière asynchrone (`spawn_blocking`) un fichier vidéo pour déterminer sa résolution réelle,
/// son codec vidéo et ses pistes audio/sous-titres intégrées (avec cache mémoire indexé par mtime/taille).
pub async fn probe_video_file(path: &Path) -> MediaProbeInfo {
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || probe_video_file_sync(&path_buf))
        .await
        .unwrap_or_else(|_| fallback_from_filename(path))
}

/// Version synchrone de l'inspection média (utilisable directement depuis un contexte `spawn_blocking`).
pub fn probe_video_file_sync(path: &Path) -> MediaProbeInfo {
    let meta = std::fs::metadata(path).ok();
    let cache_key = meta.as_ref().map(|m| {
        let mtime_secs = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        ProbeCacheKey {
            path: path.to_string_lossy().to_string(),
            size: m.len(),
            mtime_secs,
        }
    });

    if let Some(ref key) = cache_key {
        if let Ok(guard) = probe_cache().lock() {
            if let Some(cached) = guard.get(key) {
                return cached.clone();
            }
        }
    }

    let info = if path.is_file() {
        probe_via_ffprobe(path)
            .or_else(|| probe_via_binary_header(path))
            .unwrap_or_else(|| fallback_from_filename(path))
    } else {
        fallback_from_filename(path)
    };

    if let Some(key) = cache_key {
        if let Ok(mut guard) = probe_cache().lock() {
            if guard.len() >= PROBE_CACHE_MAX_ENTRIES {
                guard.clear();
            }
            guard.insert(key, info.clone());
        }
    }

    info
}

fn classify_resolution(width: Option<u32>, height: Option<u32>, filename_fallback: &str) -> String {
    match (width, height) {
        (Some(w), Some(h)) if w >= UHD_MIN_WIDTH || h >= UHD_MIN_HEIGHT => "2160p".to_string(),
        (Some(w), Some(h)) if w >= FHD_MIN_WIDTH || h >= FHD_MIN_HEIGHT => "1080p".to_string(),
        (Some(w), Some(h)) if w >= HD_MIN_WIDTH || h >= HD_MIN_HEIGHT => "720p".to_string(),
        (Some(_), Some(_)) => "SD".to_string(),
        _ => crate::services::organizer::detect_quality_tag(filename_fallback).to_string(),
    }
}

fn is_spanish_tag(lang: &str, title: &str) -> bool {
    let l = lang.trim().to_lowercase();
    let t = title.trim().to_lowercase();
    if t.contains("latino") && !t.contains("castellano") && !t.contains("españa") {
        return false;
    }
    matches!(l.as_str(), "spa" | "es" | "esp" | "es-es" | "castellano" | "spanish")
        || t.contains("castellano")
        || t.contains("español")
        || t.contains("espanol")
        || t.contains("spanish")
}

fn is_english_tag(lang: &str, title: &str) -> bool {
    let l = lang.trim().to_lowercase();
    let t = title.trim().to_lowercase();
    matches!(l.as_str(), "eng" | "en" | "en-us" | "en-gb" | "english")
        || t.contains("english")
        || t.contains("inglés")
        || t.contains("ingles")
}

fn build_probe_info(
    width: Option<u32>,
    height: Option<u32>,
    video_codec: Option<String>,
    audio_languages: Vec<String>,
    subtitle_languages: Vec<String>,
    has_es_audio: bool,
    has_en_audio: bool,
    has_es_subs: bool,
    filename_str: &str,
) -> MediaProbeInfo {
    let quality_tag = classify_resolution(width, height, filename_str);
    let lower_fn = filename_str.to_lowercase();

    let fn_has_es = lower_fn.contains("castellano")
        || lower_fn.contains("spanish")
        || lower_fn.contains("español")
        || lower_fn.contains("espanol")
        || lower_fn.contains(".spa.")
        || lower_fn.contains("[spa]");
    let fn_has_dual = lower_fn.contains("dual") || lower_fn.contains("multi");
    let fn_has_vose = lower_fn.contains("vose") || lower_fn.contains("subtitulado");

    let final_es_audio = has_es_audio || fn_has_es || fn_has_dual;
    let final_en_audio = has_en_audio || fn_has_dual || lower_fn.contains("english");
    let final_es_subs = has_es_subs || fn_has_vose;
    let has_foreign_only_audio = !audio_languages.is_empty()
        && !final_es_audio
        && audio_languages
            .iter()
            .all(|l| !matches!(l.as_str(), "und" | "spa" | "es" | "esp"));

    let detected_language_profile = if final_es_audio && final_en_audio {
        "Dual".to_string()
    } else if final_es_audio {
        "Castellano".to_string()
    } else if final_es_subs {
        "VOSE".to_string()
    } else if final_en_audio || has_foreign_only_audio {
        "English".to_string()
    } else {
        "Castellano".to_string()
    };

    MediaProbeInfo {
        width,
        height,
        quality_tag,
        video_codec,
        audio_languages,
        subtitle_languages,
        detected_language_profile,
        has_spanish_audio: final_es_audio,
        has_spanish_subtitles: final_es_subs,
    }
}

fn probe_via_ffprobe(path: &Path) -> Option<MediaProbeInfo> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout).ok()?;
    if parsed.streams.is_empty() {
        return None;
    }

    let mut width = None;
    let mut height = None;
    let mut video_codec = None;
    let mut audio_languages = Vec::new();
    let mut subtitle_languages = Vec::new();
    let mut has_es_audio = false;
    let mut has_en_audio = false;
    let mut has_es_subs = false;

    for stream in parsed.streams {
        let ctype = stream.codec_type.as_deref().unwrap_or("");
        let lang = stream
            .tags
            .as_ref()
            .and_then(|t| t.language.as_deref())
            .unwrap_or("und");
        let title = stream
            .tags
            .as_ref()
            .and_then(|t| t.title.as_deref())
            .unwrap_or("");

        match ctype {
            "video" => {
                if width.is_none() && stream.width.unwrap_or(0) > 0 {
                    width = stream.width;
                    height = stream.height;
                }
                if video_codec.is_none() {
                    video_codec = stream.codec_name;
                }
            }
            "audio" => {
                let norm_lang = lang.to_lowercase();
                if !audio_languages.contains(&norm_lang) {
                    audio_languages.push(norm_lang);
                }
                if is_spanish_tag(lang, title) {
                    has_es_audio = true;
                }
                if is_english_tag(lang, title) {
                    has_en_audio = true;
                }
            }
            "subtitle" => {
                let norm_lang = lang.to_lowercase();
                if !subtitle_languages.contains(&norm_lang) {
                    subtitle_languages.push(norm_lang);
                }
                if is_spanish_tag(lang, title) {
                    has_es_subs = true;
                }
            }
            _ => {}
        }
    }

    let filename_str = path.to_string_lossy();
    Some(build_probe_info(
        width,
        height,
        video_codec,
        audio_languages,
        subtitle_languages,
        has_es_audio,
        has_en_audio,
        has_es_subs,
        &filename_str,
    ))
}

/// Inspecte les premiers 256 Ko d'un conteneur Matroska (`.mkv`) ou MP4 lorsque `ffprobe` est absent.
fn probe_via_binary_header(path: &Path) -> Option<MediaProbeInfo> {
    let mut file = File::open(path).ok()?;
    let mut buf = vec![0u8; HEADER_PROBE_BYTES];
    let n = file.read(&mut buf).ok()?;
    if n < 64 {
        return None;
    }
    buf.truncate(n);

    // Vérifier signature Matroska / EBML (0x1A 0x45 0xDF 0xA3)
    let is_ebml = buf.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
    if !is_ebml {
        return None;
    }

    let mut width = None;
    let mut height = None;
    for i in 0..buf.len().saturating_sub(4) {
        // EBML PixelWidth ID = 0xB0, taille 2 octets (0x82)
        if width.is_none() && buf[i] == 0xB0 && buf[i + 1] == 0x82 {
            let w = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as u32;
            if (320..=7680).contains(&w) {
                width = Some(w);
            }
        }
        // EBML PixelHeight ID = 0xBA, taille 2 octets (0x82)
        if height.is_none() && buf[i] == 0xBA && buf[i + 1] == 0x82 {
            let h = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as u32;
            if (200..=4320).contains(&h) {
                height = Some(h);
            }
        }
    }

    let header_ascii = String::from_utf8_lossy(&buf);
    let video_codec = if header_ascii.contains("V_MPEGH/ISO/HEVC") {
        Some("hevc".to_string())
    } else if header_ascii.contains("V_MPEG4/ISO/AVC") {
        Some("h264".to_string())
    } else if header_ascii.contains("V_AV1") {
        Some("av1".to_string())
    } else {
        None
    };

    let has_es_sub_codec = header_ascii.contains("S_TEXT/UTF8") || header_ascii.contains("S_HDMV/PGS");
    let lower_ascii = header_ascii.to_lowercase();
    let has_es = lower_ascii.contains("castellano") || lower_ascii.contains("spanish") || lower_ascii.contains("spa");
    let has_en = lower_ascii.contains("english") || lower_ascii.contains("eng");

    let mut audio_langs = Vec::new();
    if has_es {
        audio_langs.push("spa".to_string());
    }
    if has_en {
        audio_langs.push("eng".to_string());
    }

    let mut sub_langs = Vec::new();
    if has_es_sub_codec && has_es {
        sub_langs.push("spa".to_string());
    }

    let filename_str = path.to_string_lossy();
    Some(build_probe_info(
        width,
        height,
        video_codec,
        audio_langs,
        sub_langs,
        has_es,
        has_en,
        has_es_sub_codec && has_es,
        &filename_str,
    ))
}

fn fallback_from_filename(path: &Path) -> MediaProbeInfo {
    let filename_str = path.to_string_lossy();
    build_probe_info(
        None,
        None,
        None,
        Vec::new(),
        Vec::new(),
        false,
        false,
        false,
        &filename_str,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_resolution_and_language_fallback() {
        let p = Path::new("/cineseries/Peliculas/Batman (1989)/Batman (1989) [1080p] Dual.mkv");
        let info = fallback_from_filename(p);
        assert_eq!(info.quality_tag, "1080p");
        assert_eq!(info.detected_language_profile, "Dual");
        assert!(info.has_spanish_audio);

        assert_eq!(classify_resolution(Some(3840), Some(1600), "movie.mkv"), "2160p");
        assert_eq!(classify_resolution(Some(1920), Some(800), "movie.mkv"), "1080p");
        assert_eq!(classify_resolution(Some(1280), Some(536), "movie.mkv"), "720p");
        assert_eq!(classify_resolution(Some(720), Some(404), "movie.mkv"), "SD");
    }

    #[test]
    fn test_ebml_binary_header_parsing() {
        let mut fake_mkv = vec![0x1A, 0x45, 0xDF, 0xA3];
        fake_mkv.extend_from_slice(&[0u8; 32]);
        // PixelWidth = 1920 (0x0780)
        fake_mkv.extend_from_slice(&[0xB0, 0x82, 0x07, 0x80]);
        // PixelHeight = 1080 (0x0438)
        fake_mkv.extend_from_slice(&[0xBA, 0x82, 0x04, 0x38]);
        fake_mkv.extend_from_slice(b"V_MPEGH/ISO/HEVC Castellano English S_TEXT/UTF8");
        fake_mkv.extend_from_slice(&[0u8; 64]);

        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(tmp.path(), &fake_mkv).expect("write mkv header");

        let probed = probe_via_binary_header(tmp.path()).expect("ebml parsed");
        assert_eq!(probed.width, Some(1920));
        assert_eq!(probed.height, Some(1080));
        assert_eq!(probed.quality_tag, "1080p");
        assert_eq!(probed.video_codec.as_deref(), Some("hevc"));
        assert_eq!(probed.detected_language_profile, "Dual");
        assert!(probed.has_spanish_audio);
        assert!(probed.has_spanish_subtitles);
    }
}

