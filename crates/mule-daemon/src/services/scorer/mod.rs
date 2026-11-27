// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredRelease {
    pub title: String,
    pub protocol: String, // "ed2k" | "torrent"
    pub link: String,
    pub hash: String,
    pub size_bytes: u64,
    pub sources: u64,
    pub score: i32,
    pub quality: String,
    pub language: String,
    pub is_rejected: bool,
    pub rejection_reason: Option<String>,
    pub indexer_name: String,
}


mod dictionaries;
mod matching;
#[cfg(test)]
mod tests;

pub use dictionaries::spanish_title_search_variants;
pub use matching::{
    clean_title_tokens, extract_tv_episode_numbers, matches_release, matches_release_with_alts,
    matches_release_with_options, matches_season_pack_release, normalize_accents_and_symbols,
    strip_trailing_year_suffix,
};

use dictionaries::{FORBIDDEN_EXTENSIONS, TRUSTED_RELEASE_GROUPS};
use matching::{has_season_pack_indicator, is_year_token};
/// Umbrales de tamaño óptimo ("Sweet-Spot" en MB) para equilibrar calidad de imagen y almacenamiento.
const MOVIE_1080P_SWEET_MIN_MB: u64 = 1800;
const MOVIE_1080P_SWEET_MAX_MB: u64 = 9500;
const MOVIE_1080P_MEDIUM_MAX_MB: u64 = 16000;
const MOVIE_OVERSIZED_THRESHOLD_MB: u64 = 25000;
const EPISODE_SWEET_MIN_MB: u64 = 350;
const EPISODE_SWEET_MAX_MB: u64 = 2200;
const SEASON_PACK_SWEET_MIN_MB: u64 = 2500;
const SEASON_PACK_SWEET_MAX_MB: u64 = 28000;
pub(super) const MAX_EPISODE_RANGE_SPAN: u32 = 30;
/// Bonificación adicional para lanzamientos con señal fuerte de Castellano de España en filtro estricto.
const STRICT_CASTILIAN_VERIFIED_BONUS: i32 = 30;
/// Bonificación adicional para lanzamientos Dual en filtro estricto.
const STRICT_CASTILIAN_DUAL_BONUS: i32 = 10;
/// Puntuación asignada a los lanzamientos descartados por el filtro estricto de Castellano de España.
const STRICT_CASTILIAN_REJECTED_SCORE: i32 = -999;
/// Penalización en eD2K cuando solo se detecta 1 fuente (alto riesgo de par desconectado o LowID).
const ED2K_SINGLE_SOURCE_PENALTY: i32 = 30;
/// Bonificación en eD2K para lanzamientos con disponibilidad saludable (3 a 5 fuentes).
const ED2K_HEALTHY_SOURCES_BONUS: i32 = 20;
/// Bonificación en eD2K para lanzamientos con alta disponibilidad (6 o más fuentes).
const ED2K_STRONG_SOURCES_BONUS: i32 = 35;

/// Comprueba si el título o el indexador pertenecen a una fuente española de confianza conocida.
fn is_trusted_spanish_source(lower_title: &str, indexer_name: &str) -> bool {
    TRUSTED_RELEASE_GROUPS.iter().any(|&grp| lower_title.contains(grp))
        || indexer_name.eq_ignore_ascii_case("dontorrent")
        || indexer_name.eq_ignore_ascii_case("grantorrent")
        || indexer_name.eq_ignore_ascii_case("divxtotal")
        || indexer_name.eq_ignore_ascii_case("atomixhq")
        || indexer_name.eq_ignore_ascii_case("pctfenix")
}

pub fn evaluate_release(
    title: &str,
    protocol: &str,
    link: &str,
    hash: &str,
    size_bytes: u64,
    sources: u64,
    indexer_name: &str,
    is_movie: bool,
    preferred_quality: &str,
    preferred_language: &str,
) -> ScoredRelease {
    let lower = title.to_lowercase();
    let mut score = 0i32;
    let mut is_rejected = false;
    let mut rejection_reason = None;

    // 1. Comprobación antifalsificaciones y extensiones peligrosas
    for &ext in FORBIDDEN_EXTENSIONS {
        if lower.ends_with(ext) || lower.contains(&format!("{ext}.")) {
            is_rejected = true;
            rejection_reason = Some(format!("Forbidden extension '{ext}' detected (fake/malware)"));
            break;
        }
    }

    // 1b. Detección de muestra / tráiler / teaser
    for &sample_word in &["sample", "trailer", "teaser", "promo", "featurette", "preview"] {
        let is_sample = lower.contains(&format!(".{sample_word}."))
            || lower.contains(&format!("-{sample_word}-"))
            || lower.contains(&format!("_{sample_word}_"))
            || lower.contains(&format!(" {sample_word} "))
            || lower.starts_with(&format!("{sample_word}."))
            || lower.ends_with(&format!(".{sample_word}"));
        if is_sample && !is_rejected {
            is_rejected = true;
            rejection_reason = Some(format!("Non-feature sample/preview detected ('{sample_word}')"));
            break;
        }
    }

    // 2. Tamaño creíble mínimo y máximo
    let pref_lower = preferred_quality.to_lowercase();
    let has_tv_marker = !extract_tv_episode_numbers(title, 1).is_empty()
        || lower.contains("1x")
        || lower.contains("2x")
        || lower.contains("3x")
        || lower.contains("s0")
        || lower.contains("s1")
        || lower.contains("cap.")
        || lower.contains("capitulo")
        || lower.contains("temporada");
    let effective_is_movie = is_movie && !has_tv_marker;
    let is_likely_season_pack = !effective_is_movie
        && has_season_pack_indicator(&lower)
        && !lower.contains("cap.")
        && !lower.contains("capitulo");

    let has_early_year = {
        let norm_full = normalize_accents_and_symbols(title);
        norm_full
            .split_whitespace()
            .filter_map(is_year_token)
            .any(|y| y < 1930)
    };
    let is_short_or_early_cinema = has_early_year
        || lower.contains("silent")
        || lower.contains("muda")
        || lower.contains("muto")
        || lower.contains("muet")
        || lower.contains("cortometraje")
        || lower.contains("short film");

    let min_size = if is_short_or_early_cinema {
        35 * 1024 * 1024 // 35 MB para cortometrajes o cine mudo de los orígenes (ej. Le Voyage dans la Lune 1902)
    } else if effective_is_movie {
        if pref_lower.contains("4k") || pref_lower.contains("2160p") {
            1800 * 1024 * 1024 // 1,8 GB mínimo para película 4K
        } else if pref_lower.contains("1080p") {
            450 * 1024 * 1024 // 450 MB mínimo para película 1080p (evita rechazar codificaciones 649 MB / microHD)
        } else {
            250 * 1024 * 1024 // 250 MB mínimo para película estándar (SD/720p)
        }
    } else if is_likely_season_pack {
        900 * 1024 * 1024 // 900 MB mínimo para un paquete de temporada completo
    } else if pref_lower.contains("1080p") {
        180 * 1024 * 1024 // 180 MB mínimo para episodio de TV en 1080p
    } else {
        40 * 1024 * 1024 // 40 MB para episodio de TV
    };
    if size_bytes < min_size && !is_rejected {
        is_rejected = true;
        rejection_reason = Some(format!(
            "Size ({} MB) is suspiciously small for credible video content",
            size_bytes / (1024 * 1024)
        ));
    }

    let max_size = if is_movie {
        85 * 1024 * 1024 * 1024
    } else if is_likely_season_pack {
        65 * 1024 * 1024 * 1024
    } else {
        18 * 1024 * 1024 * 1024
    };
    if size_bytes > max_size && !is_rejected {
        is_rejected = true;
        rejection_reason = Some(format!(
            "Size ({} GB) exceeds maximum credible bounds (possible fake/bomb)",
            size_bytes / (1024 * 1024 * 1024)
        ));
    }

    // 3. Palabras clave conocidas de falsificaciones y contenido prohibido/spam
    for bad in &[
        "codec", "password", "contraseña", "crack", "keygen", "virus",
        "setup.exe", "install.exe", "wmv-codec", "drm-protected", "password.txt",
        "pthc", "pedofilia", "pedophilia", "hebephilia", "hussyfan", "r@ygold",
        "babyshivid", "teen anal",
    ] {
        if lower.contains(bad) && !is_rejected {
            is_rejected = true;
            rejection_reason = Some(format!("Suspicious keyword '{bad}' detected"));
            break;
        }
    }

    // 4. Penalización o rechazo de CAM / TeleSync
    if lower.contains("camrip")
        || lower.contains("cam-rip")
        || lower.contains("hdcam")
        || lower.contains("telesync")
        || lower.contains("tele-sync")
        || lower.contains(".ts.")
        || lower.contains("screener")
        || lower.contains("dvdscr")
    {
        is_rejected = true;
        rejection_reason = Some("Cam/TeleSync/Screener low quality release rejected".to_string());
    }

    // 5. Detección de calidad
    let quality = if lower.contains("2160p") || lower.contains("4k") || lower.contains("uhd") {
        "2160p".to_string()
    } else if lower.contains("1080p")
        || lower.contains("1080i")
        || lower.contains("1080x")
        || lower.contains(".1080.")
        || lower.contains(" 1080 ")
        || lower.contains("fhd")
        || lower.contains("bluray")
        || lower.contains("bdrip")
        || lower.contains("web-dl")
        || lower.contains("web dl")
        || lower.contains("webrip")
    {
        "1080p".to_string()
    } else if lower.contains("720p") || lower.contains("hdrip") || lower.contains("hdtv") {
        "720p".to_string()
    } else {
        "SD".to_string()
    };

    // Tope estricto de calidad para preservar espacio en el NAS:
    // Si el perfil preferido NO solicita explícitamente 4K/2160p (ej. "1080p-es"),
    // se descarta cualquier release en 4K/2160p/UHD para evitar descargas gigantes de 20-50GB.
    let wants_4k = pref_lower.contains("4k") || pref_lower.contains("2160p");
    if quality == "2160p" && !wants_4k && !is_rejected {
        is_rejected = true;
        rejection_reason = Some("Calidad 4K/2160p rechazada: el perfil de calidad tiene como tope 1080p para preservar espacio en el NAS".to_string());
    }

    match quality.as_str() {
        "1080p" => score += 50,
        "720p" => score += 35,
        "2160p" => {
            if wants_4k {
                score += 60;
            } else {
                score += 40;
            }
        }
        _ => score += 15,
    }

    // 6. Detección de idioma (estricto Castellano / Dual / VOSE vs Latino / Extranjero / puro inglés)
    let norm_words: Vec<String> = normalize_accents_and_symbols(title)
        .split_whitespace()
        .map(String::from)
        .collect();
    let has_word = |w: &str| norm_words.iter().any(|nw| nw == w);

    let has_trusted_spanish_group = is_trusted_spanish_source(&lower, indexer_name);

    let is_sub_prefix = |w: &str| {
        matches!(
            w,
            "sub" | "subs" | "subt" | "subtitulos" | "subtitulado" | "subtitled"
        )
    };
    let has_standalone_spa_esp = norm_words.iter().enumerate().any(|(idx, nw)| {
        (nw == "spa" || nw == "esp")
            && (idx == 0 || !is_sub_prefix(&norm_words[idx - 1]))
    });
    let has_sub_prefixed_spa_esp = norm_words.iter().enumerate().any(|(idx, nw)| {
        (nw == "spa" || nw == "esp" || nw == "spanish" || nw == "espanol")
            && idx > 0
            && is_sub_prefix(&norm_words[idx - 1])
    });

    let has_spanish_sub_only = lower.contains("spanishsub")
        || lower.contains("sub.spanish")
        || lower.contains("sub.esp")
        || lower.contains("subt.esp")
        || lower.contains("sub_esp")
        || lower.contains("subs.esp")
        || lower.contains("subtitulado")
        || has_sub_prefixed_spa_esp;

    let has_castellano_audio = lower.contains("castellano")
        || lower.contains("español")
        || lower.contains("espanol")
        || has_standalone_spa_esp
        || (lower.contains("spanish")
            && (!has_spanish_sub_only
                || lower.contains("spanish.english")
                || lower.contains("spanish-english")
                || lower.contains("spanish english")
                || lower.contains("[spanish]")
                || lower.contains("(spanish)")));

    let has_english = lower.contains("english")
        || has_word("eng")
        || has_word("ing")
        || lower.contains("ingles")
        || lower.contains("inglés");
    let has_dual_tag =
        has_word("dual") || has_word("multi") || lower.contains("tri-audio") || lower.contains("triaudio");
    let has_subs = has_word("vose")
        || has_word("vos")
        || has_word("sub")
        || has_word("subt")
        || has_word("subs")
        || has_spanish_sub_only
        || lower.contains("subs")
        || lower.contains("[sub]");
    let has_latino = lower.contains("latino") && !lower.contains("castellano");
    let has_foreign = lower.contains("french")
        || lower.contains("truefrench")
        || has_word("vff")
        || has_word("vf2")
        || has_word("vfq")
        || has_word("vostfr")
        || has_word("fr")
        || has_word("fre")
        || has_word("fra")
        || lower.contains("catala")
        || lower.contains("catalan")
        || has_word("cat")
        || lower.contains("italian")
        || lower.contains("italiano")
        || has_word("ita")
        || lower.contains("german")
        || lower.contains("deutsch")
        || has_word("ger")
        || has_word("deu")
        || lower.contains("russian")
        || has_word("rus")
        || lower.contains("dublado")
        || has_word("pol")
        || lower.contains("polish");

    let has_silent = lower.contains("silent")
        || lower.contains("muda")
        || lower.contains("muto")
        || lower.contains("muet")
        || lower.contains("dts-zxx")
        || lower.contains("zxx")
        || has_early_year;

    let language = if has_castellano_audio && (has_english || has_dual_tag || has_foreign) {
        "Dual".to_string()
    } else if has_castellano_audio {
        "Castellano".to_string()
    } else if has_silent {
        "VOSE".to_string() // Film muet sans dialogue vocal : accepté en VOSE
    } else if has_latino {
        "Latino".to_string()
    } else if has_foreign && !has_castellano_audio {
        "Foreign".to_string()
    } else if has_subs {
        "VOSE".to_string()
    } else if has_dual_tag && (has_trusted_spanish_group || !has_foreign) {
        "Dual".to_string()
    } else if has_trusted_spanish_group && !has_english {
        "Castellano".to_string()
    } else if has_english {
        "English".to_string()
    } else {
        "Unknown".to_string()
    };

    let wants_spanish = preferred_language.to_lowercase().contains("es")
        || preferred_language.to_lowercase().contains("spa")
        || preferred_language.to_lowercase().contains("castellano");

    if wants_spanish {
        match language.as_str() {
            "Castellano" => score += 65,
            "Dual" => score += 60,
            "VOSE" => score += 30,
            "Latino" | "Foreign" | "English" | "Unknown" => {
                if !is_rejected {
                    is_rejected = true;
                    rejection_reason = Some(format!(
                        "Idioma '{language}' descartado (se requiere Castellano, Dual o VOSE explícito)"
                    ));
                }
            }
            _ => score += 5,
        }
    } else {
        score += 30;
    }

    // 7. Bonificación por fuentes / seeders (hasta 40 pts) y ajuste eD2K por disponibilidad real
    score += (sources.min(20) * 2) as i32;
    if protocol == "ed2k" {
        if sources <= 1 {
            // Una única fuente reportada en Kad/servidor suele corresponder a pares desconectados o LowID
            score -= ED2K_SINGLE_SOURCE_PENALTY;
        } else if sources >= 6 {
            score += ED2K_STRONG_SOURCES_BONUS;
        } else if sources >= 3 {
            score += ED2K_HEALTHY_SOURCES_BONUS;
        }
    }

    // 8. Bonificación de velocidad de protocolo BitTorrent (+10 pts si hay muchos seeders)
    if protocol == "torrent" && sources >= 5 {
        score += 10;
    }

    // 9. Bonificación por códec moderno eficiente en espacio (x265 / HEVC / AV1)
    if lower.contains("x265") || lower.contains("h265") || lower.contains("hevc") || lower.contains("av1") {
        score += 25;
    }

    // 10. Bonificación por calidad de audio multicanal (5.1, DTS, Atmos, TrueHD, AC3, EAC3)
    if lower.contains("5.1")
        || lower.contains("7.1")
        || lower.contains("atmos")
        || lower.contains("dts")
        || lower.contains("truehd")
        || lower.contains("eac3")
        || lower.contains("ddp5")
    {
        score += 10;
    }

    // 11. Curva de tasa de bits / tamaño óptimo (optimizado para Raspberry Pi / Jellyfin Direct Play)
    let size_mb = size_bytes / (1024 * 1024);
    if is_movie {
        if (MOVIE_1080P_SWEET_MIN_MB..=MOVIE_1080P_SWEET_MAX_MB).contains(&size_mb) {
            score += 25;
        } else if (1100..MOVIE_1080P_SWEET_MIN_MB).contains(&size_mb)
            || (MOVIE_1080P_SWEET_MAX_MB + 1..=MOVIE_1080P_MEDIUM_MAX_MB).contains(&size_mb)
        {
            score += 10;
        } else if size_mb > MOVIE_OVERSIZED_THRESHOLD_MB
            && !preferred_quality.to_lowercase().contains("remux")
        {
            score -= 20;
        }
    } else if is_likely_season_pack {
        if (SEASON_PACK_SWEET_MIN_MB..=SEASON_PACK_SWEET_MAX_MB).contains(&size_mb) {
            score += 25;
        }
    } else if (EPISODE_SWEET_MIN_MB..=EPISODE_SWEET_MAX_MB).contains(&size_mb) {
        score += 20;
    }

    // 12. Bonificación por uploader / grupo de lanzamiento de confianza (+20 pts)
    if TRUSTED_RELEASE_GROUPS.iter().any(|&grp| lower.contains(grp)) {
        score += 20;
    }

    ScoredRelease {
        title: title.to_string(),
        protocol: protocol.to_string(),
        link: link.to_string(),
        hash: hash.to_string(),
        size_bytes,
        sources,
        score: if is_rejected { -999 } else { score },
        quality,
        language,
        is_rejected,
        rejection_reason,
        indexer_name: indexer_name.to_string(),
    }
}

/// Detecta marcadores de español latinoamericano cuando no hay indicación explícita de Castellano de España.
pub fn has_latino_marker(title: &str) -> bool {
    let lower = title.to_lowercase();
    if lower.contains("castellano") || lower.contains("es-es") || lower.contains("es_es") {
        return false;
    }
    if lower.contains("latino")
        || lower.contains("es-419")
        || lower.contains("es_419")
        || lower.contains("latam")
        || lower.contains("argentina")
    {
        return true;
    }
    let norm = normalize_accents_and_symbols(title);
    norm.split_whitespace()
        .any(|w| matches!(w, "lat" | "latin" | "mx" | "mex"))
}

/// Clasifica el nivel de confianza de que el lanzamiento contenga audio en Castellano de España.
/// Posibles valores: `"verified"`, `"dual"`, `"possible"` o `"none"`.
pub fn castilian_confidence_level(release: &ScoredRelease) -> &'static str {
    if release.is_rejected || has_latino_marker(&release.title) {
        return "none";
    }
    if release.language != "Castellano" && release.language != "Dual" {
        return "none";
    }

    let lower = release.title.to_lowercase();
    let norm_words: Vec<String> = normalize_accents_and_symbols(&release.title)
        .split_whitespace()
        .map(String::from)
        .collect();
    let has_standalone_esp = norm_words.iter().enumerate().any(|(idx, nw)| {
        nw == "esp"
            && (idx == 0
                || !matches!(
                    norm_words[idx - 1].as_str(),
                    "sub" | "subs" | "subt" | "subtitulos" | "subtitulado" | "subtitled"
                ))
    });

    let has_strong_spain_signal = lower.contains("castellano")
        || lower.contains("es-es")
        || lower.contains("es_es")
        || has_standalone_esp
        || is_trusted_spanish_source(&lower, &release.indexer_name);

    if has_strong_spain_signal {
        "verified"
    } else if release.language == "Dual" {
        "dual"
    } else if release.language == "Castellano" {
        "possible"
    } else {
        "none"
    }
}

/// Indica si un lanzamiento supera el filtro estricto de Castellano de España.
pub fn is_strict_castilian_release(release: &ScoredRelease) -> bool {
    !release.is_rejected && castilian_confidence_level(release) != "none"
}

/// Aplica el filtro estricto de Castellano de España sobre un lote de lanzamientos evaluados,
/// descartando VOSE/Latino/Inglés y priorizando fuentes españolas verificadas.
pub fn apply_strict_castilian_filter(releases: &mut [ScoredRelease]) {
    for r in releases.iter_mut() {
        if r.is_rejected {
            continue;
        }
        match castilian_confidence_level(r) {
            "verified" => r.score += STRICT_CASTILIAN_VERIFIED_BONUS,
            "dual" => r.score += STRICT_CASTILIAN_DUAL_BONUS,
            "possible" => {}
            _ => {
                r.is_rejected = true;
                r.score = STRICT_CASTILIAN_REJECTED_SCORE;
                r.rejection_reason = Some(format!(
                    "Descartado por filtro de audio verificado (idioma detectado: {})",
                    r.language
                ));
            }
        }
    }
}

