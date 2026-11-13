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

const FORBIDDEN_EXTENSIONS: &[&str] = &[
    ".exe", ".scr", ".bat", ".cmd", ".vbs", ".zip", ".rar", ".7z", ".iso", ".tar", ".gz",
];

/// Groupes et uploaders de confiance connus sur eD2K et Torznab hispanophones.
const TRUSTED_RELEASE_GROUPS: &[&str] = &[
    "nara", "geot", "bryan_122", "bryan122", "eth@n", "ethan", "yamil", "hdo", "cartmangold",
    "caloriana", "krom", "mony2007", "zapiron", "pirolo", "wolfmax4k", "pctfenix", "dontorrent",
    "grantorrent", "divxtotal", "atomixhq", "depechemode13", "grupots", "grupohds", "sharerip",
    "parip", "hispashare", "hispamula", "forosfreaky", "nocturniap2p", "latabernadelcangrejo",
    "mokesky", "piter332", "kowalski", "sitherkiller", "oli3112", "byp58", "ercius", "yumand22",
    "rubesp",
];

/// Traductions espagnoles courantes de mots fréquents dans les titres de séries/films anglophones.
const EN_ES_TITLE_WORD_EQUIVALENTS: &[(&str, &str)] = &[
    ("station", "estacion"),
    ("eleven", "once"),
    ("11", "once"),
    ("twelve", "doce"),
    ("12", "doce"),
    ("thirteen", "trece"),
    ("13", "trece"),
    ("ten", "diez"),
    ("nine", "nueve"),
    ("eight", "ocho"),
    ("seven", "siete"),
    ("six", "seis"),
    ("five", "cinco"),
    ("four", "cuatro"),
    ("three", "tres"),
    ("two", "dos"),
    ("one", "uno"),
    ("first", "primero"),
    ("last", "ultimo"),
    ("house", "casa"),
    ("dragon", "dragon"),
    ("game", "juego"),
    ("thrones", "tronos"),
    ("ring", "anillo"),
    ("rings", "anillos"),
    ("lord", "senor"),
    ("monsieur", "senor"),
    ("mister", "senor"),
    ("mr", "senor"),
    ("flowers", "flores"),
    ("fleurs", "flores"),
    ("flower", "flor"),
    ("fleur", "flor"),
    ("koran", "coran"),
    ("quran", "coran"),
    ("power", "poder"),
    ("black", "negro"),
    ("mirror", "espejo"),
    ("white", "blanco"),
    ("lotus", "loto"),
    ("red", "rojo"),
    ("blue", "azul"),
    ("green", "verde"),
    ("golden", "dorado"),
    ("dark", "oscuro"),
    ("light", "luz"),
    ("night", "noche"),
    ("day", "dia"),
    ("dead", "muertos"),
    ("walking", "vivientes"),
    ("fear", "temor"),
    ("war", "guerra"),
    ("worlds", "mundos"),
    ("world", "mundo"),
    ("new", "nuevo"),
    ("old", "viejo"),
    ("man", "hombre"),
    ("men", "hombres"),
    ("woman", "mujer"),
    ("women", "mujeres"),
    ("girl", "chica"),
    ("girls", "chicas"),
    ("boy", "chico"),
    ("boys", "chicos"),
    ("king", "rey"),
    ("queen", "reina"),
    ("prince", "principe"),
    ("princess", "princesa"),
    ("blood", "sangre"),
    ("fire", "fuego"),
    ("ice", "hielo"),
    ("water", "agua"),
    ("earth", "tierra"),
    ("sun", "sol"),
    ("moon", "luna"),
    ("star", "estrella"),
    ("stars", "estrellas"),
    ("city", "ciudad"),
    ("town", "pueblo"),
    ("island", "isla"),
    ("river", "rio"),
    ("mountain", "montana"),
    ("forest", "bosque"),
    ("sea", "mar"),
    ("space", "espacio"),
    ("time", "tiempo"),
    ("wheel", "rueda"),
    ("love", "amor"),
    ("death", "muerte"),
    ("life", "vida"),
    ("secret", "secreto"),
    ("secrets", "secretos"),
    ("lost", "perdidos"),
    ("found", "encontrados"),
    ("family", "familia"),
    ("brothers", "hermanos"),
    ("sisters", "hermanas"),
    ("father", "padre"),
    ("mother", "madre"),
    ("son", "hijo"),
    ("sons", "hijos"),
    ("daughter", "hija"),
    ("who", "quien"),
    ("who", "kien"),
    ("i", "yo"),
    ("me", "mi"),
    ("my", "mi"),
    ("you", "tu"),
    ("trip", "viaje"),
    ("journey", "viaje"),
    ("voyage", "viaje"),
    ("moon", "lune"),
    ("seal", "foca"),
    ("seals", "focas"),
    ("island", "isla"),
    ("isle", "isla"),
    ("selkie", "foca"),
    ("little", "pequenas"),
    ("big", "gran"),
    ("lies", "mentiras"),
    ("true", "true"),
    ("detective", "detective"),
    ("american", "americana"),
    ("story", "historia"),
    ("horror", "horror"),
    ("crime", "crimen"),
    ("law", "ley"),
    ("order", "orden"),
    ("doctor", "doctor"),
    ("good", "buen"),
    ("bad", "malo"),
    ("breaking", "breaking"),
    ("stranger", "stranger"),
    ("things", "things"),
    ("witch", "bruja"),
    ("witcher", "brujo"),
    ("shadow", "sombra"),
    ("bone", "hueso"),
    ("wheel", "rueda"),
    ("foundation", "fundacion"),
    ("invincible", "invencible"),
    ("preacher", "predicador"),
    ("leftovers", "leftovers"),
    ("watchmen", "watchmen"),
    ("succession", "sucesion"),
    ("severance", "separacion"),
    ("yellowjackets", "yellowjackets"),
    ("fargo", "fargo"),
    ("mindhunter", "mindhunter"),
    ("ozark", "ozark"),
    ("narcos", "narcos"),
    ("chernobyl", "chernobil"),
];

/// Traductions officielles espagnoles de séries TV dont le titre en Espagne diverge de l'anglais.
const KNOWN_SERIES_SPANISH_TITLES: &[(&str, &[&str])] = &[
    ("station eleven", &["Estación Once", "Estacion Once"]),
    ("house of the dragon", &["La casa del dragón", "La casa del dragon"]),
    ("the lord of the rings the rings of power", &["El Señor de los Anillos Los Anillos de Poder", "Los Anillos de Poder"]),
    ("game of thrones", &["Juego de tronos"]),
    ("the wheel of time", &["La rueda del tiempo"]),
    ("the handmaids tale", &["El cuento de la criada"]),
    ("the walking dead", &["The Walking Dead", "Los muertos vivientes"]),
    ("the last of us", &["The Last of Us", "El último de nosotros"]),
    ("severance", &["Separación", "Separacion"]),
    ("foundation", &["Fundación", "Fundacion"]),
    ("three body problem", &["El problema de los 3 cuerpos", "El problema de los tres cuerpos"]),
    ("3 body problem", &["El problema de los 3 cuerpos", "El problema de los tres cuerpos"]),
    ("mobland", &["Tierra de mafiosos"]),
    ("lanterns", &["Linternas"]),
    ("what we do in the shadows", &["Lo que hacemos en las sombras"]),
    ("only murders in the building", &["Solo asesinatos en el edificio"]),
    ("the white lotus", &["The White Lotus", "El loto blanco"]),
    ("the bear", &["The Bear", "El oso"]),
    ("all creatures great and small", &["Todas las criaturas grandes y pequeñas"]),
    ("slow horses", &["Slow Horses", "Caballos lentos"]),
    ("for all mankind", &["Para toda la humanidad"]),
    ("see", &["See"]),
    ("silo", &["Silo"]),
    ("pantheon", &["Pantheon"]),
    ("dark", &["Dark"]),
    ("monsieur ibrahim", &["El señor Ibrahim y las flores del Corán", "El senor Ibrahim y las flores del Coran"]),
    ("monsieur ibrahim et les fleurs du coran", &["El señor Ibrahim y las flores del Corán", "El senor Ibrahim y las flores del Coran"]),
    ("a touch of spice", &["Un toque de canela"]),
    ("politiki kouzina", &["Un toque de canela"]),
    ("song of the sea", &["La canción del mar", "La cancion del mar"]),
    ("the secret of roan inish", &["El secreto de la isla de las focas", "El secreto de Roan Inish"]),
    ("roan inish", &["El secreto de la isla de las focas"]),
    ("a trip to the moon", &["El viaje a la Luna", "Viaje a la Luna"]),
    ("le voyage dans la lune", &["El viaje a la Luna", "Viaje a la Luna"]),
    ("ali g who iz i", &["Ali G Kien es yo", "Ali G Quien soy yo", "Ali G"]),
];

/// Génère les variantes de titres espagnols pour une série ou un film afin de trouver les releases hispanophones.
pub fn spanish_title_search_variants(title: &str, original_title: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut push_unique = |cand: String| {
        let trimmed = cand.trim().to_string();
        if !trimmed.is_empty()
            && !trimmed.eq_ignore_ascii_case(title)
            && !original_title.map(|o| trimmed.eq_ignore_ascii_case(o)).unwrap_or(false)
            && !out.iter().any(|x: &String| x.eq_ignore_ascii_case(&trimmed))
        {
            out.push(trimmed);
        }
    };

    for src in [Some(title), original_title].into_iter().flatten() {
        let norm = normalize_accents_and_symbols(src);
        let joined = norm.split_whitespace().collect::<Vec<_>>().join(" ");
        for &(en_key, es_list) in KNOWN_SERIES_SPANISH_TITLES {
            if joined == en_key {
                for &es_t in es_list {
                    push_unique(es_t.to_string());
                }
            }
        }

        // Traduction mot à mot si tous les mots significatifs ont un équivalent espagnol direct
        let tokens = clean_title_tokens(src);
        if !tokens.is_empty() && tokens.len() <= 6 {
            let mut translated = Vec::with_capacity(tokens.len());
            let mut any_changed = false;
            let mut all_mapped = true;
            for t in &tokens {
                if let Some(&(_, es_w)) = EN_ES_TITLE_WORD_EQUIVALENTS
                    .iter()
                    .find(|&&(en_w, _)| en_w == t.as_str())
                {
                    if es_w != t.as_str() {
                        any_changed = true;
                    }
                    translated.push(es_w);
                } else {
                    all_mapped = false;
                    break;
                }
            }
            if all_mapped && any_changed {
                push_unique(translated.join(" "));
            }
        }
    }

    out
}

/// Seuils de taille optimale ("Sweet-Spot" en Mo) pour équilibrer qualité d'image et stockage.
const MOVIE_1080P_SWEET_MIN_MB: u64 = 1800;
const MOVIE_1080P_SWEET_MAX_MB: u64 = 9500;
const MOVIE_1080P_MEDIUM_MAX_MB: u64 = 16000;
const MOVIE_OVERSIZED_THRESHOLD_MB: u64 = 25000;
const EPISODE_SWEET_MIN_MB: u64 = 350;
const EPISODE_SWEET_MAX_MB: u64 = 2200;
const SEASON_PACK_SWEET_MIN_MB: u64 = 2500;
const SEASON_PACK_SWEET_MAX_MB: u64 = 28000;
const MAX_EPISODE_RANGE_SPAN: u32 = 30;

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

    // 1. Anti-Fake & Dangerous Extension Check
    for &ext in FORBIDDEN_EXTENSIONS {
        if lower.ends_with(ext) || lower.contains(&format!("{ext}.")) {
            is_rejected = true;
            rejection_reason = Some(format!("Forbidden extension '{ext}' detected (fake/malware)"));
            break;
        }
    }

    // 1b. Sample / Trailer / Teaser detection
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

    // 2. Minimum & Maximum Credible Size
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
        35 * 1024 * 1024 // 35 MB pour court-métrage ou cinéma muet des origines (ex. Le Voyage dans la Lune 1902)
    } else if effective_is_movie {
        if pref_lower.contains("4k") || pref_lower.contains("2160p") {
            1800 * 1024 * 1024 // 1.8 GB minimum pour film 4K
        } else if pref_lower.contains("1080p") {
            450 * 1024 * 1024 // 450 MB minimum pour film 1080p (évite de rejeter les encodages 649 MB / microHD)
        } else {
            250 * 1024 * 1024 // 250 MB minimum pour film standard (SD/720p)
        }
    } else if is_likely_season_pack {
        900 * 1024 * 1024 // 900 MB minimum for a full season pack
    } else if pref_lower.contains("1080p") {
        180 * 1024 * 1024 // 180 MB minimum for 1080p TV episode
    } else {
        40 * 1024 * 1024 // 40 MB for TV episode
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

    // 3. Known Fake Keywords
    for bad in &[
        "codec", "password", "contraseña", "crack", "keygen", "virus",
        "setup.exe", "install.exe", "wmv-codec", "drm-protected", "password.txt"
    ] {
        if lower.contains(bad) && !is_rejected {
            is_rejected = true;
            rejection_reason = Some(format!("Suspicious keyword '{bad}' detected"));
            break;
        }
    }

    // 4. CAM / TeleSync Penalty or Rejection
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

    // 5. Quality Detection
    let quality = if lower.contains("2160p") || lower.contains("4k") || lower.contains("uhd") {
        "2160p".to_string()
    } else if lower.contains("1080p")
        || lower.contains("1080i")
        || lower.contains(".1080.")
        || lower.contains(" 1080 ")
        || lower.contains("fhd")
        || lower.contains("bluray")
        || lower.contains("bdrip")
        || lower.contains("web-dl")
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

    // 6. Language Detection (strict Castellano / Dual / VOSE vs Latino / Foreign / pure English)
    let norm_words: Vec<String> = normalize_accents_and_symbols(title)
        .split_whitespace()
        .map(String::from)
        .collect();
    let has_word = |w: &str| norm_words.iter().any(|nw| nw == w);

    let has_trusted_spanish_group =
        TRUSTED_RELEASE_GROUPS.iter().any(|&grp| lower.contains(grp))
            || indexer_name.eq_ignore_ascii_case("dontorrent")
            || indexer_name.eq_ignore_ascii_case("grantorrent")
            || indexer_name.eq_ignore_ascii_case("divxtotal")
            || indexer_name.eq_ignore_ascii_case("atomixhq")
            || indexer_name.eq_ignore_ascii_case("pctfenix");

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

    // 7. Sources / Seeders Bonus (up to 40 pts)
    score += (sources.min(20) * 2) as i32;

    // 8. BitTorrent Protocol Speed Bonus (+10 pts if high seeders)
    if protocol == "torrent" && sources >= 5 {
        score += 10;
    }

    // 9. Modern Space-Saving Codec Bonus (x265 / HEVC / AV1)
    if lower.contains("x265") || lower.contains("h265") || lower.contains("hevc") || lower.contains("av1") {
        score += 25;
    }

    // 10. Multi-channel Audio Quality Bonus (5.1, DTS, Atmos, TrueHD, AC3, EAC3)
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

    // 11. Sweet-Spot Bitrate / Size Curve (optimisé pour Raspberry Pi / Jellyfin Direct Play)
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

    // 12. Trusted Uploader / Release Group Bonus (+20 pts)
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

const STOP_WORDS: &[&str] = &[
    "de", "la", "el", "los", "las", "un", "una", "unos", "unas", "y", "o", "en", "del", "al",
    "the", "a", "an", "and", "or", "in", "on", "of", "to", "for", "with", "by", "at",
];

/// Jetons techniques/qualité/langue qui marquent la fin du titre d'une release P2P.
const RELEASE_METADATA_BOUNDARY_TOKENS: &[&str] = &[
    "1080p", "m1080p", "1080i", "720p", "m720p", "2160p", "m2160p", "480p", "576p", "4k", "uhd", "fhd", "hd", "sd",
    "bluray", "bdrip", "brrip", "microhd", "hdrip", "webrip", "webdl", "web", "dvdrip",
    "hdtv", "tvrip", "remux", "bdremux", "uhdrip", "uhdremux", "x264", "h264", "x265",
    "h265", "hevc", "av1", "xvid", "divx", "ac3", "eac3", "dts", "truehd", "atmos", "aac",
    "mp3", "flac", "hdr", "hdr10", "dv", "dolby", "ddp", "ddp5", "10bit", "10b", "8bit", "spanish",
    "espanol", "español", "castellano", "latino", "english", "eng", "spa", "esp", "dual",
    "multi", "tri", "vose", "subs", "sub", "subtitulado", "proper", "repack", "extended",
    "remastered", "unrated", "directors", "open", "matte", "imax", "criterion", "cap",
    "capitulo", "temporada", "season", "completa", "complete", "pack", "episodio", "episodios",
];

/// Marqueurs de suites ou de sous-titres de franchise qui ne doivent jamais apparaître
/// dans le titre candidat s'ils ne figurent pas dans le titre attendu.
const SEQUEL_AND_SPINOFF_MARKERS: &[&str] = &[
    "2", "3", "4", "5", "6", "7", "8", "9", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix",
    "part", "parte", "vol", "volume", "volumen", "chapter", "capitulo", "returns", "vuelve",
    "begins", "forever", "robin", "dark", "knight", "rises", "superman", "dawn", "justice",
    "resurrection", "reloaded", "revolutions", "revenge", "origins", "awakens", "apocalypse",
    "covenant", "romulus", "requiem", "predator", "salvation", "genisys", "fate", "bloodlines",
    "afterlife", "frozen", "empire", "kingdom", "conquest", "battle", "beneath", "escape",
];

pub fn clean_title_tokens(title: &str) -> Vec<String> {
    let normalized = normalize_accents_and_symbols(title);
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

/// Répare les séquences UTF-8 doublement encodées en Latin-1 (mojibake fréquent sur eD2K/Kad,
/// ex: `El seÃ±or` -> `El señor`, `InmolaciÃ³n` -> `Inmolación`, `EstaciÃÂ³n` -> `Estación`).
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

fn is_year_token(w: &str) -> Option<u32> {
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
    // Format s01e01 ou s1e1
    if bytes.first() == Some(&b's') && bytes.len() >= 4 && w[1..].contains('e') {
        let parts: Vec<&str> = w[1..].split('e').collect();
        if parts.len() >= 2
            && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            return true;
        }
    }
    // Format 1x01
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

fn has_season_pack_indicator(lower: &str) -> bool {
    lower.contains("temporada")
        || lower.contains("season")
        || lower.contains("completa")
        || lower.contains("complete")
        || lower.contains("pack")
}

/// Extrait d'un nom de fichier P2P :
/// 1. Les jetons du titre principal (avant l'année ou les tags techniques).
/// 2. Les jetons d'un éventuel titre alternatif entre parenthèses ex. `La.semilla.del.diablo.(Rosemary's.Baby)`.
/// 3. Toutes les années à 4 chiffres (`1920..=2099`) présentes dans le nom de fichier.
fn parse_candidate_release_structure(candidate_title: &str) -> (Vec<String>, Vec<String>, Vec<u32>) {
    let mut raw = candidate_title.trim();
    for ext in &[".mkv", ".mp4", ".avi", ".m4v", ".ts", ".iso", ".srt"] {
        if raw.len() > ext.len() && raw[raw.len() - ext.len()..].eq_ignore_ascii_case(ext) {
            raw = &raw[..raw.len() - ext.len()];
            break;
        }
    }

    // Collecter toutes les années présentes dans l'ensemble du nom de fichier
    let full_norm = normalize_accents_and_symbols(raw);
    let all_years: Vec<u32> = full_norm
        .split_whitespace()
        .filter_map(is_year_token)
        .collect();

    // Ignorer les préfixes entre parenthèses ou crochets au tout début : ex. `(Polanski).(1968).Titre...`
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

    // Détecter et retirer les préfixes catalogue / catégorie / réalisateur avec tirets :
    // ex: "Cine - Georges Méliès - 1902 - Viaje a la Luna (1902)..." ou "+ 1902 Méliès - Le voyage dans la lune..."
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

    // Séparer la portion avant la première parenthèse/crochet
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

    // Extraire les jetons de titre alternatif depuis tous les blocs entre parenthèses ou crochets
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

/// Retourne tous les numéros d'épisodes de la saison `season` couverts par `candidate_title`
/// (épisode simple, double ou plage multi-épisodes ex: `S01E01-E04`, `S01E01E02E03`, `1x01-03`, `Cap.101_103`).
pub fn extract_tv_episode_numbers(candidate_title: &str, season: u32) -> Vec<u32> {
    let norm = normalize_accents_and_symbols(candidate_title);
    let words: Vec<&str> = norm.split_whitespace().collect();
    let mut out = Vec::new();

    let s_pad = format!("s{season:02}");
    let s_raw = format!("s{season}");

    for (idx, &w) in words.iter().enumerate() {
        // 1. Format SxxExx (simple, collé S01E01E02E03 ou plage S01E01-E04 / S01E01 al E04)
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
                // Regarder si le jeton suivant (ou idx+2 après "al"/"a"/"to") indique la fin d'une plage
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

        // 2. Format 1x01 ou 01x01 (avec plage éventuelle 1x01-03 ou 1x01-1x03)
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

        // 3. Nomenclature espagnole : `Cap.102`, `Cap.0102`, `Capitulo 102`, `Cap.101_104`
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

/// Vérifie si le titre candidat correspond exactement à la saison et à l'épisode attendus,
/// y compris les épisodes doubles/multiples (`S01E01-E02`, `S01E01-E04`, `1x01-02`) et la nomenclature
/// espagnole (`Cap.102`, `Cap.101_103`, `Capitulo 102`).
fn matches_tv_episode_marker(candidate_title: &str, season: u32, episode: u32) -> bool {
    extract_tv_episode_numbers(candidate_title, season).contains(&episode)
}

/// Vérifie si une release correspond à un Pack de Saison complète pour la série et la saison données.
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
    ) {
        return false;
    }

    let norm = normalize_accents_and_symbols(candidate_title);
    let words: Vec<&str> = norm.split_whitespace().collect();

    // Si la release mentionne un épisode isolé (ex: S01E02 seul), ce n'est pas un pack de saison
    let covered_eps = extract_tv_episode_numbers(candidate_title, season);
    if !covered_eps.is_empty() && covered_eps.len() < 4 {
        return false;
    }

    // Chercher un marqueur explicite de la saison `season`
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

    // Ou une plage d'au moins 4 épisodes couvrant le début de la saison (ex: S01E01-E08)
    covered_eps.len() >= 4 && covered_eps.contains(&1)
}

fn matches_series_or_movie_title_tokens(
    candidate_title: &str,
    expected_title: &str,
    expected_original_title: Option<&str>,
    alternative_titles: &[&str],
    expected_year: Option<u32>,
    is_movie: bool,
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

    // Strict Year check for movies
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
    // Ajouter aussi les équivalents bilingues mot à mot pour les jetons attendus
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

    // Expansion des synonymes de numérotation pour les suites (ex. "part two" <-> "2", "ii", "parte", "dos")
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
                    // Équivalences numériques
                    if matches!(t.as_str(), "2" | "two" | "ii" | "dos") && matches!(cw.as_str(), "2" | "two" | "ii" | "dos") {
                        return true;
                    }
                    if matches!(t.as_str(), "part" | "parte") && matches!(cw.as_str(), "part" | "parte") {
                        return true;
                    }
                    // Équivalences bilingues anglais <-> espagnol
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
            // Si la cible contient "part"/"parte" mais que le candidat l'a omis au profit d'un chiffre (ex. "Dune: Part Two" -> "Dune 2"),
            // tolérer l'absence du mot "part" si le chiffre/suite a été matché.
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

        if target_tokens.len() <= 2 {
            let has_foreign_word = segment_tokens
                .iter()
                .any(|cw| !allowed_union.contains(&cw.as_str()));
            if has_foreign_word {
                return false;
            }
        } else {
            let foreign_count = segment_tokens
                .iter()
                .filter(|cw| !allowed_union.contains(&cw.as_str()))
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
    if !is_movie {
        if let Some((season, episode)) = season_episode {
            if !matches_tv_episode_marker(candidate_title, season, episode) {
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batman_1989_rejects_dark_knight_and_sequels() {
        assert!(!matches_release(
            "Batman.The.Dark.Knight.(Spanish.English.Subs).UHD.4K.HEVC-AC3.by.Geot.mkv",
            "Batman",
            Some("Batman"),
            Some(1989),
            true,
            None,
        ));
        assert!(!matches_release(
            "Batman.The.Dark.Knight.(2008).(Spanish.English.Subs).1080p.mkv",
            "Batman",
            Some("Batman"),
            Some(1989),
            true,
            None,
        ));
        assert!(!matches_release(
            "Batman.Returns.(1992).(Spanish.English.Subs).BDrip.1080p.HDR.HEVC.10b-AC3.by.nara.mkv",
            "Batman",
            Some("Batman"),
            Some(1989),
            true,
            None,
        ));
        assert!(!matches_release(
            "The.Batman.(2022).1080p.Castellano.mkv",
            "Batman",
            Some("Batman"),
            Some(1989),
            true,
            None,
        ));

        assert!(matches_release(
            "Batman.(1989).(Spanish.English.Subs).BDrip.1080p.HEVC-AC3.mkv",
            "Batman",
            Some("Batman"),
            Some(1989),
            true,
            None,
        ));
        assert!(matches_release(
            "(Tim.Burton).(1989).Batman.(Spanish.English.Subs).BDrip.1080p.mkv",
            "Batman",
            Some("Batman"),
            Some(1989),
            true,
            None,
        ));
    }

    #[test]
    fn test_sequel_and_bilingual_releases_match_correctly() {
        assert!(matches_release(
            "Batman.Returns.(1992).(Spanish.English.Subs).BDrip.1080p.HDR.HEVC.10b-AC3.by.nara.mkv",
            "Batman vuelve",
            Some("Batman Returns"),
            Some(1992),
            true,
            None,
        ));
        assert!(!matches_release(
            "Batman.(1989).(Spanish.English.Subs).BDrip.1080p.mkv",
            "Batman Returns",
            Some("Batman Returns"),
            Some(1992),
            true,
            None,
        ));

        assert!(matches_release(
            "(Polanski).(1968).La.semilla.del.diablo.(Rosemary's.Baby).(Spanish.English.Subs).BDrip.1080p.HEVC.mkv",
            "Rosemary's Baby",
            Some("Rosemary's Baby"),
            Some(1968),
            true,
            None,
        ));

        // Dune Part Two / Dune Parte Dos
        assert!(matches_release(
            "Dune.Parte.Dos.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HDR.H.265-KWK.mkv",
            "Dune: Part Two",
            Some("Dune: Part Two"),
            Some(2024),
            true,
            None,
        ));
        assert!(matches_release(
            "Dune.2.2024.1080p.Castellano.mkv",
            "Dune: Part Two",
            Some("Dune: Part Two"),
            Some(2024),
            true,
            None,
        ));
        assert!(!matches_release(
            "Dune.3.2026.1080p.Castellano.mkv",
            "Dune: Part Two",
            Some("Dune: Part Two"),
            Some(2024),
            true,
            None,
        ));
    }

    #[test]
    fn test_spanish_and_double_episode_tv_matching_and_scorer() {
        assert!(matches_release(
            "VisionQuest.Temporada.1.Cap.102.Castellano.1080p.by.Bryan_122.mkv",
            "VisionQuest",
            None,
            Some(2026),
            false,
            Some((1, 2)),
        ));
        assert!(!matches_release(
            "VisionQuest.1x10.Castellano.1080p.mkv",
            "VisionQuest",
            None,
            Some(2026),
            false,
            Some((1, 1)),
        ));
        assert!(matches_release(
            "VisionQuest.S01E01-E02.Dual.1080p.HEVC.mkv",
            "VisionQuest",
            None,
            Some(2026),
            false,
            Some((1, 2)),
        ));

        // Multi-épisodes S01E01-E04
        let span = extract_tv_episode_numbers("VisionQuest.S01E01-E04.1080p.Dual.mkv", 1);
        assert_eq!(span, vec![1, 2, 3, 4]);

        // Pack de saison complète
        assert!(matches_season_pack_release(
            "VisionQuest.Temporada.1.Completa.1080p.Castellano.HEVC",
            "VisionQuest",
            None,
            Some(2026),
            1,
        ));
        assert!(matches_season_pack_release(
            "La.casa.del.dragon.(House.of.the.Dragon).S02.1080p.Dual.HEVC",
            "La casa del dragón",
            Some("House of the Dragon"),
            Some(2022),
            2,
        ));
        assert!(!matches_season_pack_release(
            "VisionQuest.S01E02.1080p.Castellano.mkv",
            "VisionQuest",
            None,
            Some(2026),
            1,
        ));

        let rejected_eng = evaluate_release(
            "Batman.1989.1080p.BluRay.x264.English.mkv",
            "ed2k",
            "ed2k://|file|test|3000000000|aaaa|/",
            "aaaa",
            3_000_000_000,
            15,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(rejected_eng.is_rejected);

        let accepted_dual = evaluate_release(
            "Batman.(1989).(Spanish.English.Subs).BDrip.1080p.HEVC.AC3.by.nara.mkv",
            "ed2k",
            "ed2k://|file|test|3500000000|bbbb|/",
            "bbbb",
            3_500_000_000,
            20,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!accepted_dual.is_rejected);
        assert_eq!(accepted_dual.language, "Dual");
        assert!(accepted_dual.score >= 200);
    }

    #[test]
    fn test_english_unknown_and_foreign_releases_strictly_rejected_and_bilingual_matched() {
        // Unlabeled US English scene releases must be rejected when preferred_language is Spanish
        let us_scene = evaluate_release(
            "Station.Eleven.2021.S01E07.Goodbye.My.Damaged.Home.1080p.WEBRip.EAC3.HEVC-t3nzin.mkv",
            "ed2k",
            "ed2k://|file|test|1730600138|cccc|/",
            "cccc",
            1_730_600_138,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert!(us_scene.is_rejected);
        assert_eq!(us_scene.language, "Unknown");

        // French VOSTFR and Italian ITA ENG must be rejected as Foreign
        let fr_rel = evaluate_release(
            "Station.Eleven.S01E02.FASTSUB.VOSTFR.WEBRip.x264-WEEDS.mp4",
            "ed2k",
            "ed2k://|file|test|340628426|dddd|/",
            "dddd",
            340_628_426,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert!(fr_rel.is_rejected);
        assert_eq!(fr_rel.language, "Foreign");

        let ita_rel = evaluate_release(
            "Station Eleven S01E01 Wheel of Fire - h264 1080p ITA ENG - e55e.mkv",
            "ed2k",
            "ed2k://|file|test|900000000|eeee|/",
            "eeee",
            900_000_000,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert!(ita_rel.is_rejected);
        assert_eq!(ita_rel.language, "Foreign");

        // Spanish release with (Spa-Eng-Spanishsub) must be accepted as Dual
        let spa_dual = evaluate_release(
            "Estacion.once.1x01.Rueda.de.fuego.(Spanish.English.Subs).HMAX.WEB-DL.1080p.x264-AC3.(pxl-sharerip).mkv",
            "ed2k",
            "ed2k://|file|test|1200000000|ffff|/",
            "ffff",
            1_200_000_000,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert!(!spa_dual.is_rejected);
        assert_eq!(spa_dual.language, "Dual");

        let spa_hyphen = evaluate_release(
            "Pantheon.1x02.Ciclos.(Spa-Eng-Spanishsub).HDrip.1080.HEVC.10b-AC3.by.nara.mkv",
            "ed2k",
            "ed2k://|file|test|759010056|1111|/",
            "1111",
            759_010_056,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert!(!spa_hyphen.is_rejected);
        assert_eq!(spa_hyphen.language, "Dual");

        // Bilingual title matching: "Station Eleven" <-> "Estacion once"
        assert!(matches_release(
            "Estacion.once.1x01.Rueda.de.fuego.(Spanish.English.Subs).HMAX.WEB-DL.1080p.x264-AC3.(pxl-sharerip).mkv",
            "Station Eleven",
            None,
            Some(2021),
            false,
            Some((1, 1)),
        ));
        assert!(matches_release(
            "Estacion once - Temporada 1 [HDTV][Cap.106][Castellano].avi",
            "Station Eleven",
            None,
            Some(2021),
            false,
            Some((1, 6)),
        ));

        // (Eng-Sub) release by trusted group must be classified as VOSE, not Castellano,
        // and must score lower than a true Dual release
        let geot_vose = evaluate_release(
            "Pantheon.(2023).2x01.The.Gods.Have.Not.Died.In.Vain.(Eng-Sub).HD.1080p.CHDR.HEVC.10b-AAC.by.Geot.(siemprehevc).mkv",
            "ed2k",
            "ed2k://|file|test|316913519|2222|/",
            "2222",
            316_913_519,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert_eq!(geot_vose.language, "VOSE");

        let bryan_dual = evaluate_release(
            "Pantheon.S02E01.Los.dioses.no.han.muerto.en.vano.(Spanish.English.Subs).WEB-DL.1080p.x264-EAC3.by.Bryan_122.mkv",
            "ed2k",
            "ed2k://|file|test|1168908941|3333|/",
            "3333",
            1_168_908_941,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert_eq!(bryan_dual.language, "Dual");
        assert!(bryan_dual.score > geot_vose.score);

        // _Eng.Subt.Esp must be VOSE, not Dual
        let subt_esp = evaluate_release(
            "Estación Once (Station Eleven) S01E04 [Rosencrantz and Guildenstern Aren't Dead]_Eng.Subt.Esp.mkv",
            "ed2k",
            "ed2k://|file|test|1114576439|4444|/",
            "4444",
            1_114_576_439,
            5,
            "eD2k/Kad",
            false,
            "1080p-es",
            "es",
        );
        assert_eq!(subt_esp.language, "VOSE");

        // Monsieur Ibrahim / El señor Ibrahim y las flores del Corán:
        // 1. La release catalane/française `.fr.fr.vo.vos_es` doit être rejetée comme `Foreign`
        let cat_fr_rel = evaluate_release(
            "El.senyor.Ibrahim.i.las.flors.de.l'Alcora.(Monsieur.Ibrahim.et.les.fleurs.du.Coran).Francois.Dupeyron.2003.fr.fr.vo.vos_es.vd_es.mkv",
            "ed2k",
            "ed2k://|file|test|1500000000|5555|/",
            "5555",
            1_500_000_000,
            2,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(cat_fr_rel.is_rejected);
        assert_eq!(cat_fr_rel.language, "Foreign");

        // 2. La release Dual 1080p `(spa.fr).1080p.Blurayx264.mkv` doit être acceptée en Dual et matcher le titre
        let ibrahim_dual = evaluate_release(
            "El señor Ibrahim y las flores del Coran (2003) François Dupeyron (spa.fr).1080p.Blurayx264.mkv",
            "ed2k",
            "ed2k://|file|test|1804599296|6666|/",
            "6666",
            1_804_599_296,
            5,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!ibrahim_dual.is_rejected);
        assert_eq!(ibrahim_dual.language, "Dual");
        assert_eq!(ibrahim_dual.quality, "1080p");

        assert!(matches_release(
            "El señor Ibrahim y las flores del Coran (2003) François Dupeyron (spa.fr).1080p.Blurayx264.mkv",
            "Monsieur Ibrahim",
            Some("Monsieur Ibrahim et les Fleurs du Coran"),
            Some(2003),
            true,
            None,
        ));
        assert!(matches_release(
            "El seÃ±or Ibrahim y las flores del Coran (2003) FranÃ§ois Dupeyron (spa.fr).1080p.Blurayx264.mkv",
            "El señor Ibrahim y las flores del Corán",
            Some("Monsieur Ibrahim et les Fleurs du Coran"),
            Some(2003),
            true,
            None,
        ));

        // A Touch of Spice / Un toque de canela (DVD-Rip en castellano)
        let canela_rel = evaluate_release(
            "Un toque de canela.(2003 Tassos Boulmetis). (Spanish) Dvd-Rip Xvid-Ac3.avi",
            "ed2k",
            "ed2k://|file|Un toque de canela.(2003 Tassos Boulmetis). (Spanish) Dvd-Rip Xvid-Ac3.avi|1573390336|a519e07bf0deeb9afc172239d619374d|/",
            "a519e07bf0deeb9afc172239d619374d",
            1_573_390_336,
            7,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!canela_rel.is_rejected);
        assert_eq!(canela_rel.language, "Castellano");
        assert_eq!(canela_rel.quality, "SD");
        assert!(matches_release(
            "Un toque de canela.(2003 Tassos Boulmetis). (Spanish) Dvd-Rip Xvid-Ac3.avi",
            "A Touch of Spice",
            Some("Πολίτικη Κουζίνα"),
            Some(2003),
            true,
            None,
        ));
    }

    #[test]
    fn test_ali_g_who_iz_i_matches_spanish_slang_release() {
        let cand = "Ali G Kien es yo (2026) [(Who Iz I) m1080p h264 AC3 5.1 Spa Eng +EAC3 Spa +Sub yamil].mkv";
        assert!(matches_release(
            cand,
            "Ali G: Who Iz I?",
            Some("Ali G: Who Iz I?"),
            Some(2026),
            true,
            None
        ));
        let scored = evaluate_release(
            cand,
            "ed2k",
            "ed2k://|file|sample|4250000000|0123456789abcdef0123456789abcdef|/",
            "0123456789abcdef0123456789abcdef",
            4_250_000_000,
            10,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!scored.is_rejected);
        assert_eq!(scored.language, "Dual");
        assert_eq!(scored.quality, "1080p");
        assert!(scored.score > 100);
    }

    #[test]
    fn test_viaje_a_la_luna_1902_silent_matches_and_not_rejected() {
        let cand1 = "Viaje.a.la.Luna(Le.voyage.dans.la.lune(1902).(Silent.Spanishsub).HDrip.DivX-AC3.by.abantos.(seriestv.in).avi";
        assert!(matches_release(
            cand1,
            "El viaje a la Luna",
            Some("Le Voyage dans la Lune"),
            Some(1902),
            true,
            None
        ));
        let scored1 = evaluate_release(
            cand1,
            "ed2k",
            "ed2k://|file|sample|680500000|0123456789abcdef0123456789abcdef|/",
            "0123456789abcdef0123456789abcdef",
            680_500_000, // 649 MB
            10,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!scored1.is_rejected);
        assert_eq!(scored1.language, "VOSE");
        assert!(scored1.score > 50);

        let cand2 = "Cine - Georges Méliès - 1902 - Viaje a la Luna (1902) - (Le.voyage.dans.la.lune - MUDA-Spanishsub) - COLOR - Restaurada.mkv";
        assert!(matches_release(
            cand2,
            "El viaje a la Luna",
            Some("Le Voyage dans la Lune"),
            Some(1902),
            true,
            None
        ));
        let scored2 = evaluate_release(
            cand2,
            "ed2k",
            "ed2k://|file|sample|262144000|0123456789abcdef0123456789abcdef|/",
            "0123456789abcdef0123456789abcdef",
            262_144_000, // 250 MB
            8,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!scored2.is_rejected);
        assert_eq!(scored2.language, "VOSE");
        assert!(scored2.score > 50);
    }

    #[test]
    fn test_roan_inish_matches_spanish_distribution_title() {
        let cand = "El.secreto.de.la.isla.de.las.focas.(John.Sayles,1994).(Spanish.English.Subs).WEBRip.1080p.x265-AC3.AAC.mkv";
        assert!(matches_release(
            cand,
            "The Secret of Roan Inish",
            Some("The Secret of Roan Inish"),
            Some(1994),
            true,
            None
        ));
        assert!(matches_release_with_alts(
            cand,
            "The Secret of Roan Inish",
            Some("The Secret of Roan Inish"),
            &["El secreto de la isla de las focas"],
            Some(1994),
            true,
            None
        ));
        let scored = evaluate_release(
            cand,
            "ed2k",
            "ed2k://|file|sample|1948000000|0123456789abcdef0123456789abcdef|/",
            "0123456789abcdef0123456789abcdef",
            1_948_000_000,
            12,
            "eD2k/Kad",
            true,
            "1080p-es",
            "es",
        );
        assert!(!scored.is_rejected);
        assert_eq!(scored.language, "Dual");
        assert_eq!(scored.quality, "1080p");
        assert!(scored.score > 100);
    }

    #[test]
    fn test_spanish_title_variants_for_roan_and_luna() {
        let roan_variants =
            spanish_title_search_variants("The Secret of Roan Inish", Some("The Secret of Roan Inish"));
        assert!(roan_variants.iter().any(|v| v.contains("focas")));

        let luna_variants =
            spanish_title_search_variants("A Trip to the Moon", Some("Le Voyage dans la Lune"));
        assert!(luna_variants.iter().any(|v| v.to_lowercase().contains("viaje")));
    }
}
