// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Análisis de nombres de *release* y puntuación determinista de candidatos de subtítulos.

use serde::{Deserialize, Serialize};

/// Bonificación por coincidencia exacta de `moviehash` + `moviebytesize`.
const SCORE_MOVIEHASH_EXACT: i32 = 100;
/// Bonificación por coincidencia exacta del grupo de *release* (`FLUX`, `NTb`, etc.).
const SCORE_RELEASE_GROUP: i32 = 40;
/// Bonificación por coincidencia de fuente (`WEB-DL`, `BluRay`, `HDTV`, etc.).
const SCORE_SOURCE_MATCH: i32 = 30;
/// Bonificación por coincidencia de servicio de *streaming* (`AMZN`, `NF`, `DSNP`, etc.).
const SCORE_SERVICE_MATCH: i32 = 15;
/// Bonificación por coincidencia de edición (`Extended`, `Director's Cut`, `REPACK`).
const SCORE_EDITION_MATCH: i32 = 25;
/// Penalización fuerte si uno de los dos indica una edición distinta (`Extended` vs normal).
const PENALTY_EDITION_MISMATCH: i32 = -50;
/// Bonificación por coincidencia de FPS dentro de tolerancia.
const SCORE_FPS_MATCH: i32 = 20;
/// Penalización por diferencia significativa de FPS (p. ej. 25 fps vs 23.976 fps).
const PENALTY_FPS_MISMATCH: i32 = -25;
/// Bonificación por coincidencia de resolución o códec.
const SCORE_TECH_MATCH: i32 = 10;
/// Bonificación por coincidencia con la variante de idioma preferida (`es-ES` / `es-LA`).
const SCORE_VARIANT_MATCH: i32 = 20;
/// Bonificación por subtítulo verificado o de usuario de confianza.
const SCORE_TRUSTED: i32 = 10;
/// Penalización por traducción automática o por IA.
const PENALTY_AUTO_TRANSLATED: i32 = -30;
/// Bonificación máxima por popularidad (descargas).
const SCORE_DOWNLOADS_CAP: i32 = 10;
/// Tolerancia de comparación de FPS en milésimas (0,100 fps).
const FPS_TOLERANCE_MILLI: u32 = 100;

/// Metadatos extraídos de un nombre de archivo o *release*.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseMetadata {
    pub source: Option<String>,
    pub streaming_service: Option<String>,
    pub group: Option<String>,
    pub resolution: Option<String>,
    pub codec: Option<String>,
    pub edition: Option<String>,
    pub variant: Option<String>,
}

/// Contexto del archivo de vídeo objetivo para evaluar candidatos.
#[derive(Debug, Clone, Default)]
pub struct SubtitleTargetContext {
    pub release_name: Option<String>,
    pub fps_milli: Option<u32>,
    pub season_ep: Option<(u32, u32)>,
}

/// Preferencias de selección del usuario.
#[derive(Debug, Clone)]
pub struct SubtitlePreferences {
    pub variant: String,
    pub allow_hi: bool,
    pub min_score: i32,
}

impl Default for SubtitlePreferences {
    fn default() -> Self {
        Self {
            variant: "es-ES".to_string(),
            allow_hi: false,
            min_score: 40,
        }
    }
}

/// Resultado de la evaluación de un candidato.
#[derive(Debug, Clone)]
pub struct CandidateScore {
    pub accepted: bool,
    pub score: i32,
    pub reasons: Vec<String>,
}

/// Datos mínimos necesarios de un candidato para puntuarlo.
#[derive(Debug, Clone)]
pub struct ScorableCandidate<'a> {
    pub title: &'a str,
    pub release_name: Option<&'a str>,
    pub matched_by: &'a str,
    pub downloads: u64,
    pub is_hi: bool,
    pub is_bad: bool,
    pub is_trusted: bool,
    pub is_auto_translated: bool,
    pub fps_milli: Option<u32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub variant_hint: Option<&'a str>,
}

/// Extrae metadatos técnicos y de *release* a partir de un nombre de archivo o título.
pub fn parse_release_metadata(raw: &str) -> ReleaseMetadata {
    let lower = raw.to_lowercase();
    let without_ext = strip_known_extension(&lower);

    let source = if without_ext.contains("remux") {
        Some("remux".to_string())
    } else if without_ext.contains("bluray") || without_ext.contains("blu-ray") || without_ext.contains("bdrip") || without_ext.contains("brrip") {
        Some("bluray".to_string())
    } else if without_ext.contains("web-dl") || without_ext.contains("webdl") {
        Some("web-dl".to_string())
    } else if without_ext.contains("webrip") {
        Some("webrip".to_string())
    } else if without_ext.contains("hdtv") {
        Some("hdtv".to_string())
    } else if without_ext.contains("dvdrip") {
        Some("dvdrip".to_string())
    } else {
        None
    };

    let streaming_service = ["amzn", "nf", "netflix", "dsnp", "atvp", "hmax", "hbo", "hulu", "pcok"]
        .iter()
        .find(|&&svc| has_token(without_ext, svc))
        .map(|&svc| match svc {
            "netflix" => "nf".to_string(),
            _ => svc.to_string(),
        });

    let resolution = ["2160p", "1080p", "720p", "480p"]
        .iter()
        .find(|&&r| without_ext.contains(r))
        .map(|s| (*s).to_string());

    let codec = if without_ext.contains("x265") || without_ext.contains("h265") || without_ext.contains("hevc") {
        Some("hevc".to_string())
    } else if without_ext.contains("x264") || without_ext.contains("h264") || without_ext.contains("avc") {
        Some("h264".to_string())
    } else {
        None
    };

    let edition = if without_ext.contains("extended") {
        Some("extended".to_string())
    } else if without_ext.contains("director") {
        Some("directors_cut".to_string())
    } else if without_ext.contains("unrated") {
        Some("unrated".to_string())
    } else if without_ext.contains("repack") || without_ext.contains("proper") {
        Some("repack".to_string())
    } else {
        None
    };

    let variant = detect_spanish_variant(without_ext);
    let group = extract_release_group(without_ext);

    ReleaseMetadata {
        source,
        streaming_service,
        group,
        resolution,
        codec,
        edition,
        variant,
    }
}

/// Detecta si el texto indica explícitamente variante castellano (`es-ES`) o latino (`es-LA`).
pub fn detect_spanish_variant(lower_text: &str) -> Option<String> {
    if lower_text.contains("castellano")
        || lower_text.contains("es-es")
        || lower_text.contains("españa")
        || lower_text.contains("espana")
    {
        return Some("es-ES".to_string());
    }
    if lower_text.contains("latino")
        || lower_text.contains("es-la")
        || lower_text.contains("es-mx")
        || lower_text.contains("latam")
    {
        return Some("es-LA".to_string());
    }
    None
}

fn strip_known_extension(s: &str) -> &str {
    for ext in [".mkv", ".mp4", ".avi", ".srt", ".ass", ".vtt", ".sub"] {
        if let Some(stripped) = s.strip_suffix(ext) {
            return stripped;
        }
    }
    s
}

fn has_token(haystack: &str, token: &str) -> bool {
    haystack
        .split(|c: char| !c.is_alphanumeric())
        .any(|part| part == token)
}

fn extract_release_group(without_ext: &str) -> Option<String> {
    let (_, tail) = without_ext.rsplit_once('-')?;
    let clean = tail
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .trim();
    if clean.len() < 2 || clean.len() > 20 {
        return None;
    }
    if clean.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if matches!(
        clean,
        "dl" | "ray" | "rip" | "1080p" | "720p" | "2160p" | "x264" | "x265" | "h264" | "h265"
            | "es" | "spa" | "eng"
    ) {
        return None;
    }
    Some(clean.to_string())
}

/// Evalúa un candidato frente al contexto del vídeo y las preferencias del usuario.
pub fn score_candidate(
    cand: &ScorableCandidate<'_>,
    target: &SubtitleTargetContext,
    prefs: &SubtitlePreferences,
) -> CandidateScore {
    let mut reasons = Vec::new();

    if cand.is_bad {
        return CandidateScore {
            accepted: false,
            score: i32::MIN,
            reasons: vec!["descartado: marcado como defectuoso (SubBad)".to_string()],
        };
    }

    if cand.is_hi && !prefs.allow_hi {
        return CandidateScore {
            accepted: false,
            score: i32::MIN,
            reasons: vec!["descartado: subtítulo HI no permitido".to_string()],
        };
    }

    if let Some((target_s, target_e)) = target.season_ep {
        if let (Some(cs), Some(ce)) = (cand.season, cand.episode) {
            if cs != target_s || ce != target_e {
                return CandidateScore {
                    accepted: false,
                    score: i32::MIN,
                    reasons: vec![format!(
                        "descartado: episodio S{cs:02}E{ce:02} != S{target_s:02}E{target_e:02}"
                    )],
                };
            }
        }
    }

    let combined_cand_name = match cand.release_name {
        Some(rel) if !rel.is_empty() => format!("{} {}", cand.title, rel),
        _ => cand.title.to_string(),
    };
    let cand_meta = parse_release_metadata(&combined_cand_name);
    let cand_variant = cand
        .variant_hint
        .map(str::to_string)
        .or_else(|| cand_meta.variant.clone());

    if prefs.variant == "es-ES" && cand_variant.as_deref() == Some("es-LA") {
        return CandidateScore {
            accepted: false,
            score: i32::MIN,
            reasons: vec!["descartado: variante latino con preferencia es-ES".to_string()],
        };
    }
    if prefs.variant == "es-LA" && cand_variant.as_deref() == Some("es-ES") {
        return CandidateScore {
            accepted: false,
            score: i32::MIN,
            reasons: vec!["descartado: variante castellano con preferencia es-LA".to_string()],
        };
    }

    let mut score: i32 = 0;

    if cand.matched_by == "moviehash" {
        score += SCORE_MOVIEHASH_EXACT;
        reasons.push(format!("+{SCORE_MOVIEHASH_EXACT} moviehash exacto"));
    }

    if let Some(ref rel_name) = target.release_name {
        let target_meta = parse_release_metadata(rel_name);

        if target_meta.group.is_some() && target_meta.group == cand_meta.group {
            score += SCORE_RELEASE_GROUP;
            reasons.push(format!("+{SCORE_RELEASE_GROUP} grupo"));
        }
        if target_meta.source.is_some() && target_meta.source == cand_meta.source {
            score += SCORE_SOURCE_MATCH;
            reasons.push(format!("+{SCORE_SOURCE_MATCH} fuente"));
        }
        if target_meta.streaming_service.is_some()
            && target_meta.streaming_service == cand_meta.streaming_service
        {
            score += SCORE_SERVICE_MATCH;
            reasons.push(format!("+{SCORE_SERVICE_MATCH} servicio"));
        }
        match (&target_meta.edition, &cand_meta.edition) {
            (Some(te), Some(ce)) if te == ce => {
                score += SCORE_EDITION_MATCH;
                reasons.push(format!("+{SCORE_EDITION_MATCH} edición"));
            }
            (Some(_), None) | (None, Some(_)) => {
                score += PENALTY_EDITION_MISMATCH;
                reasons.push(format!("{PENALTY_EDITION_MISMATCH} edición distinta"));
            }
            _ => {}
        }
        if (target_meta.resolution.is_some() && target_meta.resolution == cand_meta.resolution)
            || (target_meta.codec.is_some() && target_meta.codec == cand_meta.codec)
        {
            score += SCORE_TECH_MATCH;
            reasons.push(format!("+{SCORE_TECH_MATCH} resolución/códec"));
        }
    }

    if let (Some(vf), Some(cf)) = (target.fps_milli, cand.fps_milli) {
        if vf.abs_diff(cf) <= FPS_TOLERANCE_MILLI {
            score += SCORE_FPS_MATCH;
            reasons.push(format!("+{SCORE_FPS_MATCH} FPS coincidente"));
        } else {
            score += PENALTY_FPS_MISMATCH;
            reasons.push(format!("{PENALTY_FPS_MISMATCH} FPS distinto"));
        }
    }

    if prefs.variant != "any" && cand_variant.as_deref() == Some(prefs.variant.as_str()) {
        score += SCORE_VARIANT_MATCH;
        reasons.push(format!("+{SCORE_VARIANT_MATCH} variante {}", prefs.variant));
    }

    if cand.is_trusted {
        score += SCORE_TRUSTED;
        reasons.push(format!("+{SCORE_TRUSTED} confianza"));
    }

    if cand.is_auto_translated {
        score += PENALTY_AUTO_TRANSLATED;
        reasons.push(format!("{PENALTY_AUTO_TRANSLATED} traducción automática"));
    }

    let pop_bonus = ((cand.downloads.saturating_add(1) as f64).log10().floor() as i32 * 2)
        .clamp(0, SCORE_DOWNLOADS_CAP);
    if pop_bonus > 0 {
        score += pop_bonus;
        reasons.push(format!("+{pop_bonus} popularidad"));
    }

    CandidateScore {
        accepted: true,
        score,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_release_metadata_extraction() {
        let meta = parse_release_metadata("Dune.Part.Two.2024.1080p.AMZN.WEB-DL.DDP5.1.H264-FLUX.mkv");
        assert_eq!(meta.source.as_deref(), Some("web-dl"));
        assert_eq!(meta.streaming_service.as_deref(), Some("amzn"));
        assert_eq!(meta.resolution.as_deref(), Some("1080p"));
        assert_eq!(meta.codec.as_deref(), Some("h264"));
        assert_eq!(meta.group.as_deref(), Some("flux"));
    }

    #[test]
    fn test_score_prefers_matching_release_over_raw_downloads() {
        let target = SubtitleTargetContext {
            release_name: Some("The.Bear.S03E01.1080p.DSNP.WEB-DL.H264-NTb.mkv".to_string()),
            fps_milli: Some(23_976),
            season_ep: Some((3, 1)),
        };
        let prefs = SubtitlePreferences::default();

        let exact_release = ScorableCandidate {
            title: "The.Bear.S03E01.1080p.DSNP.WEB-DL.H264-NTb.es.srt",
            release_name: Some("The.Bear.S03E01.1080p.DSNP.WEB-DL.H264-NTb"),
            matched_by: "imdbid",
            downloads: 40,
            is_hi: false,
            is_bad: false,
            is_trusted: true,
            is_auto_translated: false,
            fps_milli: Some(23_976),
            season: Some(3),
            episode: Some(1),
            variant_hint: Some("es-ES"),
        };

        let popular_mismatch = ScorableCandidate {
            title: "The.Bear.S03E01.HDTV.x264-LOL.srt",
            release_name: Some("The.Bear.S03E01.HDTV.x264-LOL"),
            matched_by: "query",
            downloads: 50_000,
            is_hi: false,
            is_bad: false,
            is_trusted: false,
            is_auto_translated: false,
            fps_milli: Some(25_000),
            season: Some(3),
            episode: Some(1),
            variant_hint: None,
        };

        let s_exact = score_candidate(&exact_release, &target, &prefs);
        let s_pop = score_candidate(&popular_mismatch, &target, &prefs);
        assert!(s_exact.accepted && s_pop.accepted);
        assert!(s_exact.score > s_pop.score + 50);
    }

    #[test]
    fn test_hard_reject_wrong_episode_bad_and_latino() {
        let target = SubtitleTargetContext {
            release_name: None,
            fps_milli: Some(23_976),
            season_ep: Some((2, 4)),
        };
        let prefs = SubtitlePreferences::default();

        let wrong_ep = ScorableCandidate {
            title: "Show.S01E01.srt",
            release_name: None,
            matched_by: "imdbid",
            downloads: 1000,
            is_hi: false,
            is_bad: false,
            is_trusted: false,
            is_auto_translated: false,
            fps_milli: None,
            season: Some(1),
            episode: Some(1),
            variant_hint: None,
        };
        assert!(!score_candidate(&wrong_ep, &target, &prefs).accepted);

        let bad_sub = ScorableCandidate {
            is_bad: true,
            season: Some(2),
            episode: Some(4),
            ..wrong_ep
        };
        assert!(!score_candidate(&bad_sub, &target, &prefs).accepted);

        let latino_sub = ScorableCandidate {
            title: "Show.S02E04.Latino.srt",
            season: Some(2),
            episode: Some(4),
            ..wrong_ep
        };
        assert!(!score_candidate(&latino_sub, &target, &prefs).accepted);
    }
}
