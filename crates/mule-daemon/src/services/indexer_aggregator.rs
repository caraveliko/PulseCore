// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use futures_util::future::join_all;
use reqwest::Client;
use tokio::sync::Mutex;

use crate::services::downloads::now_ms;
use crate::services::scorer::{evaluate_release, ScoredRelease};
use crate::state::AppState;

/// Tiempo de espera HTTP por indexador Torznab (en segundos).
const INDEXER_HTTP_TIMEOUT_SECS: u64 = 10;
/// Tiempo de vida de la caché de resultados de indexadores en memoria (10 minutos).
const SEARCH_CACHE_TTL_MS: i64 = 10 * 60 * 1000;
/// Número máximo de peticiones conservadas en la caché de indexadores.
const SEARCH_CACHE_MAX_ENTRIES: usize = 256;

#[derive(Clone)]
struct CachedSearchEntry {
    timestamp_ms: i64,
    results: Vec<ScoredRelease>,
}

static SHARED_HTTP_CLIENT: OnceLock<Client> = OnceLock::new();
static SHARED_SEARCH_CACHE: OnceLock<Mutex<HashMap<String, CachedSearchEntry>>> = OnceLock::new();

fn shared_http_client() -> &'static Client {
    SHARED_HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(INDEXER_HTTP_TIMEOUT_SECS))
            .user_agent(concat!("eMule-Rust/", env!("CARGO_PKG_VERSION"), " MediaManager"))
            .build()
            .unwrap_or_default()
    })
}

fn shared_search_cache() -> &'static Mutex<HashMap<String, CachedSearchEntry>> {
    SHARED_SEARCH_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct IndexerAggregator {
    http: Client,
}

impl Default for IndexerAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexerAggregator {
    pub fn new() -> Self {
        Self {
            http: shared_http_client().clone(),
        }
    }

    fn build_cache_key(query: &str, is_movie: bool, quality_profile: &str) -> String {
        format!(
            "{}|{}|{}",
            query.trim().to_lowercase(),
            if is_movie { "m" } else { "tv" },
            quality_profile.trim().to_lowercase()
        )
    }

    /// Vacía explícitamente la caché en memoria del agregador (útil en caso de forzado manual).
    pub async fn invalidate_cache(&self) {
        let mut guard = shared_search_cache().lock().await;
        guard.clear();
    }

    /// Busca en Kad DHT + todos los indexadores externos Torznab configurados en paralelo
    pub async fn search_all(
        &self,
        state: &AppState,
        query: &str,
        is_movie: bool,
        quality_profile: &str,
    ) -> Vec<ScoredRelease> {
        if state.network_policy.state() == mule_network_policy::NetworkState::Disabled {
            return Vec::new();
        }

        let now = now_ms();
        let cache_key = Self::build_cache_key(query, is_movie, quality_profile);
        {
            let guard = shared_search_cache().lock().await;
            if let Some(entry) = guard.get(&cache_key) {
                if now.saturating_sub(entry.timestamp_ms) < SEARCH_CACHE_TTL_MS {
                    return entry.results.clone();
                }
            }
        }

        // 1. Instantánea de los indexadores externos activados (bloqueo SQLite muy corto)
        let enabled_indexers: Vec<_> = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .db()
                .list_external_indexers()
                .unwrap_or_default()
                .into_iter()
                .filter(|idx| idx.enabled)
                .collect()
        };

        let encoded_q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let cat = if is_movie { "2000" } else { "5000" };
        let profile_owned = quality_profile.to_string();

        // 2. Preparar la petición Kad DHT y todas las peticiones HTTP Torznab en paralelo
        let kad_fut = state.kad.search_keywords(query, None, None);

        let torznab_futs = enabled_indexers.into_iter().map(|idx| {
            let client = self.http.clone();
            let eq = encoded_q.clone();
            let prof = profile_owned.clone();
            async move {
                let sep = if idx.base_url.contains('?') { "&" } else { "?" };
                let url = format!(
                    "{}{sep}t=search&q={eq}&cat={cat}&apikey={}",
                    idx.base_url, idx.api_key
                );
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(xml_text) = resp.text().await {
                        return parse_torznab_feed(&xml_text, &idx.name, is_movie, &prof);
                    }
                }
                Vec::new()
            }
        });

        let (kad_outcome, torznab_batches) = tokio::join!(kad_fut, join_all(torznab_futs));

        let mut results = Vec::new();
        let mut seen_hashes = HashSet::new();

        if let Ok(kad_res) = kad_outcome {
            for r in kad_res {
                let hash = r["fileHash"].as_str().unwrap_or("").to_lowercase();
                let name = r["fileName"].as_str().unwrap_or("");
                let size = r["fileSize"].as_u64().unwrap_or(0);
                let sources = r["sourceCount"].as_u64().unwrap_or(1);

                if !hash.is_empty() && !name.is_empty() && seen_hashes.insert(hash.clone()) {
                    let ed2k_link = format!("ed2k://|file|{name}|{size}|{hash}|/");
                    let evaluated = evaluate_release(
                        name,
                        "ed2k",
                        &ed2k_link,
                        &hash,
                        size,
                        sources,
                        "eD2k/Kad",
                        is_movie,
                        quality_profile,
                        "es",
                    );
                    results.push(evaluated);
                }
            }
        }

        for batch in torznab_batches {
            for r in batch {
                if seen_hashes.insert(r.hash.clone()) {
                    results.push(r);
                }
            }
        }

        // Ordenar resultados: versiones válidas primero, ordenadas por puntuación descendente, luego fuentes descendentes
        results.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.sources.cmp(&a.sources))
                .then_with(|| b.size_bytes.cmp(&a.size_bytes))
        });

        if !results.is_empty() {
            let mut guard = shared_search_cache().lock().await;
            if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                guard.retain(|_, v| now.saturating_sub(v.timestamp_ms) < SEARCH_CACHE_TTL_MS);
                if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                    guard.clear();
                }
            }
            guard.insert(
                cache_key,
                CachedSearchEntry {
                    timestamp_ms: now,
                    results: results.clone(),
                },
            );
        }

        results
    }
}

/// Analizador ligero para fuentes RSS XML de Torznab/Newznab
fn parse_torznab_feed(
    xml: &str,
    indexer_name: &str,
    is_movie: bool,
    quality_profile: &str,
) -> Vec<ScoredRelease> {
    let mut out = Vec::new();
    let items: Vec<&str> = xml.split("<item>").skip(1).collect();

    for raw in items {
        let item_str = match raw.split("</item>").next() {
            Some(s) => s,
            None => continue,
        };

        let title = extract_xml_tag(item_str, "title").unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let link = extract_xml_tag(item_str, "link")
            .or_else(|| extract_xml_attr(item_str, "enclosure", "url"))
            .unwrap_or_default();

        let size: u64 = extract_xml_tag(item_str, "size")
            .and_then(|s| s.parse().ok())
            .or_else(|| {
                extract_xml_attr(item_str, "enclosure", "length").and_then(|s| s.parse().ok())
            })
            .unwrap_or(0);

        let seeders: u64 = extract_torznab_attr(item_str, "seeders")
            .or_else(|| extract_torznab_attr(item_str, "peers"))
            .unwrap_or(1);

        // Extraer hash de enlace magnet (xt=urn:btih:<hash>) o guid
        let hash = if let Some(pos) = link.to_lowercase().find("urn:btih:") {
            let rest = &link[pos + 9..];
            rest.split('&').next().unwrap_or("").to_lowercase()
        } else {
            extract_xml_tag(item_str, "guid").unwrap_or_else(|| title.clone())
        };

        let protocol = if link.starts_with("ed2k://") {
            "ed2k"
        } else {
            "torrent"
        };

        let evaluated = evaluate_release(
            &title,
            protocol,
            &link,
            &hash,
            size,
            seeders,
            indexer_name,
            is_movie,
            quality_profile,
            "es",
        );

        out.push(evaluated);
    }

    out
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let val = xml[start..end].trim();
    // Decodificar entidades básicas
    Some(
        val.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("<![CDATA[", "")
            .replace("]]>", ""),
    )
}

fn extract_xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    let end = xml[start..].find('>')? + start;
    let tag_content = &xml[start..end];

    let needle = format!("{attr}=\"");
    let a_start = tag_content.find(&needle)? + needle.len();
    let a_end = tag_content[a_start..].find('"')? + a_start;
    Some(tag_content[a_start..a_end].to_string())
}

fn extract_torznab_attr(xml: &str, attr_name: &str) -> Option<u64> {
    let needle = format!("name=\"{attr_name}\" value=\"");
    let start = xml.find(&needle)? + needle.len();
    let end = xml[start..].find('"')? + start;
    xml[start..end].parse().ok()
}
