// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Descubrimiento activo dirigido de puentes híbridos BitTorrent ⇄ eD2K.
//!
//! A diferencia del muestreo pasivo aleatorio `BEP 51`, este módulo toma el nombre y el tamaño
//! exacto de una descarga eD2K activa para:
//! 1. Extraer consultas limpias (título, año o episodio `SxxExx` / `NxNN`).
//! 2. Buscar en el catálogo DHT local por tokens e inspeccionar también archivos internos en `files_json`.
//! 3. Consultar los indexadores Torznab/Prowlarr habilitados y resolver metadatos `ut_metadata`
//!    cuando un torrent pueda ser un pack multifichero que contenga el archivo exacto al byte.

use std::collections::HashSet;

use mule_core::{HybridBridgeRecord, HybridBridgeStatus};
use mule_torrent::{DhtHarvester, DhtVideoFile};
use serde::{Deserialize, Serialize};

use crate::services::downloads::now_ms;
use crate::services::IndexerAggregator;
use crate::state::AppState;

use super::public_search::search_public_bt_sources;
use super::service::HybridSwarmService;

/// Número máximo de coincidencias por tokens inspeccionadas en el catálogo DHT local.
const ACTIVE_DISCOVERY_MAX_DHT_TOKEN_HITS: usize = 30;
/// Número máximo de torrents candidatos cuyo diccionario `info` se resuelve vía `ut_metadata` por ronda.
const ACTIVE_DISCOVERY_MAX_TORZNAB_RESOLVES: usize = 6;
/// Máximo de consultas enviadas al metabuscador público por descarga y ronda para no saturar APIs abiertas.
const MAX_PUBLIC_QUERIES_PER_DOWNLOAD: usize = 2;
/// Longitud mínima de un token de búsqueda para construir consultas limpias.
const MIN_QUERY_TOKEN_LEN: usize = 2;
/// Número máximo de palabras del título conservadas en las consultas hacia Torznab/DHT.
const MAX_TITLE_QUERY_TOKENS: usize = 6;
/// Perfil de calidad neutro usado al consultar `IndexerAggregator::search_torznab_only`.
const DEFAULT_HYBRID_QUALITY_PROFILE: &str = "1080p";
/// Año mínimo válido considerado pista de estreno audiovisual.
const MIN_RELEASE_YEAR: u32 = 1920;
/// Año máximo válido considerado pista de estreno audiovisual.
const MAX_RELEASE_YEAR: u32 = 2035;
/// Longitud exacta de un `info_hash` BitTorrent v1 hexadecimal.
const BTIH_HEX_LEN: usize = 40;

/// Marcadores técnicos y de grupo donde finaliza normalmente el título útil de un archivo multimedia.
const TECHNICAL_STOP_TOKENS: &[&str] = &[
    "2160p", "1080p", "720p", "m720p", "480p", "4k", "uhd", "hdrip", "bluray", "bdrip", "brrip",
    "remux", "web", "webrip", "webdl", "hdtv", "dvdrip", "x264", "x265", "h264", "h265", "hevc",
    "av1", "xvid", "aac", "ac3", "dts", "eac3", "ddp5", "dd5", "atmos", "hdr", "hdr10", "dv",
    "dual", "multi", "spanish", "castellano", "latino", "espanol", "español", "vose", "sub",
    "subs", "subtitulado", "amzn", "nf", "netflix", "dsnp", "atvp", "hmax", "proper", "repack",
    "by",
];

/// Mejor torrent equivalente encontrado (mismo título/episodio) cuando no existe copia bit a bit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquivalentTorrentHint {
    pub name: String,
    pub info_hash: String,
    pub size_bytes: u64,
    pub seeders: u64,
    pub source: String,
    pub magnet: String,
}

/// Resultado observable de una ronda de descubrimiento activo para una descarga eD2K.
#[derive(Debug, Clone, Default)]
pub struct ActiveDiscoveryOutcome {
    pub enabled_indexers: usize,
    pub torznab_candidates_checked: usize,
    pub public_candidates_checked: usize,
    pub dht_token_candidates_checked: usize,
    pub exact_matches_found: usize,
    pub bridges_created: usize,
    pub best_equivalent: Option<EquivalentTorrentHint>,
}

/// Construye consultas limpias de búsqueda y determina si el lanzamiento parece película (`true`) o serie (`false`).
pub fn build_hybrid_search_queries(file_name: &str) -> (Vec<String>, bool) {
    let stem = strip_video_extension(file_name.trim());
    let cleaned = strip_bracketed_groups(stem);
    let tokens = tokenize_alnum(&cleaned);

    let mut title_tokens: Vec<String> = Vec::new();
    let mut episode_pair: Option<(String, String)> = None;
    let mut release_year: Option<String> = None;

    for tok in &tokens {
        let lower = tok.to_ascii_lowercase();
        if episode_pair.is_none() {
            if let Some((s_norm, x_norm)) = parse_episode_pair(&lower) {
                episode_pair = Some((s_norm, x_norm));
                break;
            }
        }
        if is_release_year(&lower) && !title_tokens.is_empty() {
            release_year = Some(lower);
            break;
        }
        if TECHNICAL_STOP_TOKENS.contains(&lower.as_str()) {
            break;
        }
        if title_tokens.len() < MAX_TITLE_QUERY_TOKENS
            && (lower.len() >= MIN_QUERY_TOKEN_LEN || lower.bytes().all(|b| b.is_ascii_digit()))
        {
            title_tokens.push(tok.clone());
        }
    }

    if title_tokens.is_empty() {
        for tok in tokens.into_iter().take(MAX_TITLE_QUERY_TOKENS) {
            if tok.len() >= MIN_QUERY_TOKEN_LEN {
                title_tokens.push(tok);
            }
        }
    }

    let base_title = title_tokens.join(" ");
    let mut queries = Vec::new();
    let mut seen = HashSet::new();
    let mut push_unique = |q: String| {
        let trimmed = q.trim().to_string();
        if !trimmed.is_empty() && seen.insert(trimmed.to_ascii_lowercase()) {
            queries.push(trimmed);
        }
    };

    if let Some((sxxexx, nxnn)) = episode_pair {
        if !base_title.is_empty() {
            push_unique(format!("{base_title} {sxxexx}"));
            push_unique(format!("{base_title} {nxnn}"));
            push_unique(base_title);
        }
        (queries, false)
    } else {
        if !base_title.is_empty() {
            if let Some(year) = release_year {
                push_unique(format!("{base_title} {year}"));
            }
            push_unique(base_title);
        }
        (queries, true)
    }
}

/// Ejecuta descubrimiento activo dirigido sobre catálogo DHT local (por tokens) e indexadores
/// Torznab/Prowlarr configurados, registrando puentes candidatos exactos al byte.
pub async fn discover_bridges_actively(
    state: &AppState,
    harvester: Option<&DhtHarvester>,
    ed2k_hash: &str,
    file_name: &str,
    file_size: u64,
) -> ActiveDiscoveryOutcome {
    let mut outcome = ActiveDiscoveryOutcome::default();
    if file_size == 0 || file_name.trim().is_empty() {
        return outcome;
    }

    let (queries, is_movie) = build_hybrid_search_queries(file_name);
    if queries.is_empty() {
        return outcome;
    }

    let now = now_ms();
    let mut seen_dht_hashes: HashSet<String> = HashSet::new();

    // A. Búsqueda por tokens en el catálogo DHT local (incluye archivos internos en `files_json`).
    {
        let mut dl_guard = state.downloads.lock().await;
        for q in &queries {
            let Ok(hits) = dl_guard
                .db()
                .search_dht_catalog_by_tokens(q, ACTIVE_DISCOVERY_MAX_DHT_TOKEN_HITS)
            else {
                continue;
            };
            for hit in hits {
                if !seen_dht_hashes.insert(hit.info_hash.clone()) {
                    continue;
                }
                outcome.dht_token_candidates_checked += 1;

                let matched_file = if hit.video_file_size == file_size {
                    Some((hit.video_file_index, hit.video_file_offset))
                } else {
                    serde_json::from_str::<Vec<DhtVideoFile>>(&hit.files_json)
                        .ok()
                        .and_then(|files| {
                            files
                                .into_iter()
                                .find(|f| f.size_bytes == file_size)
                                .map(|f| (f.file_index, f.offset_in_torrent))
                        })
                };

                if let Some((bt_file_index, bt_file_offset)) = matched_file {
                    outcome.exact_matches_found += 1;
                    let rec = HybridBridgeRecord {
                        ed2k_hash: ed2k_hash.to_string(),
                        bt_info_hash: hit.info_hash,
                        file_size,
                        bt_file_index,
                        bt_file_offset,
                        status: HybridBridgeStatus::Candidate,
                        origin: "dht_token_exact".to_string(),
                        verified_chunks: 0,
                        published_kad_at_ms: None,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    if dl_guard.db_mut().upsert_hybrid_bridge(&rec).is_ok() {
                        outcome.bridges_created += 1;
                    }
                }
            }
        }
    }

    // B. Búsqueda dirigida en indexadores Torznab/Prowlarr habilitados.
    let aggregator = IndexerAggregator::new();
    let mut seen_torznab_hashes: HashSet<String> = HashSet::new();
    let mut resolves_used = 0usize;

    for q in &queries {
        let (releases, enabled_count) = aggregator
            .search_torznab_only(state, q, is_movie, DEFAULT_HYBRID_QUALITY_PROFILE)
            .await;
        outcome.enabled_indexers = outcome.enabled_indexers.max(enabled_count);
        if enabled_count == 0 {
            break;
        }

        for rel in releases {
            let btih = rel.hash.trim().to_ascii_lowercase();
            if btih.len() != BTIH_HEX_LEN
                || !btih.bytes().all(|b| b.is_ascii_hexdigit())
                || !seen_torznab_hashes.insert(btih.clone())
            {
                continue;
            }
            outcome.torznab_candidates_checked += 1;
            if rel.sources > 0 {
                let candidate_hint = EquivalentTorrentHint {
                    name: rel.title.clone(),
                    info_hash: btih.clone(),
                    size_bytes: rel.size_bytes,
                    seeders: rel.sources,
                    source: rel.indexer_name.clone(),
                    magnet: build_magnet_uri(&btih, &rel.title),
                };
                update_best_equivalent(&mut outcome.best_equivalent, candidate_hint, file_size);
            }

            if rel.size_bytes == file_size {
                outcome.exact_matches_found += 1;
                let rec = HybridBridgeRecord {
                    ed2k_hash: ed2k_hash.to_string(),
                    bt_info_hash: btih,
                    file_size,
                    bt_file_index: 0,
                    bt_file_offset: 0,
                    status: HybridBridgeStatus::Candidate,
                    origin: "torznab_exact_size".to_string(),
                    verified_chunks: 0,
                    published_kad_at_ms: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                let mut dl_guard = state.downloads.lock().await;
                if dl_guard.db_mut().upsert_hybrid_bridge(&rec).is_ok() {
                    outcome.bridges_created += 1;
                }
                continue;
            }

            // Posible pack multifichero o torrent con archivos acompañantes: resolver `ut_metadata`.
            let may_contain_file = rel.size_bytes == 0 || rel.size_bytes > file_size;
            if may_contain_file && resolves_used < ACTIVE_DISCOVERY_MAX_TORZNAB_RESOLVES {
                if let (Some(h), Some(ih_bytes)) = (harvester, parse_btih_bytes(&btih)) {
                    resolves_used += 1;
                    if let Some(discovered) = h
                        .resolve_infohash(ih_bytes, Vec::new(), "torznab_active")
                        .await
                    {
                        HybridSwarmService::persist_discovered_torrent(state, &discovered).await;
                        if let Some(vf) = discovered
                            .video_files
                            .iter()
                            .find(|f| f.size_bytes == file_size)
                        {
                            outcome.exact_matches_found += 1;
                            let rec = HybridBridgeRecord {
                                ed2k_hash: ed2k_hash.to_string(),
                                bt_info_hash: discovered.info_hash.clone(),
                                file_size,
                                bt_file_index: vf.file_index,
                                bt_file_offset: vf.offset_in_torrent,
                                status: HybridBridgeStatus::Candidate,
                                origin: "torznab_pack_exact".to_string(),
                                verified_chunks: 0,
                                published_kad_at_ms: None,
                                created_at_ms: now,
                                updated_at_ms: now,
                            };
                            let mut dl_guard = state.downloads.lock().await;
                            if dl_guard.db_mut().upsert_hybrid_bridge(&rec).is_ok() {
                                outcome.bridges_created += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // C. Metabuscador público de `info_hash` sin necesidad de Prowlarr/Torznab configurado.
    for q in queries.iter().take(MAX_PUBLIC_QUERIES_PER_DOWNLOAD) {
        let public_hits = search_public_bt_sources(state, q, is_movie).await;
        for cand in public_hits {
            if !seen_torznab_hashes.insert(cand.info_hash.clone()) {
                continue;
            }
            outcome.public_candidates_checked += 1;
            if cand.seeders > 0 {
                let candidate_hint = EquivalentTorrentHint {
                    name: cand.name.clone(),
                    info_hash: cand.info_hash.clone(),
                    size_bytes: cand.size_bytes,
                    seeders: cand.seeders,
                    source: cand.source.to_string(),
                    magnet: build_magnet_uri(&cand.info_hash, &cand.name),
                };
                update_best_equivalent(&mut outcome.best_equivalent, candidate_hint, file_size);
            }

            if cand.size_bytes == file_size {
                outcome.exact_matches_found += 1;
                let rec = HybridBridgeRecord {
                    ed2k_hash: ed2k_hash.to_string(),
                    bt_info_hash: cand.info_hash,
                    file_size,
                    bt_file_index: 0,
                    bt_file_offset: 0,
                    status: HybridBridgeStatus::Candidate,
                    origin: "public_bt_exact".to_string(),
                    verified_chunks: 0,
                    published_kad_at_ms: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                let mut dl_guard = state.downloads.lock().await;
                if dl_guard.db_mut().upsert_hybrid_bridge(&rec).is_ok() {
                    outcome.bridges_created += 1;
                }
                continue;
            }

            let may_contain_file = cand.size_bytes == 0 || cand.size_bytes > file_size;
            if may_contain_file && resolves_used < ACTIVE_DISCOVERY_MAX_TORZNAB_RESOLVES {
                if let (Some(h), Some(ih_bytes)) = (harvester, parse_btih_bytes(&cand.info_hash)) {
                    resolves_used += 1;
                    if let Some(discovered) = h
                        .resolve_infohash(ih_bytes, Vec::new(), "public_bt")
                        .await
                    {
                        HybridSwarmService::persist_discovered_torrent(state, &discovered).await;
                        if let Some(vf) = discovered
                            .video_files
                            .iter()
                            .find(|f| f.size_bytes == file_size)
                        {
                            outcome.exact_matches_found += 1;
                            let rec = HybridBridgeRecord {
                                ed2k_hash: ed2k_hash.to_string(),
                                bt_info_hash: discovered.info_hash.clone(),
                                file_size,
                                bt_file_index: vf.file_index,
                                bt_file_offset: vf.offset_in_torrent,
                                status: HybridBridgeStatus::Candidate,
                                origin: "public_bt_pack".to_string(),
                                verified_chunks: 0,
                                published_kad_at_ms: None,
                                created_at_ms: now,
                                updated_at_ms: now,
                            };
                            let mut dl_guard = state.downloads.lock().await;
                            if dl_guard.db_mut().upsert_hybrid_bridge(&rec).is_ok() {
                                outcome.bridges_created += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    outcome
}

fn build_magnet_uri(info_hash: &str, name: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{info_hash}&dn={dn}")
}

fn update_best_equivalent(
    best: &mut Option<EquivalentTorrentHint>,
    candidate: EquivalentTorrentHint,
    target_size: u64,
) {
    let replace = match best {
        None => true,
        Some(cur) => {
            candidate.seeders > cur.seeders
                || (candidate.seeders == cur.seeders
                    && candidate.size_bytes.abs_diff(target_size)
                        < cur.size_bytes.abs_diff(target_size))
        }
    };
    if replace {
        *best = Some(candidate);
    }
}

fn parse_btih_bytes(hex_str: &str) -> Option<[u8; 20]> {
    let raw = hex::decode(hex_str.trim()).ok()?;
    if raw.len() != 20 {
        return None;
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&raw);
    Some(out)
}

fn strip_video_extension(name: &str) -> &str {
    const EXTENSIONS: &[&str] = &[".mkv", ".mp4", ".avi", ".mov", ".m4v", ".ts", ".wmv", ".webm"];
    let lower = name.to_ascii_lowercase();
    for ext in EXTENSIONS {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return &name[..name.len() - ext.len()];
        }
    }
    name
}

fn strip_bracketed_groups(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut round_depth = 0usize;
    let mut square_depth = 0usize;
    for ch in input.chars() {
        match ch {
            '(' => round_depth += 1,
            ')' => round_depth = round_depth.saturating_sub(1),
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            _ if round_depth == 0 && square_depth == 0 => out.push(ch),
            _ => out.push(' '),
        }
    }
    out
}

fn tokenize_alnum(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_episode_pair(tok: &str) -> Option<(String, String)> {
    // S01E03 / s1e3
    if let Some(rest) = tok.strip_prefix('s') {
        let s_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=2).contains(&s_digits) {
            let after_s = &rest[s_digits..];
            if let Some(after_e) = after_s.strip_prefix('e') {
                let e_digits = after_e.bytes().take_while(u8::is_ascii_digit).count();
                if (1..=4).contains(&e_digits) && e_digits == after_e.len() {
                    let s_num: u32 = rest[..s_digits].parse().ok()?;
                    let e_num: u32 = after_e.parse().ok()?;
                    return Some((format!("S{s_num:02}E{e_num:02}"), format!("{s_num}x{e_num:02}")));
                }
            }
        }
    }
    // 1x03 / 01x03
    if let Some(x_pos) = tok.find('x') {
        let left = &tok[..x_pos];
        let right = &tok[x_pos + 1..];
        if (1..=2).contains(&left.len())
            && (2..=4).contains(&right.len())
            && left.bytes().all(|b| b.is_ascii_digit())
            && right.bytes().all(|b| b.is_ascii_digit())
        {
            let s_num: u32 = left.parse().ok()?;
            let e_num: u32 = right.parse().ok()?;
            return Some((format!("S{s_num:02}E{e_num:02}"), format!("{s_num}x{e_num:02}")));
        }
    }
    None
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
    fn builds_clean_queries_for_spanish_series_and_movies() {
        let (series_q, is_movie_series) = build_hybrid_search_queries(
            "El.señor.de.los.anillos.Los.anillos.de.poder.1x03.Adar.m720p.AMZN.WEB-DL.DDP5.1.H.264-eth@n.mkv",
        );
        assert!(!is_movie_series);
        assert_eq!(
            series_q,
            vec![
                "El señor de los anillos Los S01E03".to_string(),
                "El señor de los anillos Los 1x03".to_string(),
                "El señor de los anillos Los".to_string(),
            ]
        );

        let (movie_q, is_movie) = build_hybrid_search_queries(
            "Pelicula.Ejemplo.2024.1080p.x265-EAC3.by.LWTL.(hispashare.org.&.tevelliure).mkv",
        );
        assert!(is_movie);
        assert_eq!(
            movie_q,
            vec![
                "Pelicula Ejemplo 2024".to_string(),
                "Pelicula Ejemplo".to_string(),
            ]
        );
    }
}
