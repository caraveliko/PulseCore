// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Manejadores JSON-RPC para el motor híbrido eD2K ⇄ Kad ⇄ BitTorrent sin indexadores.

use mule_core::persistence::sqlite::hybrid_index::{
    DEFAULT_DHT_SEARCH_LIMIT, MAX_DHT_SEARCH_LIMIT,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::JsonRpcError;
use crate::rpc::handlers::json_value;
use crate::state::AppState;

/// `hybrid.status` — telemetría del Sabueso DHT, catálogo local, puentes verificados y fusiones activas.
pub async fn handle_hybrid_status(state: &AppState) -> Result<Value, JsonRpcError> {
    let snap = state.hybrid.status_snapshot(state).await;
    Ok(json_value(&snap))
}

/// `hybrid.searchDht` — busca torrents de vídeo descubiertos en Mainline DHT sin indexadores externos.
///
/// Parámetros admitidos: `query` (texto), `exactSize` (tamaño exacto en bytes) y `limit`.
pub async fn handle_hybrid_search_dht(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let p = params.unwrap_or_else(|| json!({}));
    let query = p["query"].as_str().unwrap_or("").trim();
    let exact_size = p["exactSize"].as_u64().filter(|&v| v > 0);
    let limit = p["limit"]
        .as_u64()
        .map(|v| (v as usize).clamp(1, MAX_DHT_SEARCH_LIMIT))
        .unwrap_or(DEFAULT_DHT_SEARCH_LIMIT);

    let entries = {
        let dl = state.downloads.lock().await;
        if let Some(sz) = exact_size {
            dl.db()
                .find_dht_catalog_by_exact_video_size(sz, limit)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
        } else if !query.is_empty() {
            dl.db()
                .search_dht_catalog_by_tokens(query, limit)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
        } else {
            return Err(JsonRpcError::invalid_params(
                "Se requiere 'query' o 'exactSize' para buscar en el catálogo DHT local",
            ));
        }
    };

    let items: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            let magnet = format!(
                "magnet:?xt=urn:btih:{}&dn={}",
                entry.info_hash,
                url::form_urlencoded::byte_serialize(entry.name.as_bytes()).collect::<String>()
            );
            json!({
                "infoHash": entry.info_hash,
                "name": entry.name,
                "magnet": magnet,
                "totalBytes": entry.total_bytes,
                "videoFileName": entry.video_file_name,
                "videoFileSize": entry.video_file_size,
                "videoFileIndex": entry.video_file_index,
                "videoFileOffset": entry.video_file_offset,
                "estimatedPeers": entry.estimated_peers,
                "source": entry.source,
                "firstSeenMs": entry.first_seen_ms,
                "lastSeenMs": entry.last_seen_ms,
            })
        })
        .collect();

    Ok(json!({
        "count": items.len(),
        "items": items,
    }))
}

/// `hybrid.listBridges` — lista los puentes híbridos `ed2k_hash ↔ bt_info_hash` registrados en BD.
pub async fn handle_hybrid_list_bridges(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let p = params.unwrap_or_else(|| json!({}));
    let ed2k_hash = p["ed2kHash"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = p["limit"]
        .as_u64()
        .map(|v| (v as usize).clamp(1, MAX_DHT_SEARCH_LIMIT))
        .unwrap_or(DEFAULT_DHT_SEARCH_LIMIT);

    let bridges = {
        let dl = state.downloads.lock().await;
        if let Some(h) = ed2k_hash {
            dl.db()
                .list_hybrid_bridges_for_ed2k(h)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
        } else {
            dl.db()
                .list_all_hybrid_bridges(limit)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
        }
    };

    Ok(json!({
        "count": bridges.len(),
        "bridges": bridges,
    }))
}

/// `hybrid.discoverForDownload` — dispara descubrimiento híbrido automático (DHT local + Kad Notes + Rendezvous).
pub async fn handle_hybrid_discover_for_download(
    state: &AppState,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let p = params.ok_or_else(|| JsonRpcError::invalid_params("Faltan parámetros"))?;
    let dl_id_str = p["downloadId"]
        .as_str()
        .ok_or_else(|| JsonRpcError::invalid_params("Falta 'downloadId'"))?;
    let download_id = Uuid::parse_str(dl_id_str)
        .map_err(|_| JsonRpcError::invalid_params("UUID 'downloadId' inválido"))?;

    let report = state
        .hybrid
        .discover_for_download(state, download_id)
        .await
        .map_err(|e| JsonRpcError::invalid_params(&e))?;

    Ok(json_value(&report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::services::downloads::{now_ms, DownloadService};
    use crate::services::IndexerAggregator;
    use mule_core::{AllocationPolicy, DhtTorrentCatalogEntry};

    #[tokio::test]
    async fn rpc_hybrid_zero_indexer_search_and_exact_size_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let dl_dir = dir.path().join("downloads");
        let temp_dir = data_dir.join("temp");
        std::fs::create_dir_all(&dl_dir).unwrap();

        let mut cfg = DaemonConfig::new(data_dir.clone(), "test-token");
        cfg.default_download_dir = dl_dir.clone();
        let dl_svc = DownloadService::in_memory(temp_dir).unwrap();
        let state = AppState::new(cfg, dl_svc);

        // 1. Sembrar el catálogo DHT local (simulando descubrimiento autónomo BEP 51 + BEP 9).
        let exact_size: u64 = 1_450_123_456;
        let bt_hash = "11223344556677889900aabbccddeeff11223344";
        let now = now_ms();
        {
            let mut dl = state.downloads.lock().await;
            dl.db_mut()
                .upsert_dht_catalog_entry(&DhtTorrentCatalogEntry {
                    info_hash: bt_hash.to_string(),
                    name: "Interstellar.2014.1080p.BluRay.x264".to_string(),
                    total_bytes: exact_size + 2048,
                    video_file_name: "Interstellar.2014.1080p.BluRay.mkv".to_string(),
                    video_file_size: exact_size,
                    video_file_index: 1,
                    video_file_offset: 2048,
                    files_json: "[]".to_string(),
                    estimated_peers: 37,
                    source: "dht_bep51".to_string(),
                    first_seen_ms: now,
                    last_seen_ms: now,
                })
                .unwrap();
        }

        // 2. Búsqueda sin indexadores por texto vía `hybrid.searchDht`.
        let by_text = handle_hybrid_search_dht(
            &state,
            Some(json!({ "query": "interstellar 1080p", "limit": 10 })),
        )
        .await
        .unwrap();
        assert_eq!(by_text["count"], 1);
        assert_eq!(by_text["items"][0]["infoHash"], bt_hash);
        assert!(by_text["items"][0]["magnet"]
            .as_str()
            .unwrap()
            .starts_with(&format!("magnet:?xt=urn:btih:{bt_hash}")));

        // 3. Búsqueda sin indexadores integrada en `IndexerAggregator::search_all` (0 indexadores Torznab).
        let agg = IndexerAggregator::new();
        let releases = agg
            .search_all(&state, "Interstellar 2014", true, "1080p")
            .await;
        assert!(
            releases.iter().any(|r| {
                r.protocol == "torrent" && r.indexer_name == "BT/DHT-Local" && r.hash == bt_hash
            }),
            "IndexerAggregator debía encontrar el torrent desde BT/DHT-Local sin indexadores externos"
        );

        // 4. Descarga eD2K con el mismo tamaño exacto al byte: `hybrid.discoverForDownload` crea el puente candidato.
        let ed2k_hex = "abcdef0123456789abcdef0123456789";
        let ed2k_link = format!("ed2k://|file|Interstellar.2014.mkv|{exact_size}|{ed2k_hex}|/");
        let snap = {
            let mut dl = state.downloads.lock().await;
            dl.add_ed2k_link(&ed2k_link, &dl_dir, AllocationPolicy::None, None)
                .unwrap()
        };

        let discovery = handle_hybrid_discover_for_download(
            &state,
            Some(json!({ "downloadId": snap.id.to_string() })),
        )
        .await
        .unwrap();
        assert_eq!(discovery["dhtExactMatches"], 1);
        assert_eq!(discovery["bridges"].as_array().unwrap().len(), 1);
        assert_eq!(discovery["bridges"][0]["btInfoHash"], bt_hash);
        assert_eq!(discovery["bridges"][0]["origin"], "dht_exact_size");

        // 5. `hybrid.status` y `hybrid.listBridges` reflejan el estado real.
        let status = handle_hybrid_status(&state).await.unwrap();
        assert_eq!(status["catalog"]["catalogEntries"], 1);
        assert_eq!(status["catalog"]["bridgesCandidate"], 1);

        let bridges = handle_hybrid_list_bridges(&state, Some(json!({ "ed2kHash": ed2k_hex })))
            .await
            .unwrap();
        assert_eq!(bridges["count"], 1);
    }
}
