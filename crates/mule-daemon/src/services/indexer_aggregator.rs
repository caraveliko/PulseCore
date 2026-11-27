// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use futures_util::future::join_all;
use reqwest::Client;
use tokio::sync::{broadcast, watch, Mutex};

use crate::services::downloads::now_ms;
use crate::services::scorer::{evaluate_release, ScoredRelease};
use crate::services::server_session::ServerConnectionStatus;
use crate::state::AppState;

/// Tiempo de espera HTTP por indexador Torznab (en segundos).
const INDEXER_HTTP_TIMEOUT_SECS: u64 = 10;
/// Tiempo de vida de la caché de resultados de indexadores en memoria (10 minutos).
const SEARCH_CACHE_TTL_MS: i64 = 10 * 60 * 1000;
/// Número máximo de peticiones conservadas en la caché de indexadores.
const SEARCH_CACHE_MAX_ENTRIES: usize = 256;
/// Espaciado mínimo entre búsquedas `OP_SEARCHREQUEST` hacia el servidor eD2K para evitar límites de tasa (1,2 s).
const SERVER_SEARCH_MIN_INTERVAL_MS: i64 = 1_200;
/// Espera máxima total para recibir resultados `server.searchResult` del servidor eD2K (2,5 s).
const SERVER_SEARCH_WAIT_MAX_MS: u64 = 2_500;
/// Ventana de silencio tras recibir un lote del servidor eD2K antes de cerrar la recogida (350 ms).
const SERVER_SEARCH_IDLE_AFTER_HIT_MS: u64 = 350;
/// Número máximo de resultados recuperados del catálogo DHT local sin indexadores por consulta.
const DHT_LOCAL_SEARCH_LIMIT: usize = 50;

#[derive(Clone)]
struct CachedSearchEntry {
    timestamp_ms: i64,
    results: Vec<ScoredRelease>,
}

#[derive(Clone)]
struct CachedRawEd2kEntry {
    timestamp_ms: i64,
    results: Vec<serde_json::Value>,
}

static SHARED_HTTP_CLIENT: OnceLock<Client> = OnceLock::new();
static SHARED_SEARCH_CACHE: OnceLock<Mutex<HashMap<String, CachedSearchEntry>>> = OnceLock::new();
static SHARED_KAD_KEYWORD_CACHE: OnceLock<Mutex<HashMap<String, CachedRawEd2kEntry>>> =
    OnceLock::new();
static SHARED_SERVER_SEARCH_CACHE: OnceLock<Mutex<HashMap<String, CachedRawEd2kEntry>>> =
    OnceLock::new();
/// Puerta de serialización y cadencia para búsquedas en el servidor eD2K conectado (almacena marca temporal ms).
static SHARED_SERVER_SEARCH_GATE: OnceLock<Mutex<i64>> = OnceLock::new();
type KadInflightRx = watch::Receiver<Option<Vec<serde_json::Value>>>;
type KadInflightMap = std::sync::Mutex<HashMap<String, KadInflightRx>>;

/// Registro de búsquedas Kad DHT en curso por clave para deduplicar consultas concurrentes (*singleflight*).
static SHARED_KAD_INFLIGHT: OnceLock<KadInflightMap> = OnceLock::new();

fn shared_kad_inflight() -> &'static KadInflightMap {
    SHARED_KAD_INFLIGHT.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

struct KadInflightGuard {
    key: String,
    tx: Option<watch::Sender<Option<Vec<serde_json::Value>>>>,
}

impl Drop for KadInflightGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = shared_kad_inflight().lock() {
            guard.remove(&self.key);
        }
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Some(Vec::new()));
        }
    }
}

enum KadInflightSlot {
    Leader(KadInflightGuard),
    Follower(watch::Receiver<Option<Vec<serde_json::Value>>>),
}

fn acquire_kad_inflight_slot(kad_key: &str) -> KadInflightSlot {
    let mut guard = match shared_kad_inflight().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(rx) = guard.get(kad_key).cloned() {
        return KadInflightSlot::Follower(rx);
    }
    let (tx, rx) = watch::channel(None);
    guard.insert(kad_key.to_string(), rx);
    KadInflightSlot::Leader(KadInflightGuard {
        key: kad_key.to_string(),
        tx: Some(tx),
    })
}

fn shared_http_client() -> &'static Client {
    SHARED_HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(INDEXER_HTTP_TIMEOUT_SECS))
            .user_agent(concat!("eMule-Rust/", env!("CARGO_PKG_VERSION"), " MediaManager"))
            .build()
            .unwrap_or_default()
    })
}

fn shared_search_cache() -> &'static Mutex<HashMap<String, CachedSearchEntry>> {
    SHARED_SEARCH_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared_kad_keyword_cache() -> &'static Mutex<HashMap<String, CachedRawEd2kEntry>> {
    SHARED_KAD_KEYWORD_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared_server_search_cache() -> &'static Mutex<HashMap<String, CachedRawEd2kEntry>> {
    SHARED_SERVER_SEARCH_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared_server_search_gate() -> &'static Mutex<i64> {
    SHARED_SERVER_SEARCH_GATE.get_or_init(|| Mutex::new(0))
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
        {
            let mut guard = shared_search_cache().lock().await;
            guard.clear();
        }
        {
            let mut kad_guard = shared_kad_keyword_cache().lock().await;
            kad_guard.clear();
        }
        {
            let mut srv_guard = shared_server_search_cache().lock().await;
            srv_guard.clear();
        }
    }

    /// Consulta el servidor eD2K conectado (`OP_SEARCHREQUEST` con filtro de vídeo) con caché
    /// y serialización respetando el espaciado mínimo para no activar límites de tasa del servidor.
    async fn search_ed2k_server(state: &AppState, query: &str) -> Vec<serde_json::Value> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        if state.server_session.snapshot().await.status != ServerConnectionStatus::Connected {
            return Vec::new();
        }

        let cache_key = trimmed.to_lowercase();
        let now = now_ms();
        {
            let guard = shared_server_search_cache().lock().await;
            if let Some(entry) = guard.get(&cache_key) {
                if now.saturating_sub(entry.timestamp_ms) < SEARCH_CACHE_TTL_MS {
                    return entry.results.clone();
                }
            }
        }

        let mut last_ts = shared_server_search_gate().lock().await;

        // Doble comprobación de caché tras adquirir la puerta por si otra tarea acaba de consultarlo.
        let after_lock_now = now_ms();
        {
            let guard = shared_server_search_cache().lock().await;
            if let Some(entry) = guard.get(&cache_key) {
                if after_lock_now.saturating_sub(entry.timestamp_ms) < SEARCH_CACHE_TTL_MS {
                    return entry.results.clone();
                }
            }
        }

        if state.server_session.snapshot().await.status != ServerConnectionStatus::Connected {
            return Vec::new();
        }

        let elapsed = after_lock_now.saturating_sub(*last_ts);
        if *last_ts > 0 && elapsed < SERVER_SEARCH_MIN_INTERVAL_MS {
            tokio::time::sleep(Duration::from_millis(
                (SERVER_SEARCH_MIN_INTERVAL_MS - elapsed) as u64,
            ))
            .await;
        }

        let mut rx = state.event_tx.subscribe();
        let mut sq = mule_protocol::ed2k::SearchQuery::new(trimmed);
        sq.file_type = Some("Video".to_string());

        *last_ts = now_ms();
        if state.server_session.search(sq).await.is_err() {
            return Vec::new();
        }

        let mut collected = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(SERVER_SEARCH_WAIT_MAX_MS);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let wait_for = if collected.is_empty() {
                remaining
            } else {
                remaining.min(Duration::from_millis(SERVER_SEARCH_IDLE_AFTER_HIT_MS))
            };
            match tokio::time::timeout(wait_for, rx.recv()).await {
                Ok(Ok(raw_msg)) => {
                    if !raw_msg.contains("\"server.searchResult\"") {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw_msg) {
                        if parsed["method"].as_str() == Some("server.searchResult") {
                            if let Some(files) = parsed["params"]["files"].as_array() {
                                collected.extend(files.iter().cloned());
                            }
                        }
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }

        {
            let mut guard = shared_server_search_cache().lock().await;
            if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                if let Some(oldest_key) = guard
                    .iter()
                    .min_by_key(|(_, e)| e.timestamp_ms)
                    .map(|(k, _)| k.clone())
                {
                    guard.remove(&oldest_key);
                }
            }
            guard.insert(
                cache_key,
                CachedRawEd2kEntry {
                    timestamp_ms: now_ms(),
                    results: collected.clone(),
                },
            );
        }

        collected
    }

    /// Busca en Kad DHT + servidor eD2K conectado + indexadores externos Torznab en paralelo.
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
        let enabled_indexers = Self::load_enabled_indexers(state).await;

        let profile_owned = quality_profile.to_string();

        // 2. Preparar la petición Kad DHT (con caché por palabra clave primaria Kad) y peticiones Torznab en paralelo
        let (_, kad_id) = crate::services::kad::search_keywords::select_best_kad_keyword(query);
        let kad_key = kad_id.to_hex();
        let cached_kad = {
            let guard = shared_kad_keyword_cache().lock().await;
            guard.get(&kad_key).and_then(|entry| {
                if now.saturating_sub(entry.timestamp_ms) < SEARCH_CACHE_TTL_MS {
                    Some(entry.results.clone())
                } else {
                    None
                }
            })
        };

        let kad_fut = async {
            if let Some(hit) = cached_kad {
                return Ok(hit);
            }
            let mut inflight_guard = match acquire_kad_inflight_slot(&kad_key) {
                KadInflightSlot::Follower(mut rx) => {
                    let _ = rx.wait_for(|v| v.is_some()).await;
                    return Ok(rx.borrow().clone().unwrap_or_default());
                }
                KadInflightSlot::Leader(guard) => guard,
            };

            let res = state.kad.search_keywords(query, None, None).await;
            let items_to_notify = match &res {
                Ok(items) => {
                    if !items.is_empty() {
                        let mut guard = shared_kad_keyword_cache().lock().await;
                        if guard.len() >= SEARCH_CACHE_MAX_ENTRIES {
                            if let Some(oldest_key) = guard
                                .iter()
                                .min_by_key(|(_, e)| e.timestamp_ms)
                                .map(|(k, _)| k.clone())
                            {
                                guard.remove(&oldest_key);
                            }
                        }
                        guard.insert(
                            kad_key,
                            CachedRawEd2kEntry {
                                timestamp_ms: now_ms(),
                                results: items.clone(),
                            },
                        );
                    }
                    items.clone()
                }
                Err(_) => Vec::new(),
            };
            if let Ok(mut guard) = shared_kad_inflight().lock() {
                guard.remove(&inflight_guard.key);
            }
            if let Some(tx) = inflight_guard.tx.take() {
                let _ = tx.send(Some(items_to_notify));
            }
            res
        };

        let server_fut = Self::search_ed2k_server(state, query);
        let torznab_fut = self.query_torznab_indexers(&enabled_indexers, query, is_movie, &profile_owned);
        let public_bt_fut =
            crate::services::hybrid::public_search::search_public_bt_sources(state, query, is_movie);

        let (kad_outcome, server_outcome, torznab_batches, public_bt_hits) =
            tokio::join!(kad_fut, server_fut, torznab_fut, public_bt_fut);

        let mut results = Vec::new();
        let mut seen_hashes = HashSet::new();

        // Fusionar resultados eD2K de Kad DHT y del servidor eD2K por hash conservando el máximo de fuentes reales.
        let mut ed2k_merged: HashMap<String, (String, u64, u64, bool, bool)> = HashMap::new();
        let mut insert_ed2k_raw = |items: Vec<serde_json::Value>, is_server: bool| {
            for r in items {
                let hash = r["fileHash"].as_str().unwrap_or("").to_lowercase();
                let name = r["fileName"].as_str().unwrap_or("");
                let size = r["fileSize"].as_u64().unwrap_or(0);
                let sources = r["sourceCount"].as_u64().unwrap_or(1);
                if hash.is_empty() || name.is_empty() {
                    continue;
                }
                ed2k_merged
                    .entry(hash)
                    .and_modify(|e| {
                        if e.1 == 0 && size > 0 {
                            e.1 = size;
                        }
                        if sources > e.2 {
                            e.2 = sources;
                        }
                        if is_server {
                            e.4 = true;
                        } else {
                            e.3 = true;
                        }
                    })
                    .or_insert_with(|| (name.to_string(), size, sources, !is_server, is_server));
            }
        };

        if let Ok(kad_res) = kad_outcome {
            insert_ed2k_raw(kad_res, false);
        }
        if !server_outcome.is_empty() {
            insert_ed2k_raw(server_outcome, true);
        }

        for (hash, (name, size, sources, from_kad, from_server)) in ed2k_merged {
            if seen_hashes.insert(hash.clone()) {
                let source_label = match (from_server, from_kad) {
                    (true, true) => "eD2k/Server+Kad",
                    (true, false) => "eD2k/Server",
                    _ => "eD2k/Kad",
                };
                let ed2k_link = format!("ed2k://|file|{name}|{size}|{hash}|/");
                let evaluated = evaluate_release(
                    &name,
                    "ed2k",
                    &ed2k_link,
                    &hash,
                    size,
                    sources,
                    source_label,
                    is_movie,
                    quality_profile,
                    "es",
                );
                results.push(evaluated);
            }
        }

        for batch in torznab_batches {
            for r in batch {
                if seen_hashes.insert(r.hash.clone()) {
                    results.push(r);
                }
            }
        }

        // Incorporar resultados del metabuscador público BitTorrent (apibay / yts) sin necesidad de Torznab.
        for cand in public_bt_hits {
            if seen_hashes.insert(cand.info_hash.clone()) {
                let magnet = format!(
                    "magnet:?xt=urn:btih:{}&dn={}",
                    cand.info_hash,
                    url::form_urlencoded::byte_serialize(cand.name.as_bytes()).collect::<String>()
                );
                let source_label = format!("BT/{}", cand.source);
                let evaluated = evaluate_release(
                    &cand.name,
                    "torrent",
                    &magnet,
                    &cand.info_hash,
                    cand.size_bytes,
                    cand.seeders,
                    &source_label,
                    is_movie,
                    quality_profile,
                    "es",
                );
                results.push(evaluated);
            }
        }

        // Incorporar resultados del catálogo Mainline DHT local (0 indexadores externos).
        let local_dht_entries = {
            let dl_svc = state.downloads.lock().await;
            dl_svc
                .db()
                .search_dht_catalog_by_tokens(query, DHT_LOCAL_SEARCH_LIMIT)
                .unwrap_or_default()
        };
        for entry in local_dht_entries {
            if seen_hashes.insert(entry.info_hash.clone()) {
                let magnet = format!(
                    "magnet:?xt=urn:btih:{}&dn={}",
                    entry.info_hash,
                    url::form_urlencoded::byte_serialize(entry.name.as_bytes()).collect::<String>()
                );
                let title = if entry.video_file_name.is_empty() {
                    &entry.name
                } else {
                    &entry.video_file_name
                };
                let evaluated = evaluate_release(
                    title,
                    "torrent",
                    &magnet,
                    &entry.info_hash,
                    entry.video_file_size,
                    u64::from(entry.estimated_peers.max(1)),
                    "BT/DHT-Local",
                    is_movie,
                    quality_profile,
                    "es",
                );
                results.push(evaluated);
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

    async fn load_enabled_indexers(state: &AppState) -> Vec<mule_core::model::ExternalIndexer> {
        let dl_svc = state.downloads.lock().await;
        dl_svc
            .db()
            .list_external_indexers()
            .unwrap_or_default()
            .into_iter()
            .filter(|idx| idx.enabled)
            .collect()
    }

    async fn query_torznab_indexers(
        &self,
        enabled_indexers: &[mule_core::model::ExternalIndexer],
        query: &str,
        is_movie: bool,
        quality_profile: &str,
    ) -> Vec<Vec<ScoredRelease>> {
        if enabled_indexers.is_empty() || query.trim().is_empty() {
            return Vec::new();
        }
        let encoded_q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let cat = if is_movie { "2000" } else { "5000" };
        let futs = enabled_indexers.iter().map(|idx| {
            let client = self.http.clone();
            let eq = encoded_q.clone();
            let prof = quality_profile.to_string();
            let base_url = idx.base_url.clone();
            let api_key = idx.api_key.clone();
            let name = idx.name.clone();
            async move {
                let sep = if base_url.contains('?') { "&" } else { "?" };
                let url = format!("{base_url}{sep}t=search&q={eq}&cat={cat}");
                let mut req = client.get(&url);
                if !api_key.is_empty() {
                    req = req.header("X-Api-Key", &api_key);
                }
                if let Ok(resp) = req.send().await {
                    if let Ok(xml_text) = resp.text().await {
                        return parse_torznab_feed(&xml_text, &name, is_movie, &prof);
                    }
                }
                Vec::new()
            }
        });
        join_all(futs).await
    }

    /// Consulta únicamente los indexadores externos Torznab/Prowlarr habilitados y devuelve
    /// `(resultados_torrent_deduplicados, num_indexadores_habilitados)`.
    pub async fn search_torznab_only(
        &self,
        state: &AppState,
        query: &str,
        is_movie: bool,
        quality_profile: &str,
    ) -> (Vec<ScoredRelease>, usize) {
        if matches!(
            state.network_policy.state(),
            mule_network_policy::NetworkState::Disabled
                | mule_network_policy::NetworkState::ShuttingDown
        ) {
            return (Vec::new(), 0);
        }
        let enabled_indexers = Self::load_enabled_indexers(state).await;
        let indexer_count = enabled_indexers.len();
        if indexer_count == 0 {
            return (Vec::new(), 0);
        }
        let batches = self
            .query_torznab_indexers(&enabled_indexers, query, is_movie, quality_profile)
            .await;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for batch in batches {
            for r in batch {
                if r.protocol == "torrent" && !r.hash.is_empty() && seen.insert(r.hash.clone()) {
                    out.push(r);
                }
            }
        }
        (out, indexer_count)
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

        // Extraer hash de enlace magnet (xt=urn:btih:<hex|base32>), atributo Torznab `infohash` o guid
        let raw_hash = if let Some(pos) = link.to_lowercase().find("urn:btih:") {
            let rest = &link[pos + 9..];
            rest.split('&').next().unwrap_or("").to_string()
        } else if let Some(ih) = extract_torznab_str_attr(item_str, "infohash") {
            ih
        } else {
            extract_xml_tag(item_str, "guid").unwrap_or_else(|| title.clone())
        };
        let hash = crate::services::hybrid::kad_bridge::normalize_btih_token(raw_hash.trim())
            .unwrap_or_else(|| raw_hash.trim().to_lowercase());

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
    extract_torznab_str_attr(xml, attr_name).and_then(|s| s.parse().ok())
}

fn extract_torznab_str_attr(xml: &str, attr_name: &str) -> Option<String> {
    let needle = format!("name=\"{attr_name}\" value=\"");
    let start = xml.find(&needle)? + needle.len();
    let end = xml[start..].find('"')? + start;
    Some(xml[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torznab_feed_extracts_infohash_attr_and_base32_magnet() {
        let hex_hash = "0123456789abcdef0123456789abcdef01234567";
        let raw_bytes = hex::decode(hex_hash).unwrap();
        let b32_hash = data_encoding::BASE32_NOPAD.encode(&raw_bytes);

        let xml = format!(
            r#"<rss><channel>
                <item>
                    <title>Serie.S01E03.1080p.WEB-DL.mkv</title>
                    <link>magnet:?xt=urn:btih:{b32_hash}&amp;dn=Serie</link>
                    <size>1587832376</size>
                    <torznab:attr name="seeders" value="14"/>
                </item>
                <item>
                    <title>Pelicula.2024.1080p.BluRay.mkv</title>
                    <link>http://127.0.0.1:9696/download/123</link>
                    <size>4200000000</size>
                    <torznab:attr name="infohash" value="{hex_hash}"/>
                    <torznab:attr name="seeders" value="8"/>
                </item>
            </channel></rss>"#
        );

        let parsed = parse_torznab_feed(&xml, "ProwlarrTest", false, "1080p");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].hash, hex_hash);
        assert_eq!(parsed[0].sources, 14);
        assert_eq!(parsed[1].hash, hex_hash);
        assert_eq!(parsed[1].sources, 8);
    }

    #[test]
    fn evaluates_public_bt_candidate_as_torrent_scored_release() {
        let cand = crate::services::hybrid::public_search::PublicBtCandidate {
            info_hash: "0123456789abcdef0123456789abcdef01234567".to_string(),
            name: "Interstellar.2014.1080p.Castellano.BluRay.x264".to_string(),
            size_bytes: 2_400_000_000,
            seeders: 125,
            source: "apibay",
        };
        let magnet = format!("magnet:?xt=urn:btih:{}", cand.info_hash);
        let rel = evaluate_release(
            &cand.name,
            "torrent",
            &magnet,
            &cand.info_hash,
            cand.size_bytes,
            cand.seeders,
            &format!("BT/{}", cand.source),
            true,
            "1080p",
            "es",
        );
        assert_eq!(rel.protocol, "torrent");
        assert_eq!(rel.indexer_name, "BT/apibay");
        assert_eq!(rel.sources, 125);
        assert!(!rel.is_rejected);
    }
}
