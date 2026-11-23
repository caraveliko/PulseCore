// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use mule_core::model::MediaItem;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Número máximo de miembros del equipo creativo (directores/creadores/guionistas) devueltos.
const MAX_CREATIVE_CREDITS: usize = 6;
/// Número máximo de actores principales devueltos en la ficha multimedia.
const MAX_CAST_CREDITS: usize = 16;
/// Número máximo de personas para las cuales se intenta una foto de respaldo de Wikipedia por llamada.
const MAX_PHOTO_FALLBACK_LOOKUPS: usize = 6;
/// Tiempo máximo de espera (en segundos) para una petición de foto de Wikipedia de respaldo.
const PHOTO_FALLBACK_TIMEOUT_SECS: u64 = 4;
/// Tiempo máximo de espera (en segundos) para la traducción automática de sinopsis al español.
const TRANSLATION_FALLBACK_TIMEOUT_SECS: u64 = 4;
/// Longitud máxima de cada fragmento enviado al traductor de respaldo.
const MAX_TRANSLATION_CHUNK_CHARS: usize = 450;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPerson {
    pub name: String,
    pub role: String,
    pub department: String,
    pub photo_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRatings {
    pub tmdb: Option<f64>,
    pub imdb: Option<f64>,
    pub rotten_tomatoes: Option<f64>,
    pub metacritic: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeRichMeta {
    pub season_number: u32,
    pub episode_number: u32,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub image_url: Option<String>,
    pub runtime_mins: Option<u32>,
    pub air_date: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaExtendedDetails {
    pub tmdb_id: Option<u64>,
    pub tvdb_id: Option<u64>,
    pub imdb_id: Option<String>,
    pub directors: Vec<CreditPerson>,
    pub cast: Vec<CreditPerson>,
    pub genres: Vec<String>,
    pub runtime_mins: Option<u32>,
    pub studio_or_network: Option<String>,
    pub certification: Option<String>,
    pub show_status: Option<String>,
    pub trailer_youtube_id: Option<String>,
    pub backdrop_url: Option<String>,
    pub poster_url: Option<String>,
    pub original_language: Option<String>,
    pub ratings: MediaRatings,
    pub episode_meta: HashMap<String, EpisodeRichMeta>,
    #[serde(default)]
    pub alternative_titles: Vec<String>,
    #[serde(default)]
    pub spanish_title: Option<String>,
    #[serde(default)]
    pub spanish_overview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMediaResult {
    pub id: String,
    pub tmdb_id: Option<u64>,
    pub tvdb_id: Option<u64>,
    pub imdb_id: Option<String>,
    pub media_type: String, // "movie" | "tv"
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<u32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub rating: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct MediaMetadataService {
    client: Client,
    details_cache: Arc<RwLock<HashMap<String, MediaExtendedDetails>>>,
    person_photo_cache: Arc<RwLock<HashMap<String, Option<String>>>>,
    es_overview_cache: Arc<RwLock<HashMap<String, String>>>,
}

/// Detecta de forma heurística si una sinopsis está redactada en inglés.
pub fn is_likely_english_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 16 {
        return false;
    }
    let padded = format!(" {} ", trimmed.to_lowercase());
    let en_markers = [
        " the ", " and ", " with ", " when ", " after ", " their ", " from ", " into ",
        " while ", " who ", " his ", " her ", " they ", " that ", " this ", " have ",
        " has ", " been ", " finds ", " discovers ", " wakes ", " must ", " world ",
        " life ", " family ", " young ", " two ", " one ", " for ", " are ", " is ",
    ];
    let es_markers = [
        " el ", " la ", " los ", " las ", " una ", " con ", " para ", " por ", " del ",
        " que ", " cuando ", " pero ", " como ", " sus ", " tras ", " hacia ", " donde ",
        " mientras ", " entre ", " sobre ", " hasta ", " desde ", " todo ", " vida ",
        " años ", " es ", " son ", " un ", " en ", " su ", " se ", " al ",
    ];
    let en_hits = en_markers.iter().filter(|m| padded.contains(*m)).count();
    let es_hits = es_markers.iter().filter(|m| padded.contains(*m)).count();
    en_hits > es_hits && en_hits >= 2
}

/// Traduce nombres de géneros cinematográficos/televisivos de TMDB/TVDB al español de España.
pub fn translate_genre_to_spanish(genre: &str) -> String {
    match genre.trim().to_lowercase().as_str() {
        "action" => "Acción",
        "action & adventure" => "Acción y aventura",
        "adventure" => "Aventura",
        "animation" => "Animación",
        "biography" => "Biografía",
        "children" => "Infantil",
        "comedy" => "Comedia",
        "crime" => "Crimen",
        "documentary" => "Documental",
        "drama" => "Drama",
        "family" => "Familia",
        "fantasy" => "Fantasía",
        "history" => "Historia",
        "horror" => "Terror",
        "mini-series" | "miniseries" => "Miniserie",
        "music" => "Música",
        "musical" => "Musical",
        "mystery" => "Misterio",
        "reality" | "reality tv" => "Telerrealidad",
        "romance" => "Romance",
        "science fiction" | "sci-fi" | "sci-fi & fantasy" => "Ciencia ficción",
        "soap" => "Telenovela",
        "sport" | "sports" => "Deporte",
        "suspense" | "thriller" => "Suspense",
        "talk" => "Entrevistas",
        "tv movie" => "Película de TV",
        "war" | "war & politics" => "Bélica",
        "western" => "Western",
        other => return other.to_string(),
    }
    .to_string()
}

impl MediaMetadataService {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();
        Self {
            client,
            details_cache: Arc::new(RwLock::new(HashMap::new())),
            person_photo_cache: Arc::new(RwLock::new(HashMap::new())),
            es_overview_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Extrae el título y la sinopsis traducidos al español (prioridad `es-ES`, luego `es-MX` / `es`)
    /// desde la tabla `Translations` de la API Radarr/TMDB buscando ambos campos de forma independiente.
    fn extract_spanish_translation(item: &serde_json::Value) -> (Option<String>, Option<String>) {
        let Some(translations) = item["Translations"].as_array() else {
            return (None, None);
        };
        let preferred_langs = ["es-ES", "es-MX", "es", "spa"];
        let mut best_title: Option<String> = None;
        let mut best_overview: Option<String> = None;
        for pref in preferred_langs {
            for t in translations {
                let lang = t["Language"].as_str().unwrap_or("");
                if lang.eq_ignore_ascii_case(pref) {
                    if best_title.is_none() {
                        best_title = t["Title"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from);
                    }
                    if best_overview.is_none() {
                        best_overview = t["Overview"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty() && !is_likely_english_text(s))
                            .map(String::from);
                    }
                    if best_title.is_some() && best_overview.is_some() {
                        return (best_title, best_overview);
                    }
                }
            }
        }
        (best_title, best_overview)
    }

    /// Garantiza una sinopsis en español de España: si el texto origen está en inglés,
    /// aplica traducción automática con caché en memoria.
    pub async fn ensure_spanish_overview(
        &self,
        cache_key: &str,
        raw_overview: Option<String>,
    ) -> Option<String> {
        let trimmed = raw_overview
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        if !is_likely_english_text(&trimmed) {
            return Some(trimmed);
        }
        if let Some(cached) = self.es_overview_cache.read().await.get(cache_key).cloned() {
            return Some(cached);
        }
        if let Some(translated) = self.translate_en_to_es_mymemory(&trimmed).await {
            self.es_overview_cache
                .write()
                .await
                .insert(cache_key.to_string(), translated.clone());
            return Some(translated);
        }
        Some(trimmed)
    }

    async fn translate_en_to_es_mymemory(&self, text: &str) -> Option<String> {
        let mut chunks: Vec<String> = Vec::new();
        let mut current = String::new();
        for sentence in text.split_inclusive(['.', '!', '?']) {
            if !current.is_empty() && current.len() + sentence.len() > MAX_TRANSLATION_CHUNK_CHARS {
                chunks.push(current.trim().to_string());
                current.clear();
            }
            current.push_str(sentence);
        }
        if !current.trim().is_empty() {
            chunks.push(current.trim().to_string());
        }
        if chunks.is_empty() {
            return None;
        }

        let mut translated_parts = Vec::with_capacity(chunks.len());
        for chunk in chunks.into_iter().take(4) {
            let encoded: String = url::form_urlencoded::byte_serialize(chunk.as_bytes()).collect();
            let url = format!("https://api.mymemory.translated.net/get?q={encoded}&langpair=en|es");
            let Ok(Ok(resp)) = tokio::time::timeout(
                Duration::from_secs(TRANSLATION_FALLBACK_TIMEOUT_SECS),
                self.client.get(&url).send(),
            )
            .await
            else {
                return None;
            };
            if !resp.status().is_success() {
                return None;
            }
            let Ok(bytes) = resp.bytes().await else {
                return None;
            };
            let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return None;
            };
            let part = val["responseData"]["translatedText"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            if part.to_uppercase().contains("MYMEMORY WARNING") {
                return None;
            }
            translated_parts.push(part.to_string());
        }

        let joined = translated_parts.join(" ").trim().to_string();
        if joined.is_empty() || is_likely_english_text(&joined) {
            None
        } else {
            Some(joined)
        }
    }

    /// Busca películas a través del proxy de metadatos abierto y en caché de Cloudflare de Radarr
    pub async fn search_movies(&self, query: &str) -> Result<Vec<SearchMediaResult>, String> {
        let encoded_q: String =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let url = format!("https://api.radarr.video/v1/search?q={encoded_q}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Search failed with status: {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let json_val: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        if let Some(arr) = json_val.as_array() {
            for item in arr {
                let tmdb_id = item["TmdbId"].as_u64();
                let imdb_id = item["ImdbId"].as_str().map(String::from);
                let (es_title, es_overview) = Self::extract_spanish_translation(item);
                let default_title = item["Title"].as_str().unwrap_or("").to_string();
                let title = es_title.unwrap_or_else(|| default_title.clone());
                let original_title = item["OriginalTitle"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .or_else(|| {
                        if !default_title.is_empty() && !default_title.eq_ignore_ascii_case(&title) {
                            Some(default_title)
                        } else {
                            None
                        }
                    });
                let year = item["Year"].as_u64().map(|y| y as u32);
                let overview = es_overview.or_else(|| item["Overview"].as_str().map(String::from));
                let mut poster_url = None;
                let mut backdrop_url = None;

                if let Some(images) = item["Images"].as_array() {
                    for img in images {
                        let ctype = img["CoverType"].as_str().unwrap_or("");
                        let u = img["Url"].as_str().map(String::from);
                        if ctype == "Poster" && poster_url.is_none() {
                            poster_url = u;
                        } else if (ctype == "Fanart" || ctype == "Backdrop")
                            && backdrop_url.is_none()
                        {
                            backdrop_url = u;
                        }
                    }
                }

                let rating = item["Ratings"]
                    .as_array()
                    .and_then(|rats| rats.first().and_then(|r| r["Value"].as_f64()));

                if !title.is_empty() {
                    results.push(SearchMediaResult {
                        id: tmdb_id
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| title.clone()),
                        tmdb_id,
                        tvdb_id: None,
                        imdb_id,
                        media_type: "movie".to_string(),
                        title,
                        original_title,
                        year,
                        overview,
                        poster_url,
                        backdrop_url,
                        rating,
                    });
                }
            }
        }
        Ok(results)
    }

    /// Busca series de TV mediante el servicio de metadatos Skyhook de Sonarr
    pub async fn search_shows(&self, query: &str) -> Result<Vec<SearchMediaResult>, String> {
        let encoded_q: String =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let url = format!("https://skyhook.sonarr.tv/v1/tvdb/search/en/?term={encoded_q}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Show search failed with status: {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let json_val: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        if let Some(arr) = json_val.as_array() {
            for item in arr {
                let tvdb_id = item["tvdbId"].as_u64();
                let tmdb_id = item["tmdbId"].as_u64();
                let imdb_id = item["imdbId"].as_str().map(String::from);
                let raw_title = item["title"].as_str().unwrap_or("");
                let title = crate::services::scorer::strip_trailing_year_suffix(raw_title).to_string();
                let year = item["firstAired"]
                    .as_str()
                    .and_then(|d| d.get(0..4))
                    .and_then(|y| y.parse::<u32>().ok());
                let overview = item["overview"].as_str().map(String::from);
                let mut poster_url = None;
                let mut backdrop_url = None;

                if let Some(images) = item["images"].as_array() {
                    for img in images {
                        let ctype = img["coverType"].as_str().unwrap_or("");
                        let u = img["url"].as_str().map(String::from);
                        if ctype == "Poster" && poster_url.is_none() {
                            poster_url = u;
                        } else if ctype == "Fanart" && backdrop_url.is_none() {
                            backdrop_url = u;
                        }
                    }
                }

                let rating = item["rating"]["value"]
                    .as_str()
                    .and_then(|v| v.parse::<f64>().ok());

                if !title.is_empty() {
                    results.push(SearchMediaResult {
                        id: tvdb_id
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| title.clone()),
                        tmdb_id,
                        tvdb_id,
                        imdb_id,
                        media_type: "tv".to_string(),
                        title,
                        original_title: None,
                        year,
                        overview,
                        poster_url,
                        backdrop_url,
                        rating,
                    });
                }
            }
        }
        Ok(results)
    }

    /// Obtiene metadatos e imágenes completas de películas por ID de TMDB a través del proxy de la API de Radarr
    pub async fn get_movie(&self, tmdb_id: u64) -> Result<SearchMediaResult, String> {
        let url = format!("https://api.radarr.video/v1/movie/{tmdb_id}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Get movie failed with status: {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let item: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;

        let (es_title, es_overview) = Self::extract_spanish_translation(&item);
        let default_title = item["Title"].as_str().unwrap_or("").to_string();
        let title = es_title.unwrap_or_else(|| default_title.clone());
        let original_title = item["OriginalTitle"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| {
                if !default_title.is_empty() && !default_title.eq_ignore_ascii_case(&title) {
                    Some(default_title)
                } else {
                    None
                }
            });
        let year = item["Year"].as_u64().map(|y| y as u32);
        let raw_overview = es_overview.or_else(|| item["Overview"].as_str().map(String::from));
        let overview = self
            .ensure_spanish_overview(&format!("movie:{tmdb_id}"), raw_overview)
            .await;
        let imdb_id = item["ImdbId"].as_str().map(String::from);
        let mut poster_url = None;
        let mut backdrop_url = None;

        if let Some(images) = item["Images"].as_array() {
            for img in images {
                let ctype = img["CoverType"].as_str().unwrap_or("");
                let u = img["Url"].as_str().map(String::from);
                if ctype == "Poster" && poster_url.is_none() {
                    poster_url = u;
                } else if (ctype == "Fanart" || ctype == "Backdrop") && backdrop_url.is_none() {
                    backdrop_url = u;
                }
            }
        }
        let rating = item["MovieRatings"]["Tmdb"]["Value"].as_f64();

        Ok(SearchMediaResult {
            id: tmdb_id.to_string(),
            tmdb_id: Some(tmdb_id),
            tvdb_id: None,
            imdb_id,
            media_type: "movie".to_string(),
            title,
            original_title,
            year,
            overview,
            poster_url,
            backdrop_url,
            rating,
        })
    }

    /// Obtiene metadatos e imágenes completas de series de TV por ID de TVDB a través de Skyhook de Sonarr
    pub async fn get_show(&self, tvdb_id: u64) -> Result<SearchMediaResult, String> {
        let url = format!("https://skyhook.sonarr.tv/v1/tvdb/shows/en/{tvdb_id}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Get show failed with status: {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let item: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;

        let tmdb_id = item["tmdbId"].as_u64();
        let imdb_id = item["imdbId"].as_str().map(String::from);
        let raw_title = item["title"].as_str().unwrap_or("");
        let title = crate::services::scorer::strip_trailing_year_suffix(raw_title).to_string();
        let year = item["firstAired"]
            .as_str()
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse::<u32>().ok());
        let raw_overview = item["overview"].as_str().map(String::from);
        let overview = self
            .ensure_spanish_overview(&format!("tv:{tvdb_id}"), raw_overview)
            .await;
        let mut poster_url = None;
        let mut backdrop_url = None;

        if let Some(images) = item["images"].as_array() {
            for img in images {
                let ctype = img["coverType"].as_str().unwrap_or("");
                let u = img["url"].as_str().map(String::from);
                if ctype == "Poster" && poster_url.is_none() {
                    poster_url = u;
                } else if ctype == "Fanart" && backdrop_url.is_none() {
                    backdrop_url = u;
                }
            }
        }
        let rating = item["rating"]["value"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok());

        Ok(SearchMediaResult {
            id: tvdb_id.to_string(),
            tmdb_id,
            tvdb_id: Some(tvdb_id),
            imdb_id,
            media_type: "tv".to_string(),
            title,
            original_title: None,
            year,
            overview,
            poster_url,
            backdrop_url,
            rating,
        })
    }

    /// Obtiene detalles de una serie incluyendo todos los episodios (temporada, episodio, título, fecha de emisión)
    pub async fn get_show_episodes(
        &self,
        tvdb_id: u64,
    ) -> Result<Vec<(u32, u32, String, Option<String>)>, String> {
        let url = format!("https://skyhook.sonarr.tv/v1/tvdb/shows/en/{tvdb_id}");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Get show failed: {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let json_val: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let mut episodes = Vec::new();
        if let Some(eps) = json_val["episodes"].as_array() {
            for ep in eps {
                let season = ep["seasonNumber"].as_u64().unwrap_or(0) as u32;
                let episode = ep["episodeNumber"].as_u64().unwrap_or(0) as u32;
                if season == 0 {
                    // Omitir especiales por defecto
                    continue;
                }
                let title = ep["title"].as_str().unwrap_or("").to_string();
                let air_date = ep["airDate"].as_str().map(String::from);
                episodes.push((season, episode, title, air_date));
            }
        }
        Ok(episodes)
    }

    /// Obtiene detalles enriquecidos (director, creadores, reparto con fotos, valoraciones,
    /// géneros, estudio/cadena, tráiler y miniaturas de episodios) con almacenamiento en caché.
    pub async fn get_extended_details(&self, item: &MediaItem) -> MediaExtendedDetails {
        if let Some(cached) = self.details_cache.read().await.get(&item.id).cloned() {
            return cached;
        }

        let details = if item.media_type == "tv" {
            self.fetch_show_extended_details(item).await
        } else {
            self.fetch_movie_extended_details(item).await
        };

        self.details_cache
            .write()
            .await
            .insert(item.id.clone(), details.clone());
        details
    }

    async fn fetch_movie_extended_details(&self, item: &MediaItem) -> MediaExtendedDetails {
        let mut details = MediaExtendedDetails {
            tmdb_id: item.tmdb_id,
            tvdb_id: item.tvdb_id,
            imdb_id: item.imdb_id.clone(),
            backdrop_url: item.backdrop_path.clone(),
            poster_url: item.poster_path.clone(),
            ..Default::default()
        };

        let mut tmdb_id = item.tmdb_id;
        if tmdb_id.is_none() {
            if let Ok(results) = self.search_movies(&item.title).await {
                if let Some(best) = results
                    .iter()
                    .find(|r| item.year.is_none() || r.year == item.year)
                    .or_else(|| results.first())
                {
                    tmdb_id = best.tmdb_id;
                }
            }
        }

        let Some(tid) = tmdb_id else {
            return details;
        };
        details.tmdb_id = Some(tid);

        let url = format!("https://api.radarr.video/v1/movie/{tid}");
        let Ok(resp) = self.client.get(&url).send().await else {
            return details;
        };
        if !resp.status().is_success() {
            return details;
        }
        let Ok(bytes) = resp.bytes().await else {
            return details;
        };
        let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return details;
        };

        if details.imdb_id.is_none() {
            details.imdb_id = val["ImdbId"].as_str().map(String::from);
        }
        details.runtime_mins = val["Runtime"].as_u64().map(|v| v as u32);
        details.studio_or_network = val["Studio"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);
        details.trailer_youtube_id = val["YoutubeTrailerId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);
        details.original_language = val["OriginalLanguage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);

        // Extraer títulos alternativos y traducciones oficiales (especialmente español 'es-ES', 'es', etc.)
        if let Some(translations) = val["Translations"].as_array() {
            for t in translations {
                if let Some(t_title) = t["Title"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    if !details.alternative_titles.iter().any(|existing| existing.eq_ignore_ascii_case(t_title)) {
                        details.alternative_titles.push(t_title.to_string());
                    }
                }
            }
        }
        if let Some(alts) = val["AlternativeTitles"].as_array() {
            for a in alts {
                if let Some(a_title) = a["Title"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    if !details.alternative_titles.iter().any(|existing| existing.eq_ignore_ascii_case(a_title)) {
                        details.alternative_titles.push(a_title.to_string());
                    }
                }
            }
        }

        let (es_title, es_overview) = Self::extract_spanish_translation(&val);
        details.spanish_title = es_title;
        let raw_overview = es_overview.or_else(|| val["Overview"].as_str().map(String::from));
        details.spanish_overview = self
            .ensure_spanish_overview(&format!("movie:{tid}"), raw_overview)
            .await;

        if let Some(genres) = val["Genres"].as_array() {
            details.genres = genres
                .iter()
                .filter_map(|g| g.as_str().map(translate_genre_to_spanish))
                .collect();
        }

        if let Some(certs) = val["Certifications"].as_array() {
            let preferred_countries = ["ES", "US", "GB", "FR", "MX"];
            for country in preferred_countries {
                if let Some(cert) = certs.iter().find_map(|c| {
                    if c["Country"].as_str() == Some(country) {
                        c["Certification"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                    } else {
                        None
                    }
                }) {
                    details.certification = Some(cert);
                    break;
                }
            }
        }

        if let Some(images) = val["Images"].as_array() {
            for img in images {
                let ctype = img["CoverType"].as_str().unwrap_or("");
                if let Some(u) = img["Url"].as_str() {
                    if ctype == "Poster" && details.poster_url.is_none() {
                        details.poster_url = Some(optimize_tmdb_image_url(u, "w500"));
                    } else if (ctype == "Fanart" || ctype == "Backdrop")
                        && details.backdrop_url.is_none()
                    {
                        details.backdrop_url = Some(optimize_tmdb_image_url(u, "w1280"));
                    }
                }
            }
        }

        details.ratings.tmdb = val["MovieRatings"]["Tmdb"]["Value"].as_f64();
        details.ratings.imdb = val["MovieRatings"]["Imdb"]["Value"].as_f64();
        details.ratings.rotten_tomatoes = val["MovieRatings"]["RottenTomatoes"]["Value"].as_f64();
        details.ratings.metacritic = val["MovieRatings"]["Metacritic"]["Value"].as_f64();

        if let Some(crew) = val["Credits"]["Crew"].as_array() {
            let mut seen_names = Vec::new();
            // Prioridad a directores, luego guionistas y compositores
            let role_priority = [
                ("Director", "Director", "directing"),
                ("Screenplay", "Guion", "writing"),
                ("Writer", "Guion", "writing"),
                ("Original Music Composer", "Música", "music"),
                ("Director of Photography", "Fotografía", "camera"),
            ];
            for (job_key, role_label, dept) in role_priority {
                for member in crew {
                    if details.directors.len() >= MAX_CREATIVE_CREDITS {
                        break;
                    }
                    if member["Job"].as_str() == Some(job_key) {
                        let name = member["Name"].as_str().unwrap_or("").trim().to_string();
                        if name.is_empty() || seen_names.contains(&name) {
                            continue;
                        }
                        let photo_url = extract_headshot_url(&member["Images"]);
                        seen_names.push(name.clone());
                        details.directors.push(CreditPerson {
                            name,
                            role: role_label.to_string(),
                            department: dept.to_string(),
                            photo_url,
                        });
                    }
                }
            }
        }

        if let Some(cast) = val["Credits"]["Cast"].as_array() {
            for actor in cast.iter().take(MAX_CAST_CREDITS) {
                let name = actor["Name"].as_str().unwrap_or("").trim().to_string();
                if name.is_empty() {
                    continue;
                }
                let character = actor["Character"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let photo_url = extract_headshot_url(&actor["Images"]);
                details.cast.push(CreditPerson {
                    name,
                    role: character,
                    department: "acting".to_string(),
                    photo_url,
                });
            }
        }

        self.enrich_missing_photos(&mut details).await;
        details
    }

    async fn fetch_show_extended_details(&self, item: &MediaItem) -> MediaExtendedDetails {
        let mut details = MediaExtendedDetails {
            tmdb_id: item.tmdb_id,
            tvdb_id: item.tvdb_id,
            imdb_id: item.imdb_id.clone(),
            backdrop_url: item.backdrop_path.clone(),
            poster_url: item.poster_path.clone(),
            ..Default::default()
        };

        let mut tvdb_id = item.tvdb_id;
        if tvdb_id.is_none() {
            if let Ok(results) = self.search_shows(&item.title).await {
                if let Some(best) = results
                    .iter()
                    .find(|r| item.year.is_none() || r.year == item.year)
                    .or_else(|| results.first())
                {
                    tvdb_id = best.tvdb_id;
                }
            }
        }

        let Some(tvid) = tvdb_id else {
            return details;
        };
        details.tvdb_id = Some(tvid);

        let url = format!("https://skyhook.sonarr.tv/v1/tvdb/shows/en/{tvid}");
        let Ok(resp) = self.client.get(&url).send().await else {
            return details;
        };
        if !resp.status().is_success() {
            return details;
        }
        let Ok(bytes) = resp.bytes().await else {
            return details;
        };
        let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return details;
        };

        if details.tmdb_id.is_none() {
            details.tmdb_id = val["tmdbId"].as_u64();
        }
        if details.imdb_id.is_none() {
            details.imdb_id = val["imdbId"].as_str().map(String::from);
        }
        details.runtime_mins = val["runtime"].as_u64().map(|v| v as u32);
        details.studio_or_network = val["network"]
            .as_str()
            .or_else(|| val["originalNetwork"].as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        details.certification = val["contentRating"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);
        details.show_status = val["status"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);
        details.original_language = val["originalLanguage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);

        if let Some(aliases) = val["aliases"].as_array() {
            for a in aliases {
                if let Some(a_title) = a["name"].as_str().or_else(|| a.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
                    if !details.alternative_titles.iter().any(|existing| existing.eq_ignore_ascii_case(a_title)) {
                        details.alternative_titles.push(a_title.to_string());
                    }
                }
            }
        }
        if let Some(translations) = val["translations"].as_array() {
            for t in translations {
                if let Some(t_title) = t["name"].as_str().or_else(|| t["title"].as_str()).map(str::trim).filter(|s| !s.is_empty()) {
                    if !details.alternative_titles.iter().any(|existing| existing.eq_ignore_ascii_case(t_title)) {
                        details.alternative_titles.push(t_title.to_string());
                    }
                }
            }
        }

        details.ratings.tmdb = val["rating"]["value"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| val["rating"]["value"].as_f64());

        details.spanish_overview = self
            .ensure_spanish_overview(&format!("tv:{tvid}"), val["overview"].as_str().map(String::from))
            .await;

        if let Some(genres) = val["genres"].as_array() {
            details.genres = genres
                .iter()
                .filter_map(|g| g.as_str().map(translate_genre_to_spanish))
                .collect();
        }

        if let Some(images) = val["images"].as_array() {
            for img in images {
                let ctype = img["coverType"].as_str().unwrap_or("");
                if let Some(u) = img["url"].as_str() {
                    if ctype == "Poster" && details.poster_url.is_none() {
                        details.poster_url = Some(u.to_string());
                    } else if ctype == "Fanart" && details.backdrop_url.is_none() {
                        details.backdrop_url = Some(u.to_string());
                    }
                }
            }
        }

        if let Some(actors) = val["actors"].as_array() {
            let mut seen_actors = Vec::new();
            for actor in actors {
                if details.cast.len() >= MAX_CAST_CREDITS {
                    break;
                }
                let name = actor["name"].as_str().unwrap_or("").trim().to_string();
                if name.is_empty() || seen_actors.contains(&name) {
                    continue;
                }
                seen_actors.push(name.clone());
                let character = actor["character"].as_str().unwrap_or("").trim().to_string();
                let photo_url = actor["image"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                details.cast.push(CreditPerson {
                    name,
                    role: character,
                    department: "acting".to_string(),
                    photo_url,
                });
            }
        }

        if let Some(eps) = val["episodes"].as_array() {
            for ep in eps {
                let season = ep["seasonNumber"].as_u64().unwrap_or(0) as u32;
                let episode = ep["episodeNumber"].as_u64().unwrap_or(0) as u32;
                if season == 0 {
                    continue;
                }
                let key = format!("S{season:02}E{episode:02}");
                details.episode_meta.insert(
                    key,
                    EpisodeRichMeta {
                        season_number: season,
                        episode_number: episode,
                        title: ep["title"].as_str().filter(|s| !s.is_empty()).map(String::from),
                        overview: ep["overview"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                        image_url: ep["image"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                        runtime_mins: ep["runtime"].as_u64().map(|r| r as u32),
                        air_date: ep["airDate"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                    },
                );
            }
        }

        // Recuperación de creadores / showrunners a través de TVMaze
        let tvmaze_id = val["tvMazeId"].as_u64();
        self.populate_tvmaze_crew_and_cast(tvid, tvmaze_id, &mut details)
            .await;

        self.enrich_missing_photos(&mut details).await;
        details
    }

    async fn populate_tvmaze_crew_and_cast(
        &self,
        tvdb_id: u64,
        known_tvmaze_id: Option<u64>,
        details: &mut MediaExtendedDetails,
    ) {
        let mut tvmaze_id = known_tvmaze_id;
        if tvmaze_id.is_none() {
            let lookup_url = format!("https://api.tvmaze.com/lookup/shows?thetvdb={tvdb_id}");
            if let Ok(resp) = self.client.get(&lookup_url).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                            tvmaze_id = v["id"].as_u64();
                        }
                    }
                }
            }
        }

        let Some(maze_id) = tvmaze_id else {
            return;
        };

        let crew_url = format!("https://api.tvmaze.com/shows/{maze_id}/crew");
        if let Ok(resp) = self.client.get(&crew_url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    if let Ok(arr) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) {
                        let mut seen = Vec::new();
                        let role_map = [
                            ("Creator", "Creador"),
                            ("Developer", "Creador"),
                            ("Showrunner", "Showrunner"),
                            ("Executive Producer", "Prod. Ejecutivo"),
                            ("Co-Executive Producer", "Co-Prod. Ejecutivo"),
                        ];
                        for (raw_type, label) in role_map {
                            for entry in &arr {
                                if details.directors.len() >= MAX_CREATIVE_CREDITS {
                                    break;
                                }
                                if entry["type"].as_str() == Some(raw_type) {
                                    let name = entry["person"]["name"]
                                        .as_str()
                                        .unwrap_or("")
                                        .trim()
                                        .to_string();
                                    if name.is_empty() || seen.contains(&name) {
                                        continue;
                                    }
                                    seen.push(name.clone());
                                    let photo_url = entry["person"]["image"]["medium"]
                                        .as_str()
                                        .or_else(|| entry["person"]["image"]["original"].as_str())
                                        .map(String::from);
                                    details.directors.push(CreditPerson {
                                        name,
                                        role: label.to_string(),
                                        department: "directing".to_string(),
                                        photo_url,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn enrich_missing_photos(&self, details: &mut MediaExtendedDetails) {
        let mut lookups = 0usize;
        for person in details
            .directors
            .iter_mut()
            .chain(details.cast.iter_mut())
        {
            if person.photo_url.is_some() {
                continue;
            }
            if lookups >= MAX_PHOTO_FALLBACK_LOOKUPS {
                break;
            }
            lookups += 1;
            person.photo_url = self.scrape_person_photo_wikipedia(&person.name).await;
        }
    }

    async fn scrape_person_photo_wikipedia(&self, name: &str) -> Option<String> {
        if let Some(cached) = self.person_photo_cache.read().await.get(name).cloned() {
            return cached;
        }

        let wiki_slug = name.trim().replace(' ', "_");
        let encoded_slug: String =
            url::form_urlencoded::byte_serialize(wiki_slug.as_bytes()).collect();
        // Restablecer los guiones bajos intactos para la API REST de Wikipedia
        let encoded_slug = encoded_slug.replace("%5F", "_");
        let url = format!("https://en.wikipedia.org/api/rest_v1/page/summary/{encoded_slug}");

        let photo = match tokio::time::timeout(
            Duration::from_secs(PHOTO_FALLBACK_TIMEOUT_SECS),
            self.client.get(&url).send(),
        )
        .await
        {
            Ok(Ok(resp)) if resp.status().is_success() => {
                if let Ok(bytes) = resp.bytes().await {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        val["thumbnail"]["source"].as_str().map(String::from)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };

        self.person_photo_cache
            .write()
            .await
            .insert(name.to_string(), photo.clone());
        photo
    }
}

fn extract_headshot_url(images_val: &serde_json::Value) -> Option<String> {
    images_val.as_array().and_then(|arr| {
        arr.iter().find_map(|img| {
            img["Url"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|u| optimize_tmdb_image_url(u, "w300"))
        })
    })
}

fn optimize_tmdb_image_url(url: &str, size: &str) -> String {
    if url.contains("image.tmdb.org/t/p/original/") {
        url.replace("image.tmdb.org/t/p/original/", &format!("image.tmdb.org/t/p/{size}/"))
    } else {
        url.to_string()
    }
}
