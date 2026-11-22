// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Análisis, saneamiento, validación sintáctica y detección heurística de idioma para archivos SRT.

use serde::{Deserialize, Serialize};

/// Número mínimo de bloques de subtítulo válidos exigidos para aceptar un `.srt`.
pub const MIN_VALID_SRT_CUES: usize = 10;
/// Margen máximo (en segundos) que el último subtítulo puede exceder la duración real del vídeo.
const MAX_DURATION_OVERFLOW_SECS: u64 = 300;
/// Duración mínima de vídeo (en segundos) a partir de la cual se exige una cobertura mínima de subtítulos.
const MIN_VIDEO_DURATION_FOR_COVERAGE_CHECK_SECS: u64 = 600;
/// Porcentaje mínimo de duración que debe cubrir el último subtítulo en vídeos largos (15 %).
const MIN_SUBTITLE_COVERAGE_PERCENT: u64 = 15;
/// Proporción mínima de palabras funcionales españolas frente a inglesas para aceptar español.
const MIN_SPANISH_TO_ENGLISH_STOPWORD_RATIO: f64 = 0.6;
/// Número mínimo de coincidencias de palabras funcionales para aplicar la heurística de idioma.
const MIN_STOPWORD_SAMPLE_COUNT: usize = 6;

/// Bloque individual de un archivo SubRip (`.srt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SrtCue {
    pub index: usize,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Resultado de la validación de un archivo SRT descargado.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SrtValidationReport {
    pub cue_count: usize,
    pub last_end_ms: i64,
    pub detected_lang: String,
}

/// Decodifica bytes de subtítulos convirtiendo codificaciones heredadas Windows-1252 / ISO-8859-1 a UTF-8 limpio.
pub fn decode_subtitle_bytes(raw: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.strip_prefix('\u{feff}').unwrap_or(s).to_string();
    }

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

/// Parsea una marca de tiempo SRT (`HH:MM:SS,mmm` o `HH:MM:SS.mmm`) a milisegundos.
fn parse_srt_timestamp_ms(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    let (hms, ms_str) = trimmed
        .split_once(',')
        .or_else(|| trimmed.split_once('.'))?;
    let mut parts = hms.split(':');
    let h: i64 = parts.next()?.trim().parse().ok()?;
    let m: i64 = parts.next()?.trim().parse().ok()?;
    let s: i64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() || m < 0 || s < 0 {
        return None;
    }
    let ms_clean: String = ms_str.chars().take_while(|c| c.is_ascii_digit()).collect();
    if ms_clean.is_empty() {
        return None;
    }
    let mut ms: i64 = ms_clean.parse().ok()?;
    if ms_clean.len() == 1 {
        ms *= 100;
    } else if ms_clean.len() == 2 {
        ms *= 10;
    } else if ms_clean.len() > 3 {
        ms /= 10_i64.pow((ms_clean.len() - 3) as u32);
    }
    Some(((h * 3600 + m * 60 + s) * 1000) + ms)
}

/// Formatea milisegundos al formato estándar SRT (`HH:MM:SS,mmm`).
pub fn format_srt_timestamp_ms(ms: i64) -> String {
    let clamped = ms.max(0);
    let total_secs = clamped / 1000;
    let millis = clamped % 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{hours:02}:{mins:02}:{secs:02},{millis:03}")
}

/// Parsea un documento SRT completo en una lista de bloques [`SrtCue`].
pub fn parse_srt(content: &str) -> Result<Vec<SrtCue>, String> {
    let trimmed_start = content.trim_start();
    let lower_head: String = trimmed_start.chars().take(128).collect::<String>().to_lowercase();
    if lower_head.starts_with("<!doctype")
        || lower_head.starts_with("<html")
        || lower_head.starts_with("{")
    {
        return Err("El contenido descargado no es un archivo SRT válido (HTML/JSON)".to_string());
    }

    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }

        // Localizar la línea con `-->` (el número de índice puede faltar en archivos mal formados).
        let Some(arrow_idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };

        let timing_line = lines[arrow_idx];
        let Some((start_raw, end_raw_full)) = timing_line.split_once("-->") else {
            continue;
        };
        let end_raw = end_raw_full.split_whitespace().next().unwrap_or("");
        let (Some(start_ms), Some(end_ms)) = (
            parse_srt_timestamp_ms(start_raw),
            parse_srt_timestamp_ms(end_raw),
        ) else {
            continue;
        };

        if end_ms < start_ms {
            continue;
        }

        let text = if arrow_idx + 1 < lines.len() {
            lines[arrow_idx + 1..].join("\n")
        } else {
            String::new()
        };

        if text.trim().is_empty() {
            continue;
        }

        cues.push(SrtCue {
            index: cues.len() + 1,
            start_ms,
            end_ms,
            text,
        });
    }

    if cues.is_empty() {
        return Err("No se encontraron bloques de subtítulos válidos en el SRT".to_string());
    }

    Ok(cues)
}

/// Serializa una lista de [`SrtCue`] a texto SRT normalizado con saltos `\n`.
pub fn format_srt(cues: &[SrtCue]) -> String {
    let mut out = String::new();
    for (idx, cue) in cues.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n",
            idx + 1,
            format_srt_timestamp_ms(cue.start_ms),
            format_srt_timestamp_ms(cue.end_ms),
            cue.text.trim()
        ));
    }
    out
}

/// Comprueba si un bloque contiene publicidad habitual de portales de subtítulos.
fn is_portal_advertisement(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("opensubtitles.org")
        || lower.contains("opensubtitles.com")
        || lower.contains("subscene")
        || lower.contains("addic7ed")
        || lower.contains("podnapisi")
        || lower.contains("subtitulos por")
        || lower.contains("subtítulos por")
        || lower.contains("descargado de")
        || lower.contains("apoya nuestra web")
}

/// Elimina acotaciones entre corchetes o paréntesis típicas de subtítulos HI (`[música]`, `(risas)`).
fn strip_hi_annotations(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut in_bracket = false;
    let mut in_paren = false;
    for ch in text.chars() {
        match ch {
            '[' => in_bracket = true,
            ']' if in_bracket => in_bracket = false,
            '(' => in_paren = true,
            ')' if in_paren => in_paren = false,
            _ if !in_bracket && !in_paren => cleaned.push(ch),
            _ => {}
        }
    }
    cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "-")
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sanea anuncios de portales y opcionalmente anotaciones HI, renumerando los bloques.
pub fn sanitize_cues(cues: &mut Vec<SrtCue>, strip_hi: bool) {
    cues.retain(|c| !is_portal_advertisement(&c.text));
    if strip_hi {
        for cue in cues.iter_mut() {
            cue.text = strip_hi_annotations(&cue.text);
        }
        cues.retain(|c| !c.text.trim().is_empty());
    }
    for (idx, cue) in cues.iter_mut().enumerate() {
        cue.index = idx + 1;
    }
}

/// Estima de forma ligera si el texto del subtítulo está en español (`"es"`) o inglés (`"en"`).
pub fn detect_cue_language(cues: &[SrtCue]) -> String {
    let mut es_hits: usize = 0;
    let mut en_hits: usize = 0;

    for cue in cues.iter().take(200) {
        let lower = cue.text.to_lowercase();
        for raw_word in lower.split(|c: char| !c.is_alphabetic()) {
            match raw_word {
                "que" | "los" | "las" | "una" | "para" | "por" | "con" | "pero" | "como"
                | "esta" | "está" | "están" | "estoy" | "eso" | "muy" | "más" | "también"
                | "sí" | "qué" | "dónde" | "cuándo" | "quién" => es_hits += 1,
                "the" | "and" | "you" | "that" | "this" | "with" | "what" | "have" | "for"
                | "not" | "are" | "was" | "they" | "from" | "your" | "she" | "his" => {
                    en_hits += 1
                }
                _ => {}
            }
        }
    }

    let total = es_hits + en_hits;
    if total < MIN_STOPWORD_SAMPLE_COUNT {
        return "unknown".to_string();
    }

    if (es_hits as f64) >= (en_hits as f64) * MIN_SPANISH_TO_ENGLISH_STOPWORD_RATIO
        && es_hits > 0
    {
        "es".to_string()
    } else if en_hits > es_hits * 2 {
        "en".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Valida un conjunto de bloques SRT frente a la duración del vídeo y el idioma esperado.
pub fn validate_srt_cues(
    cues: &[SrtCue],
    video_duration_secs: Option<u64>,
    expected_lang: &str,
) -> Result<SrtValidationReport, String> {
    if cues.len() < MIN_VALID_SRT_CUES {
        return Err(format!(
            "El archivo SRT solo contiene {} bloques válidos (mínimo {MIN_VALID_SRT_CUES})",
            cues.len()
        ));
    }

    let last_end_ms = cues.iter().map(|c| c.end_ms).max().unwrap_or(0);
    let last_end_secs = (last_end_ms.max(0) as u64) / 1000;

    if let Some(vid_secs) = video_duration_secs {
        if last_end_secs > vid_secs.saturating_add(MAX_DURATION_OVERFLOW_SECS) {
            return Err(format!(
                "Duración incompatible: el subtítulo llega a {last_end_secs}s y el vídeo dura {vid_secs}s"
            ));
        }
        if vid_secs >= MIN_VIDEO_DURATION_FOR_COVERAGE_CHECK_SECS {
            let min_expected_secs = (vid_secs * MIN_SUBTITLE_COVERAGE_PERCENT) / 100;
            if last_end_secs < min_expected_secs {
                return Err(format!(
                    "Subtítulo incompleto: termina en {last_end_secs}s para un vídeo de {vid_secs}s"
                ));
            }
        }
    }

    let detected_lang = detect_cue_language(cues);
    let norm_expected = expected_lang.trim().to_lowercase();
    if matches!(norm_expected.as_str(), "es" | "spa" | "spanish" | "castellano")
        && detected_lang == "en"
    {
        return Err("El archivo SRT descargado está en inglés y no en español".to_string());
    }

    Ok(SrtValidationReport {
        cue_count: cues.len(),
        last_end_ms,
        detected_lang,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spanish_srt(count: usize, step_ms: i64) -> String {
        let mut s = String::new();
        for i in 0..count {
            let start = (i as i64) * step_ms + 1000;
            let end = start + 2000;
            s.push_str(&format!(
                "{}\n{} --> {}\n¿Qué está pasando con los archivos para esta película?\n\n",
                i + 1,
                format_srt_timestamp_ms(start),
                format_srt_timestamp_ms(end)
            ));
        }
        s
    }

    #[test]
    fn test_parse_sanitize_and_validate_spanish_srt() {
        let raw = format!(
            "1\n00:00:00,500 --> 00:00:02,000\nDescargado de OpenSubtitles.org\n\n{}",
            sample_spanish_srt(12, 60_000)
        );
        let mut cues = parse_srt(&raw).expect("parse válido");
        sanitize_cues(&mut cues, false);
        assert_eq!(cues.len(), 12);

        let report = validate_srt_cues(&cues, Some(900), "spa").expect("validación correcta");
        assert_eq!(report.cue_count, 12);
        assert_eq!(report.detected_lang, "es");
    }

    #[test]
    fn test_validate_rejects_wrong_duration_and_english_text() {
        let english = (0..12)
            .map(|i| {
                let start = (i as i64) * 5000 + 1000;
                format!(
                    "{}\n{} --> {}\nThe quick brown fox and the lazy dog from this movie\n\n",
                    i + 1,
                    format_srt_timestamp_ms(start),
                    format_srt_timestamp_ms(start + 2000)
                )
            })
            .collect::<String>();
        let cues = parse_srt(&english).expect("parse válido");
        let err = validate_srt_cues(&cues, Some(120), "es").unwrap_err();
        assert!(err.contains("inglés"));

        let spanish_long = sample_spanish_srt(12, 200_000);
        let cues_long = parse_srt(&spanish_long).expect("parse válido");
        let err_dur = validate_srt_cues(&cues_long, Some(600), "es").unwrap_err();
        assert!(err_dur.contains("Duración incompatible"));
    }

    #[test]
    fn test_rejects_html_payload() {
        let html = "<!DOCTYPE html><html><body>503 Service Unavailable</body></html>";
        assert!(parse_srt(html).is_err());
    }
}
