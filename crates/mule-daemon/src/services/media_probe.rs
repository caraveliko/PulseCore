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

/// Tamaño máximo de la cabecera binaria inspeccionada como respaldo sin `ffprobe` (256 KiB).
const HEADER_PROBE_BYTES: usize = 256 * 1024;
/// Número máximo de entradas en la caché de inspección de medios en memoria.
const PROBE_CACHE_MAX_ENTRIES: usize = 1024;
/// Límite máximo de sondas de vídeo concurrentes para no saturar CPU/NFS en Raspberry Pi.
const MAX_CONCURRENT_MEDIA_PROBES: usize = 4;
/// Umbrales de resolución de vídeo (ancho / alto).
const UHD_MIN_WIDTH: u32 = 3200;
const UHD_MIN_HEIGHT: u32 = 1800;
const FHD_MIN_WIDTH: u32 = 1700;
const FHD_MIN_HEIGHT: u32 = 900;
const HD_MIN_WIDTH: u32 = 1150;
const HD_MIN_HEIGHT: u32 = 640;

/// Multiplicador para convertir FPS a milésimas enteras (p. ej., 23.976 -> 23976).
const FPS_MILLI_SCALE: f64 = 1000.0;
/// Rango razonable de FPS válidos en milésimas (de 10 fps a 240 fps).
const MIN_VALID_FPS_MILLI: u32 = 10_000;
const MAX_VALID_FPS_MILLI: u32 = 240_000;

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
    #[serde(default)]
    pub fps_milli: Option<u32>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    #[serde(default)]
    pub embedded_text_sub_stream: Option<usize>,
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
    #[serde(default)]
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
    tags: Option<FfprobeTags>,
    disposition: Option<FfprobeDisposition>,
}

#[derive(Deserialize)]
struct FfprobeDisposition {
    forced: Option<i32>,
    hearing_impaired: Option<i32>,
}

#[derive(Deserialize)]
struct FfprobeTags {
    language: Option<String>,
    title: Option<String>,
}

/// Inspecciona de manera asíncrona (`spawn_blocking`) un archivo de vídeo para determinar su resolución real,
/// su códec de vídeo y sus pistas de audio/subtítulos integradas (con caché en memoria indexada por mtime/tamaño).
pub async fn probe_video_file(path: &Path) -> MediaProbeInfo {
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || probe_video_file_sync(&path_buf))
        .await
        .unwrap_or_else(|_| fallback_from_filename(path))
}

/// Inspecciona un lote de archivos de vídeo limitando la concurrencia mediante un semáforo
/// para evitar saturar el pool bloqueante o el montaje NFS en series con muchos episodios.
pub async fn probe_video_files_bounded(
    targets: Vec<(String, std::path::PathBuf)>,
) -> HashMap<String, MediaProbeInfo> {
    if targets.is_empty() {
        return HashMap::new();
    }
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_MEDIA_PROBES));
    let futs = targets.into_iter().map(|(id, path)| {
        let sem = std::sync::Arc::clone(&semaphore);
        async move {
            let _permit = sem.acquire_owned().await.ok();
            let info = probe_video_file(&path).await;
            (id, info)
        }
    });
    futures_util::future::join_all(futs).await.into_iter().collect()
}

/// Versión síncrona de la inspección multimedia (utilizable directamente desde un contexto `spawn_blocking`).
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

/// Convierte una fracción de `ffprobe` (p. ej. `"24000/1001"` o `"25/1"`) a milésimas de FPS.
pub fn parse_fps_milli(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let val = if let Some((num_s, den_s)) = trimmed.split_once('/') {
        let num: f64 = num_s.trim().parse().ok()?;
        let den: f64 = den_s.trim().parse().ok()?;
        if den <= 0.0 || num <= 0.0 {
            return None;
        }
        num / den
    } else {
        let f: f64 = trimmed.parse().ok()?;
        if f <= 0.0 {
            return None;
        }
        f
    };

    let milli = (val * FPS_MILLI_SCALE).round();
    if !milli.is_finite() || milli < 0.0 {
        return None;
    }
    let milli_u32 = milli as u32;
    if (MIN_VALID_FPS_MILLI..=MAX_VALID_FPS_MILLI).contains(&milli_u32) {
        Some(milli_u32)
    } else {
        None
    }
}

fn is_extractable_text_sub_codec(codec: &str) -> bool {
    matches!(
        codec.trim().to_lowercase().as_str(),
        "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text"
    )
}

#[allow(clippy::too_many_arguments)]
fn build_probe_info(
    width: Option<u32>,
    height: Option<u32>,
    video_codec: Option<String>,
    audio_languages: Vec<String>,
    subtitle_languages: Vec<String>,
    has_es_audio: bool,
    has_en_audio: bool,
    has_es_subs: bool,
    fps_milli: Option<u32>,
    duration_secs: Option<u64>,
    embedded_text_sub_stream: Option<usize>,
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
        fps_milli,
        duration_secs,
        embedded_text_sub_stream,
    }
}

fn probe_via_ffprobe(path: &Path) -> Option<MediaProbeInfo> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
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
    let mut fps_milli = None;
    let mut duration_secs = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(|d| d.trim().parse::<f64>().ok())
        .filter(|&d| d.is_finite() && d > 0.0)
        .map(|d| d.round() as u64);
    let mut audio_languages = Vec::new();
    let mut subtitle_languages = Vec::new();
    let mut has_es_audio = false;
    let mut has_en_audio = false;
    let mut has_es_subs = false;
    let mut sub_stream_rel_idx: usize = 0;
    let mut best_embedded_sub: Option<(u8, usize)> = None;

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
                if fps_milli.is_none() {
                    fps_milli = stream
                        .avg_frame_rate
                        .as_deref()
                        .and_then(parse_fps_milli)
                        .or_else(|| stream.r_frame_rate.as_deref().and_then(parse_fps_milli));
                }
                if duration_secs.is_none() {
                    duration_secs = stream
                        .duration
                        .as_deref()
                        .and_then(|d| d.trim().parse::<f64>().ok())
                        .filter(|&d| d.is_finite() && d > 0.0)
                        .map(|d| d.round() as u64);
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
                let codec = stream.codec_name.as_deref().unwrap_or("");
                if is_extractable_text_sub_codec(codec) {
                    let is_forced = stream
                        .disposition
                        .as_ref()
                        .and_then(|d| d.forced)
                        .unwrap_or(0)
                        != 0
                        || title.to_lowercase().contains("forced")
                        || title.to_lowercase().contains("forzados");
                    let is_hi = stream
                        .disposition
                        .as_ref()
                        .and_then(|d| d.hearing_impaired)
                        .unwrap_or(0)
                        != 0;
                    if !is_forced {
                        let priority = if is_english_tag(lang, title) && !is_hi {
                            4
                        } else if is_spanish_tag(lang, title) && !is_hi {
                            3
                        } else if is_english_tag(lang, title) {
                            2
                        } else {
                            1
                        };
                        if best_embedded_sub.is_none_or(|(p, _)| priority > p) {
                            best_embedded_sub = Some((priority, sub_stream_rel_idx));
                        }
                    }
                }
                sub_stream_rel_idx += 1;
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
        fps_milli,
        duration_secs,
        best_embedded_sub.map(|(_, idx)| idx),
        &filename_str,
    ))
}

/// Inspecciona los primeros 256 KiB de un contenedor Matroska (`.mkv`) o MP4 cuando `ffprobe` no está presente.
fn probe_via_binary_header(path: &Path) -> Option<MediaProbeInfo> {
    let mut file = File::open(path).ok()?;
    let mut buf = vec![0u8; HEADER_PROBE_BYTES];
    let n = file.read(&mut buf).ok()?;
    if n < 64 {
        return None;
    }
    buf.truncate(n);

    // Verificar firma Matroska / EBML (0x1A 0x45 0xDF 0xA3)
    let is_ebml = buf.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
    if !is_ebml {
        return None;
    }

    let mut width = None;
    let mut height = None;
    for i in 0..buf.len().saturating_sub(4) {
        // ID EBML PixelWidth = 0xB0, tamaño 2 bytes (0x82)
        if width.is_none() && buf[i] == 0xB0 && buf[i + 1] == 0x82 {
            let w = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as u32;
            if (320..=7680).contains(&w) {
                width = Some(w);
            }
        }
        // ID EBML PixelHeight = 0xBA, tamaño 2 bytes (0x82)
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
        None,
        None,
        None,
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
        None,
        None,
        None,
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

    #[test]
    fn test_parse_fps_milli() {
        assert_eq!(parse_fps_milli("24000/1001"), Some(23_976));
        assert_eq!(parse_fps_milli("25/1"), Some(25_000));
        assert_eq!(parse_fps_milli("23.976"), Some(23_976));
        assert_eq!(parse_fps_milli("0/0"), None);
        assert_eq!(parse_fps_milli(""), None);
    }

    #[tokio::test]
    async fn test_probe_video_files_bounded_returns_all_entries() {
        let targets: Vec<(String, std::path::PathBuf)> = (1..=6)
            .map(|idx| {
                (
                    format!("ep-{idx}"),
                    std::path::PathBuf::from(format!("/nonexistent/Serie.S01E{idx:02}.1080p.Castellano.mkv")),
                )
            })
            .collect();
        let res = probe_video_files_bounded(targets).await;
        assert_eq!(res.len(), 6);
        assert_eq!(res["ep-1"].quality_tag, "1080p");
        assert!(res["ep-6"].has_spanish_audio);
    }
}

