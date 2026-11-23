// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use super::matching::{clean_title_tokens, normalize_accents_and_symbols, strip_trailing_year_suffix};

pub(super) const FORBIDDEN_EXTENSIONS: &[&str] = &[
    ".exe", ".scr", ".bat", ".cmd", ".vbs", ".zip", ".rar", ".7z", ".iso", ".tar", ".gz",
];

/// Grupos y uploaders de confianza conocidos en eD2K y Torznab hispanohablantes.
pub(super) const TRUSTED_RELEASE_GROUPS: &[&str] = &[
    "nara", "geot", "bryan_122", "bryan122", "eth@n", "ethan", "yamil", "hdo", "cartmangold",
    "caloriana", "krom", "mony2007", "zapiron", "pirolo", "wolfmax4k", "pctfenix", "dontorrent",
    "grantorrent", "divxtotal", "atomixhq", "depechemode13", "grupots", "grupohds", "sharerip",
    "parip", "hispashare", "hispamula", "forosfreaky", "nocturniap2p", "latabernadelcangrejo",
    "mokesky", "piter332", "kowalski", "sitherkiller", "oli3112", "byp58", "ercius", "yumand22",
    "rubesp", "legan", "exploradoresp2p", "juanpaleo",
];

/// Traducciones al español comunes de palabras frecuentes en títulos de series/películas en inglés.
pub(super) const EN_ES_TITLE_WORD_EQUIVALENTS: &[(&str, &str)] = &[
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
    ("body", "cuerpo"),
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

/// Traducciones oficiales al español de series de TV cuyo título en España diverge del inglés.
pub(super) const KNOWN_SERIES_SPANISH_TITLES: &[(&str, &[&str])] = &[
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
    ("the body", &["El cuerpo"]),
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

/// Genera las variantes de títulos en español para una serie o película con el fin de hallar las versiones hispanohablantes.
pub fn spanish_title_search_variants(title: &str, original_title: Option<&str>) -> Vec<String> {
    let clean_title = strip_trailing_year_suffix(title);
    let clean_orig = original_title.map(strip_trailing_year_suffix);
    let mut out = Vec::new();
    let mut push_unique = |cand: String| {
        let trimmed = cand.trim().to_string();
        if !trimmed.is_empty()
            && !trimmed.eq_ignore_ascii_case(title)
            && !trimmed.eq_ignore_ascii_case(clean_title)
            && !original_title.map(|o| trimmed.eq_ignore_ascii_case(o)).unwrap_or(false)
            && !clean_orig.map(|o| trimmed.eq_ignore_ascii_case(o)).unwrap_or(false)
            && !out.iter().any(|x: &String| x.eq_ignore_ascii_case(&trimmed))
        {
            out.push(trimmed);
        }
    };

    for src in [Some(title), original_title].into_iter().flatten() {
        let clean_src = strip_trailing_year_suffix(src);
        let norm = normalize_accents_and_symbols(clean_src);
        let joined = norm.split_whitespace().collect::<Vec<_>>().join(" ");
        for &(en_key, es_list) in KNOWN_SERIES_SPANISH_TITLES {
            if joined == en_key {
                for &es_t in es_list {
                    push_unique(es_t.to_string());
                }
            }
        }

        // Traducción palabra por palabra si todas las palabras significativas tienen un equivalente directo en español
        let tokens = clean_title_tokens(clean_src);
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


pub(super) const STOP_WORDS: &[&str] = &[
    "de", "la", "el", "los", "las", "un", "una", "unos", "unas", "y", "o", "en", "del", "al",
    "the", "a", "an", "and", "or", "in", "on", "of", "to", "for", "with", "by", "at",
];

/// Tokens técnicos/calidad/idioma que marcan el fin del título de una versión P2P.
pub(super) const RELEASE_METADATA_BOUNDARY_TOKENS: &[&str] = &[
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

/// Marcadores de secuelas o subtítulos de franquicia que no deben aparecer nunca
/// en el título candidato si no figuran en el título esperado.
pub(super) const SEQUEL_AND_SPINOFF_MARKERS: &[&str] = &[
    "2", "3", "4", "5", "6", "7", "8", "9", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix",
    "part", "parte", "vol", "volume", "volumen", "chapter", "capitulo", "returns", "vuelve",
    "begins", "forever", "robin", "dark", "knight", "rises", "superman", "dawn", "justice",
    "resurrection", "reloaded", "revolutions", "revenge", "origins", "awakens", "apocalypse",
    "covenant", "romulus", "requiem", "predator", "salvation", "genisys", "fate", "bloodlines",
    "afterlife", "frozen", "empire", "kingdom", "conquest", "battle", "beneath", "escape",
];

