// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::dictionaries::{
    spanish_title_search_variants, EN_ES_TITLE_WORD_EQUIVALENTS, RELEASE_METADATA_BOUNDARY_TOKENS,
    SEQUEL_AND_SPINOFF_MARKERS, STOP_WORDS, TRUSTED_RELEASE_GROUPS,
};
use super::MAX_EPISODE_RANGE_SPAN;

/// Elimina un sufijo redundante de año entre paréntesis al final de un título
/// (p. ej. `"The Body (2026)"` -> `"The Body"`), preservando títulos que son
/// únicamente un año (p. ej. `"1923"` o `"(2026)"`).
pub fn strip_trailing_year_suffix(title: &str) -> &str {
    let trimmed = title.trim();
    if let Some(without_close) = trimmed.strip_suffix(')') {
        if let Some(open_idx) = without_close.rfind('(') {
            let inside = without_close[open_idx + 1..].trim();
            let prefix = without_close[..open_idx].trim();
            if !prefix.is_empty() && is_year_token(inside).is_some() {
                return prefix;
            }
        }
    }
    trimmed
}

pub fn clean_title_tokens(title: &str) -> Vec<String> {
    let normalized = normalize_accents_and_symbols(strip_trailing_year_suffix(title));
    let tokens: Vec<String> = normalized
        .split_whitespace()
        .filter(|&t| !STOP_WORDS.contains(&t) && (t.len() > 1 || t.chars().all(|c| c.is_ascii_digit())))
        .map(|s| s.to_string())
        .collect();

    if tokens.is_empty() {
        normalized.split_whitespace().map(|s| s.to_string()).collect()
    } else {
        tokens
    }
}

/// Repara secuencias UTF-8 doblemente codificadas en Latin-1 (mojibake frecuente en eD2K/Kad,
/// ej.: `El seÃ±or` -> `El señor`, `InmolaciÃ³n` -> `Inmolación`, `EstaciÃÂ³n` -> `Estación`).
fn repair_mojibake_utf8(input: &str) -> String {
    if !input.contains('Ã') && !input.contains('Â') {
        return input.to_string();
    }
    let mut current = input.to_string();
    for _ in 0..2 {
        if !current.contains('Ã') && !current.contains('Â') {
            break;
        }
        let mut bytes = Vec::with_capacity(current.len());
        let mut can_convert = true;
        for ch in current.chars() {
            let code = ch as u32;
            if code <= 0xFF {
                bytes.push(code as u8);
            } else {
                can_convert = false;
                break;
            }
        }
        if can_convert {
            if let Ok(decoded) = String::from_utf8(bytes) {
                if decoded != current {
                    current = decoded;
                    continue;
                }
            }
        }
        break;
    }
    current
}

pub fn normalize_accents_and_symbols(input: &str) -> String {
    let repaired = repair_mojibake_utf8(input);
    let mut out = String::with_capacity(repaired.len());
    for c in repaired.chars() {
        let mapped = match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            'ç' | 'Ç' => 'c',
            '\'' | '’' => continue, // Fusionner "Rosemary's" -> "rosemarys"
            _ if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            _ => ' ',
        };
        out.push(mapped);
    }
    out
}

pub(super) fn is_year_token(w: &str) -> Option<u32> {
    if w.len() == 4 {
        if let Ok(y) = w.parse::<u32>() {
            if (1890..=2099).contains(&y) && y != 1080 && y != 2160 {
                return Some(y);
            }
        }
    }
    None
}

fn is_standalone_season_token(w: &str) -> Option<u32> {
    let bytes = w.as_bytes();
    if (bytes.first() == Some(&b's') || bytes.first() == Some(&b't'))
        && (2..=3).contains(&bytes.len())
        && w[1..].chars().all(|c| c.is_ascii_digit())
    {
        if let Ok(s) = w[1..].parse::<u32>() {
            if (1..=50).contains(&s) {
                return Some(s);
            }
        }
    }
    None
}

fn is_episode_marker_token(w: &str) -> bool {
    if is_standalone_season_token(w).is_some() {
        return true;
    }
    let bytes = w.as_bytes();
    // Formato s01e01 o s1e1
    if bytes.first() == Some(&b's') && bytes.len() >= 4 && w[1..].contains('e') {
        let parts: Vec<&str> = w[1..].split('e').collect();
        if parts.len() >= 2
            && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            return true;
        }
    }
    // Formato 1x01
    if w.contains('x') {
        let parts: Vec<&str> = w.split('x').collect();
        if parts.len() == 2
            && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            return true;
        }
    }
    false
}

pub(super) fn has_season_pack_indicator(lower: &str) -> bool {
    lower.contains("temporada")
        || lower.contains("season")
        || lower.contains("completa")
        || lower.contains("complete")
        || lower.contains("pack")
}

/// Extrae de un nombre de archivo P2P:
/// 1. Los tokens del título principal (antes del año o etiquetas técnicas).
/// 2. Los tokens de un posible título alternativo entre paréntesis ej. `La.semilla.del.diablo.(Rosemary's.Baby)`.
/// 3. Todos los años de 4 dígitos (`1920..=2099`) presentes en el nombre de archivo.
fn parse_candidate_release_structure(candidate_title: &str) -> (Vec<String>, Vec<String>, Vec<u32>) {
    let mut raw = candidate_title.trim();
    for ext in &[".mkv", ".mp4", ".avi", ".m4v", ".ts", ".iso", ".srt"] {
        if raw.len() > ext.len() && raw[raw.len() - ext.len()..].eq_ignore_ascii_case(ext) {
            raw = &raw[..raw.len() - ext.len()];
            break;
        }
    }

    // Recopilar todos los años presentes en todo el nombre de archivo
    let full_norm = normalize_accents_and_symbols(raw);
    let all_years: Vec<u32> = full_norm
        .split_whitespace()
        .filter_map(is_year_token)
        .collect();

    // Ignorar prefijos entre paréntesis o corchetes al inicio: ej. `(Polanski).(1968).Titre...`
    let mut trimmed = raw.trim_start_matches(['.', ' ', '_', '-']);
    loop {
        if let Some(rest) = trimmed.strip_prefix('(') {
            if let Some(end) = rest.find(')') {
                let after = rest[end + 1..].trim_start_matches(['.', ' ', '_', '-']);
                if !after.is_empty() {
                    trimmed = after;
                    continue;
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                let after = rest[end + 1..].trim_start_matches(['.', ' ', '_', '-']);
                if !after.is_empty() {
                    trimmed = after;
                    continue;
                }
            }
        }
        break;
    }

    // Detectar y retirar prefijos de catálogo / categoría / director con guiones:
    // ej.: "Cine - Georges Méliès - 1902 - Viaje a la Luna (1902)..." o "+ 1902 Méliès - Le voyage dans la lune..."
    if trimmed.starts_with('+') {
        trimmed = trimmed.trim_start_matches('+').trim_start_matches(['.', ' ', '_', '-']);
    }
    if trimmed.contains(" - ") {
        let segs: Vec<&str> = trimmed.split(" - ").collect();
        if segs.len() >= 3 {
            for (idx, &seg) in segs.iter().enumerate() {
                let seg_clean = seg.trim();
                if is_year_token(seg_clean).is_some() && idx + 1 < segs.len() {
                    let offset = segs[idx + 1].as_ptr() as usize - trimmed.as_ptr() as usize;
                    trimmed = &trimmed[offset..];
                    break;
                }
            }
        }
    }

    // Separar la porción anterior al primer paréntesis/corchete
    let mut first_boundary_idx = trimmed.len();
    for (i, ch) in trimmed.char_indices() {
        if ch == '(' || ch == '[' {
            first_boundary_idx = i;
            break;
        }
    }

    let main_segment = &trimmed[..first_boundary_idx];
    let main_norm = normalize_accents_and_symbols(main_segment);
    let mut main_title_tokens = Vec::new();

    for w in main_norm.split_whitespace() {
        if is_year_token(w).is_some()
            || is_episode_marker_token(w)
            || RELEASE_METADATA_BOUNDARY_TOKENS.contains(&w)
        {
            break;
        }
        if !STOP_WORDS.contains(&w) && (w.len() > 1 || w.chars().all(|c| c.is_ascii_digit())) {
            main_title_tokens.push(w.to_string());
        }
    }

    // Extraer los tokens de título alternativo de todos los bloques entre paréntesis o corchetes
    let mut alt_title_tokens = Vec::new();
    let mut paren_groups = Vec::new();
    let mut in_paren = false;
    let mut in_bracket = false;
    let mut current_group = String::new();

    for ch in trimmed.chars() {
        if ch == '(' && !in_paren {
            in_paren = true;
            current_group.clear();
        } else if ch == ')' && in_paren {
            in_paren = false;
            paren_groups.push(current_group.clone());
            current_group.clear();
        } else if ch == '[' && !in_bracket {
            in_bracket = true;
            current_group.clear();
        } else if ch == ']' && in_bracket {
            in_bracket = false;
            paren_groups.push(current_group.clone());
            current_group.clear();
        } else if in_paren || in_bracket {
            current_group.push(ch);
        }
    }

    for group in paren_groups {
        let group_norm = normalize_accents_and_symbols(&group);
        let group_words: Vec<&str> = group_norm.split_whitespace().collect();
        let is_pure_metadata = group_words.iter().all(|&w| {
            is_year_token(w).is_some()
                || is_episode_marker_token(w)
                || RELEASE_METADATA_BOUNDARY_TOKENS.contains(&w)
                || TRUSTED_RELEASE_GROUPS.contains(&w)
                || STOP_WORDS.contains(&w)
                || matches!(
                    w,
                    "spa" | "eng" | "ita" | "fra" | "ger" | "sub" | "subs" | "dual" | "multi"
                        | "silent" | "muda" | "muto" | "color" | "restaurada"
                        | "h264" | "x264" | "h265" | "x265" | "hevc" | "ac3" | "eac3" | "5" | "1" | "7"
                )
        });
        if !is_pure_metadata {
            for w in group_words {
                let is_meta = is_year_token(w).is_some()
                    || is_episode_marker_token(w)
                    || RELEASE_METADATA_BOUNDARY_TOKENS.contains(&w)
                    || TRUSTED_RELEASE_GROUPS.contains(&w)
                    || STOP_WORDS.contains(&w)
                    || matches!(
                        w,
                        "spa" | "eng" | "ita" | "fra" | "ger" | "sub" | "subs" | "dual" | "multi"
                            | "silent" | "muda" | "muto" | "color" | "restaurada"
                            | "h264" | "x264" | "h265" | "x265" | "hevc" | "ac3" | "eac3" | "5" | "1" | "7"
                    );
                if !is_meta
                    && (w.len() > 1 || w.chars().all(|c| c.is_ascii_digit()))
                    && !alt_title_tokens.contains(&w.to_string())
                {
                    alt_title_tokens.push(w.to_string());
                }
            }
        }
    }

    (main_title_tokens, alt_title_tokens, all_years)
}

/// Devuelve todos los números de episodios de la temporada `season` cubiertos por `candidate_title`
/// (episodio simple, doble o rango multiepisodio ej.: `S01E01-E04`, `S01E01E02E03`, `1x01-03`, `Cap.101_103`).
pub fn extract_tv_episode_numbers(candidate_title: &str, season: u32) -> Vec<u32> {
    let norm = normalize_accents_and_symbols(candidate_title);
    let words: Vec<&str> = norm.split_whitespace().collect();
    let mut out = Vec::new();

    let s_pad = format!("s{season:02}");
    let s_raw = format!("s{season}");

    for (idx, &w) in words.iter().enumerate() {
        // 1. Formato SxxExx (simple, consecutivo S01E01E02E03 o rango S01E01-E04 / S01E01 al E04)
        if (w.starts_with(&s_pad) || w.starts_with(&s_raw)) && w.contains('e') {
            let s_len = if w.starts_with(&s_pad) { s_pad.len() } else { s_raw.len() };
            let ep_part = &w[s_len..];
            let ep_nums: Vec<u32> = ep_part
                .split('e')
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<u32>().ok())
                .collect();

            for &ep in &ep_nums {
                if !out.contains(&ep) {
                    out.push(ep);
                }
            }

            if let Some(&first_ep) = ep_nums.first() {
                // Comprobar si el siguiente token (o idx+2 tras "al"/"a"/"to") indica el final de un rango
                let next_candidates = if idx + 2 < words.len()
                    && matches!(words[idx + 1], "al" | "a" | "to")
                {
                    Some(words[idx + 2])
                } else if idx + 1 < words.len() {
                    Some(words[idx + 1])
                } else {
                    None
                };

                if let Some(next_raw) = next_candidates {
                    let trimmed_next = next_raw
                        .strip_prefix(&s_pad)
                        .or_else(|| next_raw.strip_prefix(&s_raw))
                        .unwrap_or(next_raw)
                        .trim_start_matches('e');
                    if !trimmed_next.is_empty()
                        && trimmed_next.len() <= 3
                        && trimmed_next.chars().all(|c| c.is_ascii_digit())
                    {
                        if let Ok(end_ep) = trimmed_next.parse::<u32>() {
                            if end_ep > first_ep && (end_ep - first_ep) <= MAX_EPISODE_RANGE_SPAN {
                                for ep in first_ep..=end_ep {
                                    if !out.contains(&ep) {
                                        out.push(ep);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Formato 1x01 o 01x01 (con rango eventual 1x01-03 o 1x01-1x03)
        let x_prefix_raw = format!("{season}x");
        let x_prefix_pad = format!("{season:02}x");
        if w.starts_with(&x_prefix_raw) || w.starts_with(&x_prefix_pad) {
            let p_len = if w.starts_with(&x_prefix_pad) {
                x_prefix_pad.len()
            } else {
                x_prefix_raw.len()
            };
            let ep_str = &w[p_len..];
            if !ep_str.is_empty() && ep_str.len() <= 3 && ep_str.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(first_ep) = ep_str.parse::<u32>() {
                    if !out.contains(&first_ep) {
                        out.push(first_ep);
                    }
                    if idx + 1 < words.len() {
                        let next_w = words[idx + 1]
                            .strip_prefix(&x_prefix_pad)
                            .or_else(|| words[idx + 1].strip_prefix(&x_prefix_raw))
                            .unwrap_or(words[idx + 1]);
                        if !next_w.is_empty()
                            && next_w.len() <= 3
                            && next_w.chars().all(|c| c.is_ascii_digit())
                        {
                            if let Ok(end_ep) = next_w.parse::<u32>() {
                                if end_ep > first_ep && (end_ep - first_ep) <= MAX_EPISODE_RANGE_SPAN {
                                    for ep in first_ep..=end_ep {
                                        if !out.contains(&ep) {
                                            out.push(ep);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Nomenclatura española: `Cap.102`, `Cap.0102`, `Capitulo 102`, `Cap.101_104`
        if (w == "cap" || w == "capitulo" || w == "episodio") && idx + 1 < words.len() {
            let code_w = words[idx + 1];
            if let Some(first_ep) = parse_spanish_cap_code(code_w, season) {
                if !out.contains(&first_ep) {
                    out.push(first_ep);
                }
                if idx + 2 < words.len() {
                    let second_w = if matches!(words[idx + 2], "al" | "a") && idx + 3 < words.len() {
                        words[idx + 3]
                    } else {
                        words[idx + 2]
                    };
                    if let Some(end_ep) = parse_spanish_cap_code(second_w, season) {
                        if end_ep > first_ep && (end_ep - first_ep) <= MAX_EPISODE_RANGE_SPAN {
                            for ep in first_ep..=end_ep {
                                if !out.contains(&ep) {
                                    out.push(ep);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    out.sort_unstable();
    out
}

fn parse_spanish_cap_code(code_w: &str, season: u32) -> Option<u32> {
    if !code_w.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let val = code_w.parse::<u32>().ok()?;
    if val >= season * 100 && val < (season + 1) * 100 {
        let ep = val - season * 100;
        if ep >= 1 {
            return Some(ep);
        }
    }
    if season == 1 && (1..100).contains(&val) && code_w.len() <= 2 {
        return Some(val);
    }
    None
}

/// Comprueba si el título candidato coincide exactamente con la temporada y el episodio esperados,
/// incluidos episodios dobles/múltiples (`S01E01-E02`, `S01E01-E04`, `1x01-02`) y la nomenclatura
/// española (`Cap.102`, `Cap.101_103`, `Capitulo 102`).
fn matches_tv_episode_marker(candidate_title: &str, season: u32, episode: u32) -> bool {
    extract_tv_episode_numbers(candidate_title, season).contains(&episode)
}

/// Comprueba si el título candidato coincide con un episodio de anime por temporada+episodio
/// o por su número absoluto (`1x36`, `036`, `- 36`, `[036]`, `Cap.36`, `E036`, `EP36`).
fn matches_anime_episode_marker(
    candidate_title: &str,
    season: u32,
    episode: u32,
    absolute_episode: Option<u32>,
) -> bool {
    if matches_tv_episode_marker(candidate_title, season, episode) {
        return true;
    }
    let Some(abs_ep) = absolute_episode else {
        return false;
    };
    if abs_ep == 0 {
        return false;
    }
    // 1. Formato español/anime frecuente en eD2K: "1x36", "1x105", "S01E36", "Cap.136"
    if matches_tv_episode_marker(candidate_title, 1, abs_ep) {
        return true;
    }

    // 2. Numeración absoluta suelta ("Naruto - 036", "Naruto 036 [1080p]", "Naruto Cap.36", "Naruto E036")
    let norm = normalize_accents_and_symbols(candidate_title);
    let words: Vec<&str> = norm.split_whitespace().collect();
    for (idx, &w) in words.iter().enumerate() {
        // Ignorar si va precedido de palabra de temporada (p. ej. "Temporada 2")
        if idx > 0 && matches!(words[idx - 1], "temporada" | "season" | "temp") {
            continue;
        }
        // Formatos con prefijo explícito: e036, ep036
        if let Some(rest) = w.strip_prefix("ep").or_else(|| w.strip_prefix('e')) {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(n) = rest.parse::<u32>() {
                    if n == abs_ep {
                        return true;
                    }
                }
            }
        }
        // Token numérico puro (excluyendo años 1900..=2099, resoluciones y códecs)
        if w.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = w.parse::<u32>() {
                if n == abs_ep
                    && !matches!(n, 264 | 265 | 480 | 576 | 720 | 1080 | 2160)
                    && !(1900..=2099).contains(&n)
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Comprueba si una versión corresponde a un paquete de temporada completa para la serie y temporada dadas.
pub fn matches_season_pack_release(
    candidate_title: &str,
    expected_title: &str,
    expected_original_title: Option<&str>,
    expected_year: Option<u32>,
    season: u32,
) -> bool {
    if !matches_series_or_movie_title_tokens(
        candidate_title,
        expected_title,
        expected_original_title,
        &[],
        expected_year,
        false,
        None,
    ) {
        return false;
    }

    let norm = normalize_accents_and_symbols(candidate_title);
    let words: Vec<&str> = norm.split_whitespace().collect();

    // Si la versión menciona un episodio aislado (ej.: S01E02 solo), no es un paquete de temporada
    let covered_eps = extract_tv_episode_numbers(candidate_title, season);
    if !covered_eps.is_empty() && covered_eps.len() < 4 {
        return false;
    }

    // Buscar un marcador explícito de la temporada `season`
    for (idx, &w) in words.iter().enumerate() {
        if is_standalone_season_token(w) == Some(season) {
            return true;
        }
        if matches!(w, "temporada" | "season" | "temp") && idx + 1 < words.len() {
            if let Ok(s_num) = words[idx + 1].parse::<u32>() {
                if s_num == season {
                    return true;
                }
            }
        }
    }

    // O un rango de al menos 4 episodios que cubra el inicio de la temporada (ej.: S01E01-E08)
    covered_eps.len() >= 4 && covered_eps.contains(&1)
}

#[allow(clippy::too_many_arguments)]
fn matches_series_or_movie_title_tokens(
    candidate_title: &str,
    expected_title: &str,
    expected_original_title: Option<&str>,
    alternative_titles: &[&str],
    expected_year: Option<u32>,
    is_movie: bool,
    absolute_episode: Option<u32>,
) -> bool {
    let (cand_main_tokens, cand_alt_tokens, cand_years) =
        parse_candidate_release_structure(candidate_title);

    let exp_tokens = clean_title_tokens(expected_title);
    let orig_tokens = expected_original_title.map(clean_title_tokens).unwrap_or_default();
    let es_variants = spanish_title_search_variants(expected_title, expected_original_title);
    let es_variant_tokens: Vec<Vec<String>> = es_variants
        .iter()
        .map(|v| clean_title_tokens(v))
        .filter(|v| !v.is_empty())
        .collect();
    let alt_token_groups: Vec<Vec<String>> = alternative_titles
        .iter()
        .map(|v| clean_title_tokens(v))
        .filter(|v| !v.is_empty())
        .collect();

    // Si el título principal u original tiene un número de secuela (p. ej. 2, 3, 4),
    // filtrar títulos alternativos que hayan perdido ese marcador (p. ej. "Dune" para "Dune 3").
    let extract_sequel_num = |tokens: &[String]| -> Option<u32> {
        for t in tokens {
            match t.as_str() {
                "1" | "one" | "i" | "uno" => return Some(1),
                "2" | "two" | "ii" | "dos" => return Some(2),
                "3" | "three" | "iii" | "tres" => return Some(3),
                "4" | "four" | "iv" | "cuatro" => return Some(4),
                _ => {}
            }
        }
        None
    };
    let expected_sequel = extract_sequel_num(&exp_tokens).or_else(|| extract_sequel_num(&orig_tokens));
    let alt_token_groups: Vec<Vec<String>> = alt_token_groups
        .into_iter()
        .filter(|at| {
            if let Some(exp_seq) = expected_sequel {
                if exp_seq >= 2 {
                    if let Some(alt_seq) = extract_sequel_num(at) {
                        return alt_seq == exp_seq;
                    }
                    // Si el título alternativo no tiene número de secuela y todos sus tokens ya están
                    // contenidos en el título esperado (p. ej. "Dune" frente a "Dune: Part Three"), descartarlo.
                    let is_subset = at.iter().all(|tok| exp_tokens.contains(tok) || orig_tokens.contains(tok));
                    return !is_subset;
                }
            }
            true
        })
        .collect();

    // Comprobación estricta de año para películas y protección frente a series homónimas antiguas
    if is_movie {
        if let Some(year) = expected_year {
            if !cand_years.is_empty() {
                if !cand_years.iter().any(|&y| y.abs_diff(year) <= 1) {
                    return false;
                }
            } else {
                let min_title_len = if !orig_tokens.is_empty() {
                    exp_tokens.len().min(orig_tokens.len())
                } else {
                    exp_tokens.len()
                };
                if min_title_len <= 2 {
                    return false;
                }
            }
        }
    } else if let Some(year) = expected_year {
        if !cand_years.is_empty() && !cand_years.iter().any(|&y| y >= year.saturating_sub(1)) {
            return false;
        }
    }

    if cand_main_tokens.is_empty() {
        return false;
    }

    let mut allowed_union: Vec<&str> = exp_tokens.iter().map(|s| s.as_str()).collect();
    for t in &orig_tokens {
        if !allowed_union.contains(&t.as_str()) {
            allowed_union.push(t.as_str());
        }
    }
    for vt in &es_variant_tokens {
        for t in vt {
            if !allowed_union.contains(&t.as_str()) {
                allowed_union.push(t.as_str());
            }
        }
    }
    for at in &alt_token_groups {
        for t in at {
            if !allowed_union.contains(&t.as_str()) {
                allowed_union.push(t.as_str());
            }
        }
    }
    // Añadir también equivalentes bilingües palabra por palabra para los tokens esperados
    let mut bilingual_additions = Vec::new();
    for &w in &allowed_union {
        for &(en_w, es_w) in EN_ES_TITLE_WORD_EQUIVALENTS {
            if w == en_w && !bilingual_additions.contains(&es_w) {
                bilingual_additions.push(es_w);
            } else if w == es_w && !bilingual_additions.contains(&en_w) {
                bilingual_additions.push(en_w);
            }
        }
    }
    for bw in bilingual_additions {
        if !allowed_union.contains(&bw) {
            allowed_union.push(bw);
        }
    }

    // Expansión de sinónimos de numeración para secuelas (ej. "part two" <-> "2", "ii", "parte", "dos")
    let mut syn_expansions = Vec::new();
    let has_two = allowed_union.iter().any(|&w| matches!(w, "2" | "two" | "ii" | "dos"));
    let has_part = allowed_union.iter().any(|&w| matches!(w, "part" | "parte"));
    if has_two {
        syn_expansions.extend_from_slice(&["2", "two", "ii", "dos"]);
        if has_part {
            syn_expansions.extend_from_slice(&["part", "parte"]);
        }
    }
    let has_three = allowed_union.iter().any(|&w| matches!(w, "3" | "three" | "iii" | "tres"));
    if has_three {
        syn_expansions.extend_from_slice(&["3", "three", "iii", "tres"]);
        if has_part {
            syn_expansions.extend_from_slice(&["part", "parte"]);
        }
    }
    for syn in syn_expansions {
        if !allowed_union.contains(&syn) {
            allowed_union.push(syn);
        }
    }

    let has_unexpected_sequel_marker = cand_main_tokens
        .iter()
        .chain(cand_alt_tokens.iter())
        .any(|cw| {
            SEQUEL_AND_SPINOFF_MARKERS.contains(&cw.as_str())
                && !allowed_union.contains(&cw.as_str())
        });
    if has_unexpected_sequel_marker {
        return false;
    }

    let matches_segment = |target_tokens: &[String], segment_tokens: &[String]| -> bool {
        if target_tokens.is_empty() || segment_tokens.is_empty() {
            return false;
        }

        let matched_count = target_tokens
            .iter()
            .filter(|t| {
                segment_tokens.iter().any(|cw| {
                    if cw == t.as_str() {
                        return true;
                    }
                    // Equivalencias numéricas
                    if matches!(t.as_str(), "2" | "two" | "ii" | "dos") && matches!(cw.as_str(), "2" | "two" | "ii" | "dos") {
                        return true;
                    }
                    if matches!(t.as_str(), "3" | "three" | "iii" | "tres") && matches!(cw.as_str(), "3" | "three" | "iii" | "tres") {
                        return true;
                    }
                    if matches!(t.as_str(), "4" | "four" | "iv" | "cuatro") && matches!(cw.as_str(), "4" | "four" | "iv" | "cuatro") {
                        return true;
                    }
                    if matches!(t.as_str(), "part" | "parte") && matches!(cw.as_str(), "part" | "parte") {
                        return true;
                    }
                    // Equivalencias bilingües inglés <-> español
                    if EN_ES_TITLE_WORD_EQUIVALENTS
                        .iter()
                        .any(|&(en_w, es_w)| {
                            (t.as_str() == en_w && cw.as_str() == es_w)
                                || (t.as_str() == es_w && cw.as_str() == en_w)
                        })
                    {
                        return true;
                    }
                    false
                })
            })
            .count();

        let recall_ok = if target_tokens.len() <= 2 {
            matched_count == target_tokens.len()
        } else {
            // Si el objetivo contiene "part"/"parte" pero el candidato lo omitió a favor de un número (ej. "Dune: Part Two" -> "Dune 2"),
            // tolerar la ausencia de la palabra "part" si el número/secuela coincidió.
            let has_target_part = target_tokens.iter().any(|t| matches!(t.as_str(), "part" | "parte"));
            let segment_has_number = segment_tokens.iter().any(|cw| cw.chars().all(|c| c.is_ascii_digit()) || matches!(cw.as_str(), "ii" | "iii"));
            if has_target_part && segment_has_number && matched_count + 1 >= target_tokens.len() {
                true
            } else {
                matched_count >= (target_tokens.len() * 4).div_ceil(5)
            }
        };
        if !recall_ok {
            return false;
        }

        let is_abs_ep_token = |cw: &str| -> bool {
            let Some(abs_ep) = absolute_episode else {
                return false;
            };
            let num_str = cw
                .strip_prefix("ep")
                .or_else(|| cw.strip_prefix('e'))
                .unwrap_or(cw);
            !num_str.is_empty()
                && num_str.chars().all(|c| c.is_ascii_digit())
                && num_str.parse::<u32>().ok() == Some(abs_ep)
        };

        // En anime con numeración absoluta (p. ej. "Naruto - 036 - Clones contra clones"), los tokens
        // posteriores al número absoluto pertenecen al título del episodio y no al título de la serie.
        let effective_segment: &[String] = if absolute_episode.is_some() {
            if let Some(abs_pos) = segment_tokens
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, cw)| is_abs_ep_token(cw))
                .map(|(idx, _)| idx)
            {
                &segment_tokens[..abs_pos]
            } else {
                segment_tokens
            }
        } else {
            segment_tokens
        };

        if target_tokens.len() <= 2 {
            let has_foreign_word = effective_segment
                .iter()
                .any(|cw| !allowed_union.contains(&cw.as_str()) && !is_abs_ep_token(cw));
            if has_foreign_word {
                return false;
            }
        } else {
            let foreign_count = effective_segment
                .iter()
                .filter(|cw| !allowed_union.contains(&cw.as_str()) && !is_abs_ep_token(cw))
                .count();
            if foreign_count > 1 {
                return false;
            }
        }

        true
    };

    let matches_strict_title = |target_tokens: &[String]| -> bool {
        matches_segment(target_tokens, &cand_main_tokens)
            || (!cand_alt_tokens.is_empty() && matches_segment(target_tokens, &cand_alt_tokens))
            || (!cand_alt_tokens.is_empty() && {
                let mut combined = cand_main_tokens.clone();
                for at in &cand_alt_tokens {
                    if !combined.contains(at) {
                        combined.push(at.clone());
                    }
                }
                matches_segment(target_tokens, &combined)
            })
    };

    if matches_strict_title(&exp_tokens) {
        return true;
    }

    if !orig_tokens.is_empty() && matches_strict_title(&orig_tokens) {
        return true;
    }

    for vt in &es_variant_tokens {
        if matches_strict_title(vt) {
            return true;
        }
    }

    for at in &alt_token_groups {
        if matches_strict_title(at) {
            return true;
        }
    }

    false
}

pub fn matches_release(
    candidate_title: &str,
    expected_title: &str,
    expected_original_title: Option<&str>,
    expected_year: Option<u32>,
    is_movie: bool,
    season_episode: Option<(u32, u32)>,
) -> bool {
    matches_release_with_alts(
        candidate_title,
        expected_title,
        expected_original_title,
        &[],
        expected_year,
        is_movie,
        season_episode,
    )
}

pub fn matches_release_with_alts(
    candidate_title: &str,
    expected_title: &str,
    expected_original_title: Option<&str>,
    alternative_titles: &[&str],
    expected_year: Option<u32>,
    is_movie: bool,
    season_episode: Option<(u32, u32)>,
) -> bool {
    matches_release_with_options(
        candidate_title,
        expected_title,
        expected_original_title,
        alternative_titles,
        expected_year,
        is_movie,
        season_episode,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn matches_release_with_options(
    candidate_title: &str,
    expected_title: &str,
    expected_original_title: Option<&str>,
    alternative_titles: &[&str],
    expected_year: Option<u32>,
    is_movie: bool,
    season_episode: Option<(u32, u32)>,
    absolute_episode: Option<u32>,
) -> bool {
    if !is_movie {
        if let Some((season, episode)) = season_episode {
            if !matches_anime_episode_marker(candidate_title, season, episode, absolute_episode) {
                return false;
            }
        }
    }

    matches_series_or_movie_title_tokens(
        candidate_title,
        expected_title,
        expected_original_title,
        alternative_titles,
        expected_year,
        is_movie,
        absolute_episode,
    )
}

