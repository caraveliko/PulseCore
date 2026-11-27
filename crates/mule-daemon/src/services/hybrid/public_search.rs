// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Metabuscador público ligero de `info_hash` BitTorrent sin necesidad de indexadores Torznab.
//!
//! Consulta APIs JSON abiertas (`apibay`, `yts`) para obtener `info_hash` candidatos por título,
//! aplicando estrictamente la política de red (`NetworkPolicyManager`) y el filtro anti-abuso/adulto
//! antes de resolver cualquier metadato directamente por P2P (`ut_metadata` / Mainline DHT).

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use mule_network_policy::NetworkState;
use mule_torrent::is_blocked_abusive_or_adult;
use reqwest::Client;
use serde::Deserialize;

use crate::state::AppState;

use super::kad_bridge::normalize_btih_token;

/// Tiempo máximo de espera HTTP para consultas al metabuscador público de `info_hash`.
const PUBLIC_BT_HTTP_TIMEOUT: Duration = Duration::from_secs(6);
/// Punto de acceso JSON público de `apibay.org`.
const APIBAY_SEARCH_URL: &str = "https://apibay.org/q.php";
/// Categoría de vídeo de `apibay.org` (200 = Video).
const APIBAY_VIDEO_CATEGORY: &str = "200";
/// Punto de acceso JSON público de películas YTS v2.
const YTS_LIST_MOVIES_URL: &str = "https://yts.mx/api/v2/list_movies.json";
/// Límite máximo de películas solicitadas por consulta a YTS.
const YTS_MAX_MOVIES: usize = 10;
/// Número máximo de candidatos devueltos por ronda tras deduplicar y ordenar por semillas.
const MAX_PUBLIC_BT_RESULTS: usize = 40;
/// Hash centinela devuelto por `apibay.org` cuando no existen resultados.
const ZERO_INFOHASH: &str = "0000000000000000000000000000000000000000";
/// Sufijos de idioma añadidos al metabuscador público BT cuando la consulta no especifica idioma español.
const SPANISH_SEARCH_SUFFIXES: &[&str] = &["Spanish", "Castellano"];

static SHARED_PUBLIC_HTTP: OnceLock<Client> = OnceLock::new();

fn public_http_client() -> &'static Client {
    SHARED_PUBLIC_HTTP.get_or_init(|| {
        Client::builder()
            .timeout(PUBLIC_BT_HTTP_TIMEOUT)
            .user_agent(concat!("eMule-Rust/", env!("CARGO_PKG_VERSION"), " HybridSwarm"))
            .build()
            .unwrap_or_default()
    })
}

/// Candidato BitTorrent obtenido desde un metabuscador público de `info_hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicBtCandidate {
    pub info_hash: String,
    pub name: String,
    pub size_bytes: u64,
    pub seeders: u64,
    pub source: &'static str,
}

#[derive(Deserialize)]
struct ApibayItem {
    #[serde(default)]
    name: String,
    #[serde(default)]
    info_hash: String,
    #[serde(default)]
    size: serde_json::Value,
    #[serde(default)]
    seeders: serde_json::Value,
}

#[derive(Deserialize)]
struct YtsResponse {
    data: Option<YtsData>,
}

#[derive(Deserialize)]
struct YtsData {
    #[serde(default)]
    movies: Vec<YtsMovie>,
}

#[derive(Deserialize)]
struct YtsMovie {
    #[serde(default)]
    title_long: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    torrents: Vec<YtsTorrent>,
}

#[derive(Deserialize)]
struct YtsTorrent {
    #[serde(default)]
    hash: String,
    #[serde(default)]
    quality: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    size_bytes: u64,
    #[serde(default)]
    seeds: u64,
}

fn parse_json_u64(val: &serde_json::Value) -> u64 {
    if let Some(n) = val.as_u64() {
        return n;
    }
    if let Some(s) = val.as_str() {
        return s.trim().parse::<u64>().unwrap_or(0);
    }
    0
}

/// Analiza la respuesta JSON de `apibay.org` descartando el centinela cero y contenido bloqueado.
pub fn parse_apibay_json(body: &str) -> Vec<PublicBtCandidate> {
    let Ok(items) = serde_json::from_str::<Vec<ApibayItem>>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(hash) = normalize_btih_token(item.info_hash.trim()) else {
            continue;
        };
        if hash == ZERO_INFOHASH {
            continue;
        }
        let name = item.name.trim().to_string();
        if name.is_empty() || is_blocked_abusive_or_adult(&name) {
            continue;
        }
        out.push(PublicBtCandidate {
            info_hash: hash,
            name,
            size_bytes: parse_json_u64(&item.size),
            seeders: parse_json_u64(&item.seeders),
            source: "apibay",
        });
    }
    out
}

/// Analiza la respuesta JSON de `yts.mx` descartando hashes inválidos y contenido bloqueado.
pub fn parse_yts_json(body: &str) -> Vec<PublicBtCandidate> {
    let Ok(resp) = serde_json::from_str::<YtsResponse>(body) else {
        return Vec::new();
    };
    let Some(data) = resp.data else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for movie in data.movies {
        let base_title = if movie.title_long.trim().is_empty() {
            movie.title.trim()
        } else {
            movie.title_long.trim()
        };
        if base_title.is_empty() || is_blocked_abusive_or_adult(base_title) {
            continue;
        }
        for tor in movie.torrents {
            let Some(hash) = normalize_btih_token(tor.hash.trim()) else {
                continue;
            };
            let label = match (tor.quality.trim(), tor.kind.trim()) {
                ("", "") => base_title.to_string(),
                (q, "") => format!("{base_title} [{q}]"),
                ("", k) => format!("{base_title} [{k}]"),
                (q, k) => format!("{base_title} [{q} {k}]"),
            };
            if is_blocked_abusive_or_adult(&label) {
                continue;
            }
            out.push(PublicBtCandidate {
                info_hash: hash,
                name: label,
                size_bytes: tor.size_bytes,
                seeders: tor.seeds,
                source: "yts",
            });
        }
    }
    out
}

/// Genera variantes de consulta enfocadas al idioma español más la consulta original como respaldo.
pub fn build_public_bt_queries(query: &str) -> Vec<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("spanish")
        || lower.contains("castellano")
        || lower.contains("español")
        || lower.contains("dual")
    {
        return vec![trimmed.to_string()];
    }
    let mut out = Vec::with_capacity(SPANISH_SEARCH_SUFFIXES.len() + 1);
    for suffix in SPANISH_SEARCH_SUFFIXES {
        out.push(format!("{trimmed} {suffix}"));
    }
    out.push(trimmed.to_string());
    out
}

/// Consulta fuentes públicas abiertas de `info_hash` para una consulta limpia de vídeo.
///
/// Respeta el interruptor de corte y el aislamiento `LocalLab` sin emitir tráfico en perfiles aislados.
pub async fn search_public_bt_sources(
    state: &AppState,
    query: &str,
    is_movie: bool,
) -> Vec<PublicBtCandidate> {
    let trimmed = query.trim();
    if trimmed.is_empty()
        || state.network_policy.profile().is_loopback_only()
        || matches!(
            state.network_policy.state(),
            NetworkState::Disabled | NetworkState::ShuttingDown
        )
    {
        return Vec::new();
    }

    let client = public_http_client();
    let queries = build_public_bt_queries(trimmed);
    let encoded_main: String = url::form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();

    let apibay_futs = queries.into_iter().map(|q| async move {
        let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
        let url = format!("{APIBAY_SEARCH_URL}?q={encoded}&cat={APIBAY_VIDEO_CATEGORY}");
        let Ok(resp) = client.get(&url).send().await else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            return Vec::new();
        }
        let Ok(text) = resp.text().await else {
            return Vec::new();
        };
        parse_apibay_json(&text)
    });

    let yts_fut = async {
        if !is_movie {
            return Vec::new();
        }
        let url =
            format!("{YTS_LIST_MOVIES_URL}?query_term={encoded_main}&limit={YTS_MAX_MOVIES}");
        let Ok(resp) = client.get(&url).send().await else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            return Vec::new();
        }
        let Ok(text) = resp.text().await else {
            return Vec::new();
        };
        parse_yts_json(&text)
    };

    let (apibay_batches, yts_hits) =
        tokio::join!(futures_util::future::join_all(apibay_futs), yts_fut);

    let mut merged: HashMap<String, PublicBtCandidate> = HashMap::new();
    for cand in apibay_batches.into_iter().flatten().chain(yts_hits) {
        merged
            .entry(cand.info_hash.clone())
            .and_modify(|existing| {
                if cand.seeders > existing.seeders {
                    *existing = cand.clone();
                }
            })
            .or_insert(cand);
    }

    let mut out: Vec<PublicBtCandidate> = merged.into_values().collect();
    out.sort_by(|a, b| b.seeders.cmp(&a.seeders).then_with(|| a.name.cmp(&b.name)));
    out.truncate(MAX_PUBLIC_BT_RESULTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apibay_and_yts_json_and_filters_sentinel_and_adult() {
        let apibay_sample = r#"[
            {"name":"No results returned","info_hash":"0000000000000000000000000000000000000000","size":"0","seeders":"0"},
            {"name":"Interstellar 2014 1080p BluRay x264","info_hash":"ABCDEF0123456789ABCDEF0123456789ABCDEF01","size":"2110599168","seeders":"42"},
            {"name":"Pornhub XXX Adult Clip","info_hash":"1111111111111111111111111111111111111111","size":"500000000","seeders":"99"}
        ]"#;
        let apibay = parse_apibay_json(apibay_sample);
        assert_eq!(apibay.len(), 1);
        assert_eq!(
            apibay[0].info_hash,
            "abcdef0123456789abcdef0123456789abcdef01"
        );
        assert_eq!(apibay[0].size_bytes, 2_110_599_168);
        assert_eq!(apibay[0].seeders, 42);
        assert_eq!(apibay[0].source, "apibay");

        let yts_sample = r#"{
            "data": {
                "movies": [
                    {
                        "title_long": "Interstellar (2014)",
                        "torrents": [
                            {"hash": "9988776655443322110099887766554433221100", "quality": "1080p", "type": "bluray", "size_bytes": 2400000000, "seeds": 120}
                        ]
                    }
                ]
            }
        }"#;
        let yts = parse_yts_json(yts_sample);
        assert_eq!(yts.len(), 1);
        assert_eq!(yts[0].name, "Interstellar (2014) [1080p bluray]");
        assert_eq!(yts[0].seeders, 120);
        assert_eq!(yts[0].source, "yts");
    }

    #[test]
    fn builds_spanish_focused_public_bt_queries() {
        assert_eq!(
            build_public_bt_queries("South Park"),
            vec![
                "South Park Spanish".to_string(),
                "South Park Castellano".to_string(),
                "South Park".to_string(),
            ]
        );
        assert_eq!(
            build_public_bt_queries("South Park Castellano"),
            vec!["South Park Castellano".to_string()]
        );
    }
}
