// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Fachada del servicio de subtítulos: búsqueda puntuada, validación SRT, lista negra y sincronización.

pub mod opensubtitles;
pub mod scorer;
pub mod srt;
pub mod sync;

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::services::downloads::types::now_ms;
use crate::services::media_probe::{probe_video_file, MediaProbeInfo};
use crate::state::AppState;

use self::opensubtitles::{search_opensubtitles_combined, subtitle_http_client, RawSubtitleItem};
use self::scorer::{score_candidate, ScorableCandidate, SubtitlePreferences, SubtitleTargetContext};
use self::srt::{format_srt, parse_srt, sanitize_cues, validate_srt_cues};
use self::sync::{align_cues_to_reference, extract_embedded_srt_via_ffmpeg, rescale_cues_fps, shift_cues_ms};

pub use self::srt::decode_subtitle_bytes;

/// Tamaño mínimo de archivo requerido para el hash de 64 bits de OpenSubtitles (128 KiB).
const MIN_OPENSUBTITLES_HASH_FILE_SIZE: u64 = 131_072;
/// Número de palabras de 64 bits leídas al inicio y al final del archivo (64 KiB = 8192 * 8 bytes).
const OPENSUBTITLES_CHUNK_WORDS: usize = 8192;
/// Desplazamiento final para los últimos 64 KiB.
const OPENSUBTITLES_END_OFFSET: i64 = -65_536;
/// Puntuación mínima por defecto para aceptar una descarga automática.
const DEFAULT_MIN_SUBTITLE_SCORE: i32 = 40;

/// Candidato de subtítulo enriquecido con puntuación, variante y metadatos técnicos.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCandidate {
    pub id: String,
    pub provider: String,
    pub title: String,
    pub release_name: Option<String>,
    pub language: String,
    pub format: String,
    pub download_link: String,
    pub downloads: u64,
    pub rating: String,
    pub matched_by: String,
    pub is_hi: bool,
    pub is_trusted: bool,
    pub is_auto_translated: bool,
    pub fps_milli: Option<u32>,
    pub variant: Option<String>,
    pub score: i32,
    pub score_reasons: Vec<String>,
}

/// Modo de sincronización automática configurado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSyncMode {
    Off,
    FpsOnly,
    EmbeddedSub,
}

impl AutoSyncMode {
    pub fn from_setting(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "off" | "disabled" | "none" => Self::Off,
            "fps_only" | "fps" => Self::FpsOnly,
            _ => Self::EmbeddedSub,
        }
    }
}

/// Opciones efectivas de subtítulos leídas desde SQLite.
#[derive(Debug, Clone)]
pub struct SubtitleRuntimeConfig {
    pub enabled: bool,
    pub language: String,
    pub prefs: SubtitlePreferences,
    pub auto_sync: AutoSyncMode,
}

impl Default for SubtitleRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            language: "spa".to_string(),
            prefs: SubtitlePreferences::default(),
            auto_sync: AutoSyncMode::EmbeddedSub,
        }
    }
}

/// Calcula el hash estándar de 64 bits de OpenSubtitles (primeros 64 KiB + últimos 64 KiB + tamaño de archivo).
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

    for _ in 0..OPENSUBTITLES_CHUNK_WORDS {
        file.read_exact(&mut buf)?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    file.seek(SeekFrom::End(OPENSUBTITLES_END_OFFSET))?;
    for _ in 0..OPENSUBTITLES_CHUNK_WORDS {
        file.read_exact(&mut buf)?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    Ok((format!("{hash:016x}"), file_size))
}

/// Versión asíncrona no bloqueante (`spawn_blocking`) del cálculo de hash de OpenSubtitles.
pub async fn compute_opensubtitles_hash_async(path: &Path) -> std::io::Result<(String, u64)> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || compute_opensubtitles_hash(&p))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
}

/// Normaliza un código de idioma a formato corto para el nombre de archivo acompañante (`.es.srt`).
pub fn normalize_companion_lang(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "spa" | "spanish" | "castellano" | "es-es" | "es-la" | "es" => "es".to_string(),
        "eng" | "english" | "en" => "en".to_string(),
        other => other.to_string(),
    }
}

/// Genera una clave estable para la lista negra de subtítulos de un elemento o episodio.
pub fn build_media_blacklist_key(media_id: &str, season_ep: Option<(u32, u32)>) -> String {
    match season_ep {
        Some((s, e)) => format!("{media_id}:S{s:02}E{e:02}"),
        None => media_id.to_string(),
    }
}

/// Servicio principal de gestión, descarga, validación y sincronización de subtítulos.
pub struct SubtitleService;

impl SubtitleService {
    /// Lee la configuración vigente de subtítulos desde SQLite.
    pub async fn load_runtime_config(state: &AppState) -> SubtitleRuntimeConfig {
        let dl_svc = state.downloads.lock().await;
        let enabled = dl_svc
            .get_setting("subtitles_auto_download")
            .ok()
            .flatten()
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);
        let language = dl_svc
            .get_setting("subtitles_language")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "spa".to_string());
        let variant = dl_svc
            .get_setting("subtitles_variant")
            .ok()
            .flatten()
            .filter(|s| matches!(s.as_str(), "es-ES" | "es-LA" | "any"))
            .unwrap_or_else(|| "es-ES".to_string());
        let allow_hi = dl_svc
            .get_setting("subtitles_allow_hi")
            .ok()
            .flatten()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let min_score = dl_svc
            .get_setting("subtitles_min_score")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(DEFAULT_MIN_SUBTITLE_SCORE);
        let auto_sync_raw = dl_svc
            .get_setting("subtitles_auto_sync")
            .ok()
            .flatten()
            .unwrap_or_else(|| "embedded_sub".to_string());

        SubtitleRuntimeConfig {
            enabled,
            language,
            prefs: SubtitlePreferences {
                variant,
                allow_hi,
                min_score,
            },
            auto_sync: AutoSyncMode::from_setting(&auto_sync_raw),
        }
    }

    /// Detecta qué archivos de subtítulos complementarios ya existen junto al archivo de vídeo.
    pub fn detect_existing_subtitles(video_path: &Path) -> Vec<String> {
        let Some(parent) = video_path.parent() else {
            return Vec::new();
        };
        let Some(stem) = video_path.file_stem().and_then(|s| s.to_str()) else {
            return Vec::new();
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
                            langs.push("es".to_string());
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

    /// Busca y puntúa candidatos de subtítulos en OpenSubtitles, filtrando descartes y lista negra.
    #[allow(clippy::too_many_arguments)]
    pub async fn search_subtitles_scored(
        video_path: Option<&Path>,
        release_name: Option<&str>,
        title: &str,
        year: Option<u32>,
        imdb_id: Option<&str>,
        is_movie: bool,
        season_ep: Option<(u32, u32)>,
        lang: &str,
        prefs: &SubtitlePreferences,
        blacklisted: &HashSet<(String, String)>,
        probe_opt: Option<&MediaProbeInfo>,
    ) -> Result<Vec<SubtitleCandidate>, String> {
        let raw_items = search_opensubtitles_combined(
            video_path,
            title,
            year,
            imdb_id,
            is_movie,
            season_ep,
            lang,
        )
        .await;

        let fps_milli = match probe_opt {
            Some(p) => p.fps_milli,
            None => match video_path {
                Some(vp) if vp.is_file() => probe_video_file(vp).await.fps_milli,
                _ => None,
            },
        };

        let effective_release = release_name.map(str::to_string).or_else(|| {
            video_path
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(str::to_string)
        });

        let target_ctx = SubtitleTargetContext {
            release_name: effective_release,
            fps_milli,
            season_ep,
        };

        let mut scored: Vec<SubtitleCandidate> = raw_items
            .into_iter()
            .filter(|raw| !blacklisted.contains(&(raw.provider.clone(), raw.id.clone())))
            .filter_map(|raw| Self::score_raw_item(raw, &target_ctx, prefs))
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| (b.matched_by == "moviehash").cmp(&(a.matched_by == "moviehash")))
                .then_with(|| b.downloads.cmp(&a.downloads))
        });

        Ok(scored)
    }

    fn score_raw_item(
        raw: RawSubtitleItem,
        target_ctx: &SubtitleTargetContext,
        prefs: &SubtitlePreferences,
    ) -> Option<SubtitleCandidate> {
        let scorable = ScorableCandidate {
            title: &raw.title,
            release_name: raw.release_name.as_deref(),
            matched_by: &raw.matched_by,
            downloads: raw.downloads,
            is_hi: raw.is_hi,
            is_bad: raw.is_bad,
            is_trusted: raw.is_trusted,
            is_auto_translated: raw.is_auto_translated,
            fps_milli: raw.fps_milli,
            season: raw.season,
            episode: raw.episode,
            variant_hint: raw.variant.as_deref(),
        };
        let eval = score_candidate(&scorable, target_ctx, prefs);
        if !eval.accepted {
            return None;
        }
        Some(SubtitleCandidate {
            id: raw.id,
            provider: raw.provider,
            title: raw.title,
            release_name: raw.release_name,
            language: raw.language,
            format: raw.format,
            download_link: raw.download_link,
            downloads: raw.downloads,
            rating: raw.rating,
            matched_by: raw.matched_by,
            is_hi: raw.is_hi,
            is_trusted: raw.is_trusted,
            is_auto_translated: raw.is_auto_translated,
            fps_milli: raw.fps_milli,
            variant: raw.variant,
            score: eval.score,
            score_reasons: eval.reasons,
        })
    }

    /// Compatibilidad con la API de búsqueda directa con preferencias por defecto.
    pub async fn search_subtitles(
        video_path: Option<&Path>,
        title: &str,
        year: Option<u32>,
        imdb_id: Option<&str>,
        is_movie: bool,
        season_ep: Option<(u32, u32)>,
        lang: &str,
    ) -> Result<Vec<SubtitleCandidate>, String> {
        Self::search_subtitles_scored(
            video_path,
            None,
            title,
            year,
            imdb_id,
            is_movie,
            season_ep,
            lang,
            &SubtitlePreferences::default(),
            &HashSet::new(),
            None,
        )
        .await
    }

    /// Descarga los bytes de un subtítulo, descomprime gzip si es necesario y decodifica a UTF-8.
    async fn fetch_and_decode_subtitle(download_url: &str) -> Result<String, String> {
        let client = subtitle_http_client();
        let resp = client.get(download_url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {} descargando subtítulo", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;

        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let raw_data = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
                let mut gz = GzDecoder::new(&bytes[..]);
                let mut out = Vec::new();
                gz.read_to_end(&mut out)
                    .map_err(|e| format!("Fallo descomprimiendo gzip: {e}"))?;
                out
            } else {
                bytes.to_vec()
            };
            Ok(decode_subtitle_bytes(&raw_data))
        })
        .await
        .map_err(|e| format!("Error en tarea de decodificación: {e}"))?
    }

    /// Descarga, valida, sanea, sincroniza opcionalmente y guarda un subtítulo SRT.
    #[allow(clippy::too_many_arguments)]
    pub async fn download_validate_and_sync(
        download_url: &str,
        target_srt_path: &Path,
        video_path: Option<&Path>,
        expected_lang: &str,
        cand_fps_milli: Option<u32>,
        is_exact_hash: bool,
        strip_hi: bool,
        auto_sync: AutoSyncMode,
        probe_opt: Option<&MediaProbeInfo>,
    ) -> Result<Value, String> {
        let text = Self::fetch_and_decode_subtitle(download_url).await?;
        let mut cues = parse_srt(&text)?;
        sanitize_cues(&mut cues, strip_hi);

        let owned_probe = match probe_opt {
            Some(p) => Some(p.clone()),
            None => match video_path {
                Some(vp) if vp.is_file() => Some(probe_video_file(vp).await),
                _ => None,
            },
        };
        let duration_secs = owned_probe.as_ref().and_then(|p| p.duration_secs);
        let report = validate_srt_cues(&cues, duration_secs, expected_lang)?;

        let mut fps_rescaled = false;
        let mut aligned_offset_ms: i64 = 0;

        if !is_exact_hash && auto_sync != AutoSyncMode::Off {
            if let (Some(sub_fps), Some(vid_fps)) = (
                cand_fps_milli,
                owned_probe.as_ref().and_then(|p| p.fps_milli),
            ) {
                if sub_fps.abs_diff(vid_fps) > 100 {
                    rescale_cues_fps(&mut cues, sub_fps, vid_fps);
                    fps_rescaled = true;
                    info!(
                        "[SUBTITLES] Reescalado FPS aplicado: {sub_fps} -> {vid_fps} en '{}'",
                        target_srt_path.display()
                    );
                }
            }

            if auto_sync == AutoSyncMode::EmbeddedSub {
                if let (Some(vp), Some(sub_idx)) = (
                    video_path,
                    owned_probe.as_ref().and_then(|p| p.embedded_text_sub_stream),
                ) {
                    if let Some(ref_cues) = extract_embedded_srt_via_ffmpeg(vp, sub_idx).await {
                        let align_res = align_cues_to_reference(&mut cues, &ref_cues);
                        if align_res.applied {
                            aligned_offset_ms = align_res.offset_ms;
                            info!(
                                "[SUBTITLES] Alineación sub-a-sub aplicada ({} ms, coincidencias {} -> {}) en '{}'",
                                align_res.offset_ms,
                                align_res.matched_before,
                                align_res.matched_after,
                                target_srt_path.display()
                            );
                        }
                    }
                }
            }
        }

        let formatted = format_srt(&cues);
        let target_buf = target_srt_path.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            if let Some(parent) = target_buf.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Error creando directorio de subtítulos: {e}"))?;
            }
            std::fs::write(&target_buf, formatted.as_bytes())
                .map_err(|e| format!("Error escribiendo subtítulo: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("Error en tarea de escritura: {e}"))??;

        info!(
            "[SUBTITLES] Subtítulo guardado y validado ({} bloques, lang={}): '{}'",
            report.cue_count,
            report.detected_lang,
            target_srt_path.display()
        );

        Ok(json!({
            "cueCount": report.cue_count,
            "detectedLang": report.detected_lang,
            "fpsRescaled": fps_rescaled,
            "alignedOffsetMs": aligned_offset_ms
        }))
    }

    /// Descarga y guarda un subtítulo validándolo en UTF-8 (método de compatibilidad).
    pub async fn download_and_save_subtitle(
        download_url: &str,
        target_srt_path: &Path,
    ) -> Result<(), String> {
        let _ = Self::download_validate_and_sync(
            download_url,
            target_srt_path,
            None,
            "spa",
            None,
            false,
            false,
            AutoSyncMode::Off,
            None,
        )
        .await?;
        Ok(())
    }

    /// Sincroniza un archivo `.srt` ya existente en disco (automático contra pista embebida o manual).
    pub async fn sync_existing_subtitle(
        srt_path: &Path,
        video_path: Option<&Path>,
        mode: &str,
        offset_ms: Option<i64>,
        from_fps_milli: Option<u32>,
        to_fps_milli: Option<u32>,
    ) -> Result<Value, String> {
        let srt_buf = srt_path.to_path_buf();
        let raw_bytes = tokio::task::spawn_blocking(move || std::fs::read(&srt_buf))
            .await
            .map_err(|e| format!("Error leyendo SRT: {e}"))?
            .map_err(|e| format!("No se pudo leer '{}': {e}", srt_path.display()))?;

        let decoded = decode_subtitle_bytes(&raw_bytes);
        let mut cues = parse_srt(&decoded)?;
        let mut applied_offset_ms: i64 = 0;
        let mut fps_rescaled = false;

        if mode == "manual" {
            if let (Some(from_f), Some(to_f)) = (from_fps_milli, to_fps_milli) {
                rescale_cues_fps(&mut cues, from_f, to_f);
                fps_rescaled = true;
            }
            if let Some(off) = offset_ms {
                shift_cues_ms(&mut cues, off);
                applied_offset_ms = off;
            }
        } else if let Some(vp) = video_path {
            let probe = probe_video_file(vp).await;
            if let Some(sub_idx) = probe.embedded_text_sub_stream {
                if let Some(ref_cues) = extract_embedded_srt_via_ffmpeg(vp, sub_idx).await {
                    let res = align_cues_to_reference(&mut cues, &ref_cues);
                    if res.applied {
                        applied_offset_ms = res.offset_ms;
                    }
                }
            }
        }

        let formatted = format_srt(&cues);
        let srt_write = srt_path.to_path_buf();
        tokio::task::spawn_blocking(move || std::fs::write(&srt_write, formatted.as_bytes()))
            .await
            .map_err(|e| format!("Error guardando SRT sincronizado: {e}"))?
            .map_err(|e| format!("No se pudo escribir '{}': {e}", srt_path.display()))?;

        Ok(json!({
            "success": true,
            "path": srt_path.to_string_lossy(),
            "appliedOffsetMs": applied_offset_ms,
            "fpsRescaled": fps_rescaled,
            "cueCount": cues.len()
        }))
    }

    /// Obtiene automáticamente el subtítulo complementario para un vídeo con configuración explícita.
    #[allow(clippy::too_many_arguments)]
    pub async fn auto_fetch_for_video_with_config(
        video_path: &Path,
        release_name: Option<&str>,
        title: &str,
        year: Option<u32>,
        imdb_id: Option<&str>,
        is_movie: bool,
        season_ep: Option<(u32, u32)>,
        cfg: &SubtitleRuntimeConfig,
        blacklisted: &HashSet<(String, String)>,
    ) -> Result<Option<PathBuf>, String> {
        let existing = Self::detect_existing_subtitles_async(video_path).await;
        let normalized_lang = normalize_companion_lang(&cfg.language);

        if existing.contains(&normalized_lang) {
            return Ok(None);
        }

        let probe = probe_video_file(video_path).await;
        if normalized_lang == "es" && probe.has_spanish_subtitles && probe.has_spanish_audio {
            info!(
                "[SUBTITLES] '{}' ya incorpora audio y subtítulos en español; se omite descarga externa.",
                video_path.display()
            );
            return Ok(None);
        }

        let candidates = Self::search_subtitles_scored(
            Some(video_path),
            release_name,
            title,
            year,
            imdb_id,
            is_movie,
            season_ep,
            &cfg.language,
            &cfg.prefs,
            blacklisted,
            Some(&probe),
        )
        .await?;

        let viable: Vec<SubtitleCandidate> = candidates
            .into_iter()
            .filter(|c| c.score >= cfg.prefs.min_score)
            .collect();

        if viable.is_empty() {
            info!(
                "[SUBTITLES] Sin candidatos con puntuación >= {} para '{}'",
                cfg.prefs.min_score, title
            );
            return Ok(None);
        }

        let target_srt = video_path.with_extension(format!("{normalized_lang}.srt"));
        let strip_hi = !cfg.prefs.allow_hi;

        for cand in viable.into_iter().take(4) {
            match Self::download_validate_and_sync(
                &cand.download_link,
                &target_srt,
                Some(video_path),
                &cfg.language,
                cand.fps_milli,
                cand.matched_by == "moviehash",
                strip_hi,
                cfg.auto_sync,
                Some(&probe),
            )
            .await
            {
                Ok(_) => return Ok(Some(target_srt)),
                Err(err) => {
                    warn!(
                        "[SUBTITLES] Candidato '{}' ({}) descartado tras validar: {err}",
                        cand.title, cand.id
                    );
                }
            }
        }

        Ok(None)
    }

    /// Obtiene automáticamente el subtítulo complementario para un archivo de vídeo recién organizado.
    pub async fn auto_fetch_for_video(
        video_path: &Path,
        title: &str,
        year: Option<u32>,
        imdb_id: Option<&str>,
        is_movie: bool,
        season_ep: Option<(u32, u32)>,
        lang: &str,
    ) -> Result<Option<PathBuf>, String> {
        let cfg = SubtitleRuntimeConfig {
            language: lang.to_string(),
            ..Default::default()
        };
        Self::auto_fetch_for_video_with_config(
            video_path,
            None,
            title,
            year,
            imdb_id,
            is_movie,
            season_ep,
            &cfg,
            &HashSet::new(),
        )
        .await
    }

    /// Barre toda la mediateca (películas + episodios de TV) y obtiene automáticamente los subtítulos faltantes.
    pub async fn auto_fetch_all_missing(state: &AppState, lang_override: &str) -> Value {
        let mut cfg = Self::load_runtime_config(state).await;
        if !lang_override.trim().is_empty() {
            cfg.language = lang_override.to_string();
        }
        let mut downloaded = Vec::new();

        let (movies, completed_episodes) = {
            let dl_svc = state.downloads.lock().await;
            let db = dl_svc.db();
            let movies = db
                .list_media_items(Some("movie"), Some("completed"))
                .unwrap_or_default();
            let shows = db.list_media_items(Some("tv"), None).unwrap_or_default();
            let mut ep_targets = Vec::new();
            for show in shows {
                if let Ok(eps) = db.get_episodes_for_media(&show.id) {
                    for ep in eps {
                        if ep.status == "completed" {
                            if let Some(fp) = ep.file_path {
                                if !fp.is_empty() {
                                    ep_targets.push((
                                        show.id.clone(),
                                        show.title.clone(),
                                        show.year,
                                        show.imdb_id.clone(),
                                        ep.season_number,
                                        ep.episode_number,
                                        fp,
                                        ep.original_release_name,
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
                    let bl = {
                        let dl_svc = state.downloads.lock().await;
                        dl_svc
                            .db()
                            .list_subtitle_blacklist(&m.id)
                            .unwrap_or_default()
                            .into_iter()
                            .collect::<HashSet<_>>()
                    };
                    if let Ok(Some(sub_file)) = Self::auto_fetch_for_video_with_config(
                        p,
                        m.original_release_name.as_deref(),
                        &m.title,
                        m.year,
                        m.imdb_id.as_deref(),
                        true,
                        None,
                        &cfg,
                        &bl,
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

        for (show_id, show_title, show_year, show_imdb, s_num, e_num, fp, rel_name) in completed_episodes {
            let p = Path::new(&fp);
            if p.is_file() {
                let key = build_media_blacklist_key(&show_id, Some((s_num, e_num)));
                let bl = {
                    let dl_svc = state.downloads.lock().await;
                    dl_svc
                        .db()
                        .list_subtitle_blacklist(&key)
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<HashSet<_>>()
                };
                if let Ok(Some(sub_file)) = Self::auto_fetch_for_video_with_config(
                    p,
                    rel_name.as_deref(),
                    &show_title,
                    show_year,
                    show_imdb.as_deref(),
                    false,
                    Some((s_num, e_num)),
                    &cfg,
                    &bl,
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

        let _ = now_ms();
        json!({
            "downloaded": downloaded
        })
    }
}
