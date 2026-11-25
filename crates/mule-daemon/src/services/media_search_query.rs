// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use crate::services::media_metadata::SearchMediaResult;

/// Año mínimo aceptado al extraer un indicio de año desde un slug de URL.
const MIN_VALID_YEAR: u32 = 1888;
/// Año máximo aceptado al extraer un indicio de año desde un slug de URL.
const MAX_VALID_YEAR: u32 = 2100;
/// Longitud mínima de dígitos tras el prefijo `tt` para considerar válido un ID de IMDb.
const IMDB_ID_MIN_DIGITS: usize = 6;

/// Resultado del análisis de una consulta de búsqueda multimedia introducida por el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMediaQuery {
    /// Identificador numérico directo de película en TMDB.
    TmdbMovie(u64),
    /// Identificador directo de IMDb (formato `tt...`).
    ImdbId(String),
    /// Consulta textual limpia con indicio opcional de año y tipo multimedia inferido.
    Text {
        query: String,
        year_hint: Option<u32>,
        inferred_media_type: Option<&'static str>,
    },
}

/// Analiza la consulta del usuario y detecta IDs directos (`tt...`, `tmdb:...`) o URLs
/// de Trakt, IMDb y TMDB para normalizar la búsqueda.
pub fn parse_media_query(raw: &str) -> ParsedMediaQuery {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParsedMediaQuery::Text {
            query: String::new(),
            year_hint: None,
            inferred_media_type: None,
        };
    }

    if let Some(imdb_id) = extract_imdb_id(trimmed) {
        return ParsedMediaQuery::ImdbId(imdb_id);
    }

    if let Some(tmdb_id) = extract_tmdb_movie_id(trimmed) {
        return ParsedMediaQuery::TmdbMovie(tmdb_id);
    }

    if let Some((query, year_hint, media_type)) = extract_trakt_or_tmdb_tv_url(trimmed) {
        return ParsedMediaQuery::Text {
            query,
            year_hint,
            inferred_media_type: Some(media_type),
        };
    }

    ParsedMediaQuery::Text {
        query: trimmed.to_string(),
        year_hint: None,
        inferred_media_type: None,
    }
}

/// Ordena de forma estable los resultados priorizando coincidencias exactas o cercanas
/// al año sugerido en la consulta (si existe).
pub fn sort_results_by_year_hint(results: &mut [SearchMediaResult], year_hint: Option<u32>) {
    let Some(target_year) = year_hint else {
        return;
    };
    results.sort_by_key(|item| match item.year {
        Some(y) if y == target_year => 0_u8,
        Some(y) if y.abs_diff(target_year) <= 1 => 1_u8,
        _ => 2_u8,
    });
}

/// Convierte la respuesta JSON de la API de sugerencias de IMDb en resultados de búsqueda
/// filtrando personas, episodios y tipos multimedia incompatibles.
pub fn parse_imdb_suggestions(
    val: &serde_json::Value,
    want_movie: bool,
    year_hint: Option<u32>,
) -> Vec<SearchMediaResult> {
    let Some(items) = val["d"].as_array() else {
        return Vec::new();
    };

    let mut results = Vec::new();
    for item in items {
        let Some(imdb_id) = item["id"].as_str().map(str::trim) else {
            continue;
        };
        if !imdb_id.starts_with("tt") {
            continue;
        }
        let Some(title) = item["l"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };

        let qid = item["qid"].as_str().unwrap_or("");
        let q_label = item["q"].as_str().unwrap_or("");

        if !matches_imdb_media_type(qid, q_label, want_movie) {
            continue;
        }

        let year = item["y"].as_u64().map(|y| y as u32);
        let overview = item["s"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|cast| format!("Reparto: {cast}"));
        let poster_url = item["i"]["imageUrl"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        results.push(SearchMediaResult {
            id: imdb_id.to_string(),
            tmdb_id: None,
            tvdb_id: None,
            imdb_id: Some(imdb_id.to_string()),
            media_type: if want_movie { "movie" } else { "tv" }.to_string(),
            title: title.to_string(),
            original_title: None,
            year,
            overview,
            poster_url,
            backdrop_url: None,
            rating: None,
        });
    }

    sort_results_by_year_hint(&mut results, year_hint);
    results
}

/// Comprueba si un elemento de IMDb corresponde al tipo solicitado (`movie` o `tv`).
fn matches_imdb_media_type(qid: &str, q_label: &str, want_movie: bool) -> bool {
    if want_movie {
        if matches!(
            qid,
            "tvSeries" | "tvMiniSeries" | "tvEpisode" | "videoGame"
        ) {
            return false;
        }
        matches!(qid, "movie" | "tvMovie" | "short" | "video")
            || matches!(q_label, "feature" | "TV movie")
    } else {
        matches!(qid, "tvSeries" | "tvMiniSeries")
            || matches!(q_label, "TV series" | "TV mini-series")
    }
}

/// Extrae un identificador IMDb (`tt...`) desde entrada directa (`tt...`, `imdb:tt...`) o URL.
fn extract_imdb_id(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix("imdb:") {
        return validate_imdb_token(rest.trim());
    }

    if lower.contains("imdb.com/title/") {
        if let Some(after) = lower.split("imdb.com/title/").nth(1) {
            let token = after.split(['/', '?', '#']).next().unwrap_or("");
            return validate_imdb_token(token);
        }
    }

    if !lower.contains(' ') && !lower.contains('/') {
        return validate_imdb_token(&lower);
    }

    None
}

/// Valida que un token tenga el formato `tt` seguido de al menos `IMDB_ID_MIN_DIGITS` dígitos.
fn validate_imdb_token(token: &str) -> Option<String> {
    let digits = token.strip_prefix("tt")?;
    if digits.len() >= IMDB_ID_MIN_DIGITS && digits.bytes().all(|b| b.is_ascii_digit()) {
        Some(format!("tt{digits}"))
    } else {
        None
    }
}

/// Extrae un ID numérico de película TMDB desde `tmdb:123`, `tmdb-123` o URL de TMDB.
fn extract_tmdb_movie_id(input: &str) -> Option<u64> {
    let lower = input.to_ascii_lowercase();

    for prefix in ["tmdb:", "tmdb-"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let digits: String = rest
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                return digits.parse::<u64>().ok();
            }
        }
    }

    if lower.contains("themoviedb.org/movie/") {
        if let Some(after) = lower.split("themoviedb.org/movie/").nth(1) {
            let segment = after.split(['/', '?', '#']).next().unwrap_or("");
            let digits: String = segment.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits.parse::<u64>().ok();
            }
        }
    }

    None
}

/// Extrae el título limpio, año opcional y tipo multimedia desde URLs de Trakt o series TMDB.
fn extract_trakt_or_tmdb_tv_url(input: &str) -> Option<(String, Option<u32>, &'static str)> {
    let lower = input.to_ascii_lowercase();

    let (slug_raw, media_type, strip_leading_numeric_id) =
        if let Some(idx) = lower.find("trakt.tv/movies/") {
            (&input[idx + "trakt.tv/movies/".len()..], "movie", false)
        } else if let Some(idx) = lower.find("trakt.tv/shows/") {
            (&input[idx + "trakt.tv/shows/".len()..], "tv", false)
        } else {
            let idx = lower.find("themoviedb.org/tv/")?;
            (&input[idx + "themoviedb.org/tv/".len()..], "tv", true)
        };

    let clean_path = slug_raw
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let first_segment = clean_path.split('/').next().unwrap_or("").trim();
    if first_segment.is_empty() {
        return None;
    }

    let mut tokens: Vec<&str> = first_segment
        .split('-')
        .filter(|t| !t.is_empty())
        .collect();

    if strip_leading_numeric_id && tokens.first().is_some_and(|t| t.bytes().all(|b| b.is_ascii_digit()))
    {
        tokens.remove(0);
    }

    if tokens.is_empty() {
        return None;
    }

    let mut year_hint = None;
    if tokens.len() > 1 {
        if let Some(last) = tokens.last() {
            if last.len() == 4 {
                if let Ok(y) = last.parse::<u32>() {
                    if (MIN_VALID_YEAR..=MAX_VALID_YEAR).contains(&y) {
                        year_hint = Some(y);
                        tokens.pop();
                    }
                }
            }
        }
    }

    let title = tokens.join(" ");
    if title.is_empty() {
        return None;
    }

    Some((title, year_hint, media_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_trakt_movie_url_with_year() {
        let parsed = parse_media_query("https://app.trakt.tv/movies/heart-of-the-beast-2026");
        assert_eq!(
            parsed,
            ParsedMediaQuery::Text {
                query: "heart of the beast".to_string(),
                year_hint: Some(2026),
                inferred_media_type: Some("movie"),
            }
        );
    }

    #[test]
    fn parses_trakt_show_url() {
        let parsed = parse_media_query("https://trakt.tv/shows/breaking-bad?tab=episodes");
        assert_eq!(
            parsed,
            ParsedMediaQuery::Text {
                query: "breaking bad".to_string(),
                year_hint: None,
                inferred_media_type: Some("tv"),
            }
        );
    }

    #[test]
    fn parses_imdb_id_and_url() {
        assert_eq!(
            parse_media_query("tt7526136"),
            ParsedMediaQuery::ImdbId("tt7526136".to_string())
        );
        assert_eq!(
            parse_media_query("https://www.imdb.com/title/tt7526136/?ref_=nv"),
            ParsedMediaQuery::ImdbId("tt7526136".to_string())
        );
    }

    #[test]
    fn parses_tmdb_id_and_url() {
        assert_eq!(
            parse_media_query("tmdb:1263337"),
            ParsedMediaQuery::TmdbMovie(1263337)
        );
        assert_eq!(
            parse_media_query("https://www.themoviedb.org/movie/1263337-heart-of-the-beast"),
            ParsedMediaQuery::TmdbMovie(1263337)
        );
    }

    #[test]
    fn preserves_plain_spanish_text_query() {
        let parsed = parse_media_query("En el corazón de la bestia");
        assert_eq!(
            parsed,
            ParsedMediaQuery::Text {
                query: "En el corazón de la bestia".to_string(),
                year_hint: None,
                inferred_media_type: None,
            }
        );
    }

    #[test]
    fn parses_imdb_suggestions_filtering_people_and_shows_and_sorting_by_year() {
        let payload = json!({
            "d": [
                {
                    "id": "nm0933988",
                    "l": "Rainn Wilson"
                },
                {
                    "id": "tt0852974",
                    "l": "The Heart of the Beast",
                    "qid": "movie",
                    "q": "feature",
                    "y": 2005
                },
                {
                    "id": "tt0231040",
                    "l": "Policías, en el corazón de la calle",
                    "qid": "tvSeries",
                    "q": "TV series",
                    "y": 2000
                },
                {
                    "id": "tt7526136",
                    "l": "Heart of the Beast",
                    "qid": "movie",
                    "q": "feature",
                    "y": 2026,
                    "s": "Brad Pitt, J.K. Simmons",
                    "i": {
                        "imageUrl": "https://m.media-amazon.com/images/M/test.jpg"
                    }
                }
            ]
        });

        let movies = parse_imdb_suggestions(&payload, true, Some(2026));
        assert_eq!(movies.len(), 2);
        assert_eq!(movies[0].imdb_id.as_deref(), Some("tt7526136"));
        assert_eq!(movies[0].title, "Heart of the Beast");
        assert_eq!(movies[0].year, Some(2026));
        assert_eq!(
            movies[0].poster_url.as_deref(),
            Some("https://m.media-amazon.com/images/M/test.jpg")
        );
        assert_eq!(movies[1].imdb_id.as_deref(), Some("tt0852974"));

        let shows = parse_imdb_suggestions(&payload, false, None);
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].imdb_id.as_deref(), Some("tt0231040"));
        assert_eq!(shows[0].media_type, "tv");
    }
}
