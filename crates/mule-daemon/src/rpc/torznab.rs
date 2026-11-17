// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Implementación de la API de indexador Torznab para la integración con Radarr / Sonarr / Prowlarr.
//!
//! Proporciona capacidades estándar de Torznab y funciones de búsqueda (movie, tv, general)
//! consultando la DHT de Kademlia y formateando los resultados como RSS 2.0 XML.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::state::AppState;

struct CachedSearchResult {
    timestamp: Instant,
    results: Vec<(String, String, u64, u64)>,
}

static SEARCH_CACHE: LazyLock<Mutex<HashMap<String, CachedSearchResult>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const CACHE_TTL: Duration = Duration::from_secs(600); // 10 minutes cache

#[derive(Debug, Deserialize)]
pub struct TorznabQuery {
    pub t: Option<String>,
    pub q: Option<String>,
    pub cat: Option<String>,
    pub season: Option<String>,
    pub ep: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub apikey: Option<String>,
}

fn torznab_rss_response(items_xml: &str) -> Response {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="1.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:torznab="http://torznab.com/schemas/2015/feed">
  <channel>
    <title>eMule-Rust Torznab Indexer</title>
    <description>eMule-Rust eD2k/Kad Network Search Results</description>
    <link>http://localhost</link>
    <language>es-es</language>
{items_xml}  </channel>
</rss>"#
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        xml,
    )
        .into_response()
}

fn sample_validation_feed_response() -> Response {
    torznab_rss_response(
        r#"    <item>
      <title>Sample.Test.Movie.mkv</title>
      <guid isPermaLink="false">A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4</guid>
      <pubDate>Sun, 01 Nov 2026 00:00:00 +0000</pubDate>
      <size>1073741824</size>
      <link>magnet:?xt=urn:btih:a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d400000000&amp;dn=Sample.Test.Movie.mkv&amp;xl=1073741824</link>
      <enclosure url="magnet:?xt=urn:btih:a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d400000000&amp;dn=Sample.Test.Movie.mkv&amp;xl=1073741824" length="1073741824" type="application/x-bittorrent" />
      <torznab:attr name="category" value="2000"/>
      <torznab:attr name="peers" value="10"/>
      <torznab:attr name="seeders" value="10"/>
    </item>
    <item>
      <title>Sample.Test.TV.Show.S01E01.mkv</title>
      <guid isPermaLink="false">B1C2D3E4F5A6B1C2D3E4F5A6B1C2D3E4</guid>
      <pubDate>Sun, 01 Nov 2026 00:00:00 +0000</pubDate>
      <size>1073741824</size>
      <link>magnet:?xt=urn:btih:b1c2d3e4f5a6b1c2d3e4f5a6b1c2d3e400000000&amp;dn=Sample.Test.TV.Show.S01E01.mkv&amp;xl=1073741824</link>
      <enclosure url="magnet:?xt=urn:btih:b1c2d3e4f5a6b1c2d3e4f5a6b1c2d3e400000000&amp;dn=Sample.Test.TV.Show.S01E01.mkv&amp;xl=1073741824" length="1073741824" type="application/x-bittorrent" />
      <torznab:attr name="category" value="5040"/>
      <torznab:attr name="peers" value="10"/>
      <torznab:attr name="seeders" value="10"/>
    </item>
"#,
    )
}

fn empty_feed_response() -> Response {
    torznab_rss_response("")
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

fn is_common_stopword(w: &str) -> bool {
    matches!(
        w,
        "the" | "and" | "for" | "with" | "los" | "las" | "del" | "por" | "para" | "una" | "uno" | "con" | "von" | "der" | "die" | "das"
    )
}

pub async fn handle_torznab(
    State(state): State<AppState>,
    Query(params): Query<TorznabQuery>,
) -> Response {
    let t = params.t.as_deref().unwrap_or("caps");

    // Validar apikey si se proporciona o cuando esté vinculado a una interfaz que no sea loopback (Spec 001 §5 / Spec 002)
    if t != "caps" {
        let expected_token = state.config.auth_token.as_str();
        let is_loopback_bind = state.config.bind_addr.ip().is_loopback();
        match params.apikey.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            Some(provided) if !expected_token.is_empty() && provided != expected_token => {
                return (
                    StatusCode::UNAUTHORIZED,
                    [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                    r#"<?xml version="1.0" encoding="UTF-8"?><error code="100" description="Incorrect user credentials"/>"#,
                )
                    .into_response();
            }
            None if !is_loopback_bind && !expected_token.is_empty() => {
                return (
                    StatusCode::UNAUTHORIZED,
                    [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                    r#"<?xml version="1.0" encoding="UTF-8"?><error code="100" description="Missing API key"/>"#,
                )
                    .into_response();
            }
            _ => {}
        }
    }

    match t {
        "caps" => {
            let caps_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <server version="1.0" title="eMule-Rust Torznab Indexer"/>
  <limits max="100" default="100"/>
  <searching>
    <search available="yes" supportedParams="q"/>
    <tv-search available="yes" supportedParams="q,season,ep"/>
    <movie-search available="yes" supportedParams="q"/>
  </searching>
  <categories>
    <category id="2000" name="Movies">
      <subcat id="2010" name="Movies/Foreign"/>
      <subcat id="2020" name="Movies/Other"/>
      <subcat id="2030" name="Movies/SD"/>
      <subcat id="2040" name="Movies/HD"/>
      <subcat id="2045" name="Movies/UHD"/>
      <subcat id="2050" name="Movies/BluRay"/>
      <subcat id="2060" name="Movies/3D"/>
      <subcat id="2070" name="Movies/DVD"/>
      <subcat id="2080" name="Movies/WEB-DL"/>
      <subcat id="2090" name="Movies/x265"/>
    </category>
    <category id="5000" name="TV">
      <subcat id="5010" name="TV/WEB-DL"/>
      <subcat id="5020" name="TV/Foreign"/>
      <subcat id="5030" name="TV/SD"/>
      <subcat id="5040" name="TV/HD"/>
      <subcat id="5045" name="TV/UHD"/>
      <subcat id="5050" name="TV/Other"/>
      <subcat id="5060" name="TV/Sport"/>
      <subcat id="5070" name="TV/Anime"/>
      <subcat id="5080" name="TV/Documentary"/>
      <subcat id="5090" name="TV/x265"/>
    </category>
  </categories>
</caps>"#;
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                caps_xml,
            )
                .into_response()
        }
        "search" | "movie" | "tvsearch" => {
            let query_text = params.q.as_deref().unwrap_or("").trim();
            if query_text.is_empty() {
                // Solicitud de validación de conectividad del indexador desde *arr
                return sample_validation_feed_response();
            }

            tracing::info!(
                "Torznab search request: t='{t}', q='{query_text}', season={:?}, ep={:?}",
                params.season,
                params.ep
            );

            let cache_key = query_text.to_lowercase();
            let cached = SEARCH_CACHE.lock().ok().and_then(|mut lock| {
                if lock.len() > 100 {
                    lock.retain(|_, v| v.timestamp.elapsed() < CACHE_TTL);
                }
                lock.get(&cache_key).and_then(|entry| {
                    if entry.timestamp.elapsed() < CACHE_TTL {
                        Some(entry.results.clone())
                    } else {
                        None
                    }
                })
            });

            let all_results = if let Some(cached_results) = cached {
                tracing::info!(
                    "Torznab search cache HIT for query '{}' ({} items)",
                    query_text,
                    cached_results.len()
                );
                cached_results
            } else {
                let mut results = Vec::new();
                let mut seen_hashes = HashSet::new();

                // 1. Buscar directamente en Kad el texto de la consulta
                if let Ok(kad_res) = state.kad.search_keywords(query_text, None, None).await {
                    for r in kad_res {
                        let hash = r["fileHash"].as_str().unwrap_or("").to_lowercase();
                        let name = r["fileName"].as_str().unwrap_or("");
                        let size = r["fileSize"].as_u64().unwrap_or(0);
                        let sources = r["sourceCount"].as_u64().unwrap_or(1);

                        if !hash.is_empty() && !name.is_empty() && seen_hashes.insert(hash.clone()) {
                            results.push((hash, name.to_string(), size, sources));
                        }
                    }
                }

                if let Ok(mut lock) = SEARCH_CACHE.lock() {
                    lock.insert(
                        cache_key,
                        CachedSearchResult {
                            timestamp: Instant::now(),
                            results: results.clone(),
                        },
                    );
                }
                results
            };

            // Extraer palabras de la consulta para el filtrado de relevancia
            let query_words: Vec<String> = query_text
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() >= 3)
                .map(|w| w.to_lowercase())
                .filter(|w| !is_common_stopword(w))
                .collect();

            let mut matched_results: Vec<_> = all_results
                .into_iter()
                .filter(|(_hash, name, _size, _sources)| {
                    let name_lower = name.to_lowercase();
                    if query_words.is_empty() {
                        true
                    } else {
                        let match_count = query_words.iter().filter(|w| name_lower.contains(*w)).count();
                        if query_words.len() <= 2 {
                            match_count == query_words.len()
                        } else {
                            match_count >= 2 && match_count * 2 >= query_words.len()
                        }
                    }
                })
                .collect();

            // Ordenar resultados coincidentes por número de fuentes descendente, luego por tamaño
            matched_results.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| b.2.cmp(&a.2)));

            // Aplicar paginación
            let offset = params.offset.unwrap_or(0);
            let limit = params.limit.unwrap_or(100);
            let page: Vec<_> = matched_results.into_iter().skip(offset).take(limit).collect();

            tracing::info!(
                "Torznab returning {} results for '{}'",
                page.len(),
                query_text
            );

            // Generar XML RSS de Torznab
            let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="1.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:torznab="http://torznab.com/schemas/2015/feed">
  <channel>
    <title>eMule-Rust Torznab Indexer</title>
    <description>eMule-Rust eD2k/Kad Search Results</description>
    <link>http://localhost</link>
    <language>es-es</language>
"#);

            for (hash, name, size, sources) in page {
                let esc_title = escape_xml(&name);
                // Formato magnet con relleno amutorrent / Servarr:
                // urn:btih:<32-hex-hash>00000000
                let padded_btih = format!("{hash}00000000");
                let magnet_url = format!(
                    "magnet:?xt=urn:btih:{}&dn={}&xl={}",
                    padded_btih,
                    url_encode(&name),
                    size
                );
                let esc_magnet = escape_xml(&magnet_url);
                let cat_id = if t == "tvsearch" { "5040" } else { "2000" };

                xml.push_str(&format!(
                    r#"    <item>
      <title>{esc_title}</title>
      <guid isPermaLink="false">{hash}</guid>
      <pubDate>Sun, 01 Nov 2026 00:00:00 +0000</pubDate>
      <size>{size}</size>
      <link>{esc_magnet}</link>
      <enclosure url="{esc_magnet}" length="{size}" type="application/x-bittorrent" />
      <torznab:attr name="category" value="{cat_id}"/>
      <torznab:attr name="peers" value="{sources}"/>
      <torznab:attr name="seeders" value="{sources}"/>
    </item>
"#
                ));
            }

            xml.push_str("  </channel>\n</rss>");

            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                xml,
            )
                .into_response()
        }
        _ => empty_feed_response(),
    }
}
