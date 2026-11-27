// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Filtro estricto de seguridad, anti-spam adulto y clasificación de cine/series/anime
//! para metadatos descubiertos de forma abierta en Mainline DHT.
//!
//! Se aplica en memoria **antes** de persistir cualquier entrada en SQLite o mostrarla en la WebUI.

/// Tamaño mínimo (50 MiB) exigido a un archivo de vídeo descubierto al azar en DHT para evitar
/// clips basura o muestras promocionales.
pub const MIN_DHT_MEDIA_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
/// Año mínimo aceptado como pista válida de estreno audiovisual en nombres de lanzamiento.
const MIN_RELEASE_YEAR: u32 = 1920;
/// Año máximo aceptado como pista válida de estreno audiovisual en nombres de lanzamiento.
const MAX_RELEASE_YEAR: u32 = 2035;

/// Subcadenas bloqueadas de forma incondicional (contenido abusivo/ilegal, spam explícito y portales adultos).
const BLOCKED_SUBSTRINGS: &[&str] = &[
    "pthc",
    "pedo",
    "paedo",
    "childporn",
    "preteen",
    "pornokind",
    "kiddy",
    "underage",
    "csam",
    "lolita",
    "jailbait",
    "incest",
    "stepsister",
    "stepmom",
    "stepdad",
    "stepbro",
    "pornhub",
    "xvideos",
    "xhamster",
    "brazzers",
    "nubiles",
    "bangbros",
    "naughtyamerica",
    "realitykings",
    "blacked",
    "tushy",
    "vixen",
    "jacquie",
    "onlyfans",
    "chaturbate",
    "stripchat",
    "manyvids",
    "camsoda",
    "myfreecams",
    "bonga",
    "evilangel",
    "kink.com",
    "fakehub",
    "mofos",
    "teamskeet",
    "metart",
    "hegre",
    "playboy",
    "penthouse",
    "dorcel",
    "private.com",
    "fc2-ppv",
    "fc2ppv",
    "caribbeancom",
    "1pondo",
    "tokyo-hot",
    "heyzo",
    "bydda.cc",
    "javhd",
    "onlytarts",
    "blowjob",
    "deepthroat",
    "creampie",
    "gangbang",
    "bukkake",
    "cumshot",
    "handjob",
    "footjob",
    "threesome",
    "foursome",
    "lesbian",
    "shemale",
    "femdom",
    "cuckold",
    "hentai",
    "ecchi",
    "gravure",
    "香蕉传媒",
    "肉体",
    "丝袜",
    "atk.natural",
    "hairy.bushes",
];

/// Tokens exactos bloqueados (comprobados tras normalizar a palabras alfanuméricas en minúsculas).
const BLOCKED_EXACT_TOKENS: &[&str] = &[
    "xxx",
    "porn",
    "porno",
    "nsfw",
    "milf",
    "bdsm",
    "jav",
    "anal",
    "pussy",
    "cock",
    "dick",
    "tits",
    "boobs",
    "ass",
    "slut",
    "whore",
    "fuck",
    "fucking",
    "fucked",
    "sex",
    "sexy",
    "nude",
    "nudes",
    "erotic",
    "erotica",
    "fetish",
    "horny",
    "orgasm",
    "masturbation",
    "camgirl",
    "webcam",
    "escort",
    "uncensored",
    "adult",
    "playmate",
    "rape",
    "cp",
];

/// Prefijos de estudios JAV frecuentes seguidos de guion/dígitos (p. ej. `SIVR-348`, `FNS-178`).
const BLOCKED_JAV_PREFIXES: &[&str] = &[
    "abp", "abs", "abw", "adn", "apns", "atid", "blk", "caud", "cawd", "cjod", "dasd", "dldss",
    "ebod", "eyan", "fc2", "fns", "fsdss", "hmn", "hnd", "ipx", "ipzz", "jul", "jur", "jux",
    "kavr", "kiwvr", "mdyd", "meyd", "miab", "miaa", "mide", "midv", "mifd", "mird", "mism",
    "mkmp", "mmnd", "mogi", "mopp", "mudr", "mvg", "nacr", "nkkd", "npjs", "nsfs", "ofje",
    "pfes", "pgd", "pkpl", "pppd", "pppe", "pred", "rbd", "royd", "sdab", "sdde", "sdmu", "sdnm", "shkd",
    "sivr", "snis", "ssis", "ssni", "sone", "star", "stars", "stko", "tek", "ure", "venx",
    "vrkm", "vrtm", "waaa", "wanf", "wanz", "xjx", "ymdd",
];

/// Tokens positivos característicos de lanzamientos legítimos de cine, series y anime.
const MEDIA_RELEASE_SIGNALS: &[&str] = &[
    "2160p",
    "1080p",
    "720p",
    "480p",
    "4k",
    "uhd",
    "bluray",
    "bdrip",
    "brrip",
    "remux",
    "webrip",
    "webdl",
    "hdtv",
    "dvdrip",
    "x264",
    "x265",
    "h264",
    "h265",
    "hevc",
    "av1",
    "xvid",
    "aac",
    "ac3",
    "dts",
    "eac3",
    "ddp5",
    "dd5",
    "atmos",
    "hdr",
    "hdr10",
    "dv",
    "dual",
    "multi",
    "spanish",
    "castellano",
    "latino",
    "español",
    "sub",
    "subs",
    "subtitulado",
    "vose",
    "season",
    "temporada",
    "complete",
    "completa",
    "episode",
    "episodio",
    "anime",
    "ova",
    "netflix",
    "amzn",
    "nf",
    "dsnp",
    "atvp",
    "hmax",
    "criterion",
];

/// Evalúa si un texto contiene cualquier indicio abusivo, ilegal o de spam adulto (bloqueo duro).
pub fn is_blocked_abusive_or_adult(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if BLOCKED_SUBSTRINGS.iter().any(|s| lower.contains(s)) {
        return true;
    }

    let tokens = split_alnum_tokens(&lower);
    for (idx, tok) in tokens.iter().enumerate() {
        if BLOCKED_EXACT_TOKENS.contains(&tok.as_str()) {
            return true;
        }
        // Detectar códigos JAV del tipo `SIVR-348` / `FNS-178` / `YMDD501`.
        if is_jav_catalog_code(tok, tokens.get(idx + 1).map(String::as_str)) {
            return true;
        }
    }
    false
}

/// Comprueba si un torrent de vídeo es seguro (pasa el bloqueo duro) y corresponde a un
/// lanzamiento legítimo de cine, series o anime apto para el catálogo DHT general.
pub fn is_clean_catalog_video(torrent_name: &str, file_path: &str, file_size: u64) -> bool {
    if is_blocked_abusive_or_adult(torrent_name) || is_blocked_abusive_or_adult(file_path) {
        return false;
    }
    if file_size < MIN_DHT_MEDIA_VIDEO_BYTES {
        return false;
    }
    has_media_release_signal(torrent_name) || has_media_release_signal(file_path)
}

/// Señales de afinidad hispana/española para priorizar vecindades DHT relevantes para el usuario.
const SPANISH_AFFINITY_TOKENS: &[&str] = &[
    "spanish",
    "castellano",
    "español",
    "espanol",
    "latino",
    "vose",
    "esp",
    "spa",
    "dual",
    "subtitulado",
    "temporada",
    "completa",
    "capitulo",
    "episodio",
    "pelicula",
    "anime",
];

/// Puntuación máxima de afinidad de idioma/vecindad devuelta por [`compute_language_affinity_score`].
const MAX_AFFINITY_SCORE: u8 = 100;
/// Puntos sumados por cada señal de idioma español/dual detectada en el nombre del torrent o archivo.
const AFFINITY_POINTS_PER_SIGNAL: u8 = 35;

/// Calcula una puntuación de afinidad (`0..=100`) indicando si el torrent corresponde al idioma
/// español/dual o a series/anime de interés para priorizar esa vecindad en el Sabueso DHT.
pub fn compute_language_affinity_score(torrent_name: &str, file_path: &str) -> u8 {
    let combined = format!(
        "{} {}",
        torrent_name.to_ascii_lowercase(),
        file_path.to_ascii_lowercase()
    );
    let mut score: u8 = 0;
    if combined.contains("es-es") || combined.contains("es-la") || combined.contains("spa-eng") {
        score = score.saturating_add(AFFINITY_POINTS_PER_SIGNAL);
    }
    for tok in split_alnum_tokens(&combined) {
        if SPANISH_AFFINITY_TOKENS.contains(&tok.as_str()) {
            score = score.saturating_add(AFFINITY_POINTS_PER_SIGNAL);
            if score >= MAX_AFFINITY_SCORE {
                return MAX_AFFINITY_SCORE;
            }
        }
    }
    score.min(MAX_AFFINITY_SCORE)
}

/// Detecta señales claras de lanzamiento de cine, series o anime (calidad, códec, patrón SxxExx/NxNN o año).
pub fn has_media_release_signal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("web-dl") || lower.contains("blu-ray") {
        return true;
    }
    let tokens = split_alnum_tokens(&lower);
    for tok in &tokens {
        if MEDIA_RELEASE_SIGNALS.contains(&tok.as_str()) {
            return true;
        }
        if is_episode_marker(tok) || is_release_year(tok) {
            return true;
        }
    }
    false
}

fn split_alnum_tokens(lower: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            cur.push(ch);
        } else if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn is_jav_catalog_code(tok: &str, next: Option<&str>) -> bool {
    // Forma separada: `sivr` seguido de `348` o `348a`.
    if BLOCKED_JAV_PREFIXES.contains(&tok) {
        if let Some(n) = next {
            let digits = n.bytes().take_while(u8::is_ascii_digit).count();
            if (3..=5).contains(&digits) {
                return true;
            }
        }
    }
    // Forma pegada: `ymdd501j`, `ssis123`.
    for &prefix in BLOCKED_JAV_PREFIXES {
        if let Some(rest) = tok.strip_prefix(prefix) {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if (3..=5).contains(&digits) && rest.len() <= digits + 2 {
                return true;
            }
        }
    }
    false
}

fn is_episode_marker(tok: &str) -> bool {
    // S01E01, S1E12, S01
    if let Some(rest) = tok.strip_prefix('s') {
        let s_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=2).contains(&s_digits) {
            let after_s = &rest[s_digits..];
            if let Some(after_e) = after_s.strip_prefix('e') {
                let e_digits = after_e.bytes().take_while(u8::is_ascii_digit).count();
                if (1..=4).contains(&e_digits) && e_digits == after_e.len() {
                    return true;
                }
            }
        }
    }
    // 1x01, 01x015
    if let Some(x_pos) = tok.find('x') {
        let left = &tok[..x_pos];
        let right = &tok[x_pos + 1..];
        if (1..=2).contains(&left.len())
            && (2..=4).contains(&right.len())
            && left.bytes().all(|b| b.is_ascii_digit())
            && right.bytes().all(|b| b.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

fn is_release_year(tok: &str) -> bool {
    tok.len() == 4
        && tok.bytes().all(|b| b.is_ascii_digit())
        && tok
            .parse::<u32>()
            .is_ok_and(|y| (MIN_RELEASE_YEAR..=MAX_RELEASE_YEAR).contains(&y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_abusive_adult_and_jav_spam_unconditionally() {
        assert!(is_blocked_abusive_or_adult("SIVR-348-1.mp4"));
        assert!(is_blocked_abusive_or_adult("FNS-178.mp4"));
        assert!(is_blocked_abusive_or_adult("bydda.cc@YMDD501J.mp4"));
        assert!(is_blocked_abusive_or_adult(
            "[Nubiles-Porn] Sample Clip (2025).mp4"
        ));
        assert!(is_blocked_abusive_or_adult("bad pthc spam.mpg"));
        assert!(!is_clean_catalog_video(
            "SIVR-348.1080p.WEB-DL",
            "SIVR-348.mp4",
            2_000_000_000
        ));
    }

    #[test]
    fn accepts_clean_movies_series_and_anime_and_rejects_tiny_clips() {
        assert!(is_clean_catalog_video(
            "Naruto.Complete.001-220.1080p.BluRay.Dual",
            "Naruto - 001.mkv",
            350 * 1024 * 1024
        ));
        assert!(is_clean_catalog_video(
            "American.Dad.S15E06.720p.AMZN.WEB-DL.mkv",
            "American.Dad.S15E06.720p.AMZN.WEB-DL.mkv",
            260 * 1024 * 1024
        ));
        assert!(is_clean_catalog_video(
            "Dune.Part.Two.2024.mkv",
            "Dune.Part.Two.2024.mkv",
            4_500_000_000
        ));
        // Archivo demasiado pequeño (< 50 MiB) o sin señales de cine/series se descarta del catálogo general.
        assert!(!is_clean_catalog_video(
            "Dune.Part.Two.2024.1080p.mkv",
            "sample.mkv",
            12 * 1024 * 1024
        ));
        assert!(!is_clean_catalog_video(
            "camera_backup_001",
            "IMG_9912.mp4",
            200 * 1024 * 1024
        ));
    }

    #[test]
    fn scores_spanish_and_dual_audio_affinity_higher_than_unrelated_regions() {
        assert!(
            compute_language_affinity_score(
                "Naruto.Temporada.1.Completa.Castellano.1080p",
                "Naruto.001.Dual.mkv"
            ) >= 70
        );
        assert_eq!(
            compute_language_affinity_score("Vstat.na.nogi.S01.2025.HEVC", "E01.mkv"),
            0
        );
    }
}
