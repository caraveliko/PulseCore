// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Persistencia SQLite del catálogo DHT local (sin indexadores) y los puentes híbridos eD2K ⇄ BitTorrent.

use rusqlite::{params, Connection};

use crate::error::CoreError;
use crate::model::{
    DhtTorrentCatalogEntry, DhtTorrentFileEntry, HybridBridgeRecord, HybridBridgeStatus,
    HybridCatalogStats,
};

/// Límite máximo de entradas conservadas en el catálogo DHT local (evicción LRU por `last_seen_ms`).
pub const MAX_DHT_CATALOG_ENTRIES: usize = 200_000;
/// Límite máximo de archivos de vídeo indexados por cada pack multifichero.
pub const MAX_DHT_FILES_PER_TORRENT: usize = 500;
/// Límite predeterminado de resultados devueltos en búsquedas del catálogo DHT local.
pub const DEFAULT_DHT_SEARCH_LIMIT: usize = 50;
/// Límite máximo de resultados permitidos en una sola consulta del catálogo DHT.
pub const MAX_DHT_SEARCH_LIMIT: usize = 200;
/// Longitud mínima de un token de búsqueda para evitar consultas degeneradas.
const MIN_SEARCH_TOKEN_LEN: usize = 2;
/// Número máximo de tokens aplicados en una consulta SQL sobre el catálogo DHT.
const MAX_SEARCH_TOKENS: usize = 8;

/// Normaliza un texto de nombre de archivo o torrent a tokens en minúsculas separados por espacios
/// para búsquedas sin indexadores tolerantes a puntos, guiones y corchetes.
pub fn normalize_search_text(name: &str, video_file_name: &str) -> String {
    let mut out = String::with_capacity(name.len() + video_file_name.len() + 1);
    let mut push_clean = |input: &str| {
        let mut prev_space = out.is_empty() || out.ends_with(' ');
        for ch in input.chars() {
            if ch.is_alphanumeric() {
                for lower in ch.to_lowercase() {
                    out.push(lower);
                }
                prev_space = false;
            } else if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        }
    };
    push_clean(name);
    if !video_file_name.is_empty() && video_file_name != name {
        push_clean(video_file_name);
    }
    out.trim().to_string()
}

/// Inserta o actualiza un torrent de vídeo descubierto en el catálogo DHT local.
pub fn upsert_dht_catalog_entry(
    conn: &mut Connection,
    entry: &DhtTorrentCatalogEntry,
) -> Result<(), CoreError> {
    let info_hash = entry.info_hash.trim().to_ascii_lowercase();
    let search_text = normalize_search_text(&entry.name, &entry.video_file_name);
    conn.execute(
        "INSERT INTO dht_torrent_catalog (
            info_hash, name, search_text, total_bytes, video_file_name, video_file_size,
            video_file_index, video_file_offset, files_json, estimated_peers, source,
            first_seen_ms, last_seen_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(info_hash) DO UPDATE SET
            name = excluded.name,
            search_text = excluded.search_text,
            total_bytes = excluded.total_bytes,
            video_file_name = excluded.video_file_name,
            video_file_size = excluded.video_file_size,
            video_file_index = excluded.video_file_index,
            video_file_offset = excluded.video_file_offset,
            files_json = excluded.files_json,
            estimated_peers = MAX(dht_torrent_catalog.estimated_peers, excluded.estimated_peers),
            last_seen_ms = MAX(dht_torrent_catalog.last_seen_ms, excluded.last_seen_ms)",
        params![
            info_hash,
            entry.name,
            search_text,
            entry.total_bytes as i64,
            entry.video_file_name,
            entry.video_file_size as i64,
            entry.video_file_index as i64,
            entry.video_file_offset as i64,
            entry.files_json,
            i64::from(entry.estimated_peers),
            entry.source,
            entry.first_seen_ms,
            entry.last_seen_ms,
        ],
    )?;
    Ok(())
}

/// Almacena los metadatos `.torrent` comprimidos (`zstd`) y la lista de pares conocidos para arranque instantáneo.
pub fn store_dht_torrent_meta(
    conn: &mut Connection,
    info_hash: &str,
    torrent_meta_zstd: &[u8],
    known_peers_json: &str,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE dht_torrent_catalog
         SET torrent_meta_zstd = ?2,
             known_peers_json = ?3
         WHERE info_hash = ?1",
        params![
            info_hash.trim().to_ascii_lowercase(),
            torrent_meta_zstd,
            known_peers_json,
        ],
    )?;
    Ok(())
}

/// Par `(metadatos_torrent_zstd_opcionales, known_peers_json)` persistido en `dht_torrent_catalog`.
pub type CachedDhtTorrentMeta = (Option<Vec<u8>>, String);

/// Recupera los metadatos `.torrent` comprimidos (`zstd`) y la lista JSON de pares conocidos, si existen.
pub fn load_dht_torrent_meta(
    conn: &Connection,
    info_hash: &str,
) -> Result<Option<CachedDhtTorrentMeta>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT torrent_meta_zstd, known_peers_json
         FROM dht_torrent_catalog
         WHERE info_hash = ?1",
    )?;
    let mut rows = stmt.query(params![info_hash.trim().to_ascii_lowercase()])?;
    if let Some(row) = rows.next()? {
        let meta: Option<Vec<u8>> = row.get(0)?;
        let peers_json: String = row.get(1)?;
        Ok(Some((meta, peers_json)))
    } else {
        Ok(None)
    }
}

/// Indexa todos los archivos de vídeo contenidos en un torrent multifichero (packs de series/anime)
/// para permitir cruces por tamaño exacto y búsquedas por episodio individual.
pub fn upsert_dht_catalog_files(
    conn: &mut Connection,
    info_hash: &str,
    torrent_name: &str,
    files: &[DhtTorrentFileEntry],
) -> Result<(), CoreError> {
    if files.is_empty() {
        return Ok(());
    }
    let clean_hash = info_hash.trim().to_ascii_lowercase();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO dht_torrent_files (
                info_hash, file_index, file_path, search_text, file_size, file_offset
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(info_hash, file_index) DO UPDATE SET
                file_path = excluded.file_path,
                search_text = excluded.search_text,
                file_size = excluded.file_size,
                file_offset = excluded.file_offset",
        )?;
        for file in files.iter().take(MAX_DHT_FILES_PER_TORRENT) {
            let search_text = normalize_search_text(torrent_name, &file.file_path);
            stmt.execute(params![
                clean_hash,
                file.file_index as i64,
                file.file_path,
                search_text,
                file.file_size as i64,
                file.file_offset as i64,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Poda las entradas más antiguas del catálogo DHT cuando supera `max_entries` y limpia archivos huérfanos.
pub fn prune_dht_catalog(conn: &mut Connection, max_entries: usize) -> Result<usize, CoreError> {
    let limit = max_entries.max(1) as i64;
    let deleted = conn.execute(
        "DELETE FROM dht_torrent_catalog
         WHERE info_hash NOT IN (
             SELECT info_hash FROM dht_torrent_catalog
             ORDER BY last_seen_ms DESC
             LIMIT ?1
         )",
        params![limit],
    )?;
    if deleted > 0 {
        let _ = conn.execute(
            "DELETE FROM dht_torrent_files
             WHERE info_hash NOT IN (SELECT info_hash FROM dht_torrent_catalog)",
            [],
        );
    }
    Ok(deleted)
}

fn map_catalog_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DhtTorrentCatalogEntry> {
    Ok(DhtTorrentCatalogEntry {
        info_hash: row.get(0)?,
        name: row.get(1)?,
        total_bytes: row.get::<_, i64>(2)?.max(0) as u64,
        video_file_name: row.get(3)?,
        video_file_size: row.get::<_, i64>(4)?.max(0) as u64,
        video_file_index: row.get::<_, i64>(5)?.max(0) as usize,
        video_file_offset: row.get::<_, i64>(6)?.max(0) as u64,
        files_json: row.get(7)?,
        estimated_peers: row.get::<_, i64>(8)?.max(0) as u32,
        source: row.get(9)?,
        first_seen_ms: row.get(10)?,
        last_seen_ms: row.get(11)?,
    })
}

/// Busca torrents en el catálogo DHT local (incluyendo episodios individuales dentro de packs
/// multifichero en `dht_torrent_files`) que coincidan exactamente al byte con `video_file_size`.
pub fn find_dht_catalog_by_exact_video_size(
    conn: &Connection,
    video_file_size: u64,
    limit: usize,
) -> Result<Vec<DhtTorrentCatalogEntry>, CoreError> {
    if video_file_size == 0 {
        return Ok(Vec::new());
    }
    let lim = limit.clamp(1, MAX_DHT_SEARCH_LIMIT) as i64;
    let mut out = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();

    // 1. Buscar primero en la tabla multifichero `dht_torrent_files` (cubre todos los episodios de packs).
    {
        let mut stmt = conn.prepare(
            "SELECT c.info_hash, c.name, c.total_bytes, f.file_path, f.file_size,
                    f.file_index, f.file_offset, c.files_json, c.estimated_peers,
                    c.source, c.first_seen_ms, c.last_seen_ms
             FROM dht_torrent_files f
             JOIN dht_torrent_catalog c ON c.info_hash = f.info_hash
             WHERE f.file_size = ?1
             ORDER BY c.estimated_peers DESC, c.last_seen_ms DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![video_file_size as i64, lim], map_catalog_row)?;
        for row in rows {
            let entry = row?;
            if seen_keys.insert((entry.info_hash.clone(), entry.video_file_index)) {
                out.push(entry);
            }
        }
    }

    // 2. Complementar con `dht_torrent_catalog` para entradas monofichero o previas sin filas hijas.
    if out.len() < lim as usize {
        let remaining = lim - out.len() as i64;
        let mut stmt = conn.prepare(
            "SELECT info_hash, name, total_bytes, video_file_name, video_file_size,
                    video_file_index, video_file_offset, files_json, estimated_peers,
                    source, first_seen_ms, last_seen_ms
             FROM dht_torrent_catalog
             WHERE video_file_size = ?1
             ORDER BY estimated_peers DESC, last_seen_ms DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![video_file_size as i64, remaining], map_catalog_row)?;
        for row in rows {
            let entry = row?;
            if seen_keys.insert((entry.info_hash.clone(), entry.video_file_index)) {
                out.push(entry);
            }
        }
    }
    Ok(out)
}

/// Busca en el catálogo DHT local (torrents y episodios individuales en packs) por palabras clave
/// normalizadas sin indexadores HTTP.
pub fn search_dht_catalog_by_tokens(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<DhtTorrentCatalogEntry>, CoreError> {
    let normalized = normalize_search_text(query, "");
    let tokens: Vec<&str> = normalized
        .split_whitespace()
        .filter(|t| t.len() >= MIN_SEARCH_TOKEN_LEN)
        .take(MAX_SEARCH_TOKENS)
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let lim = limit.clamp(1, MAX_DHT_SEARCH_LIMIT) as i64;
    let mut out = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();

    // 1. Buscar episodios individuales dentro de packs (`dht_torrent_files`).
    {
        let mut sql = String::from(
            "SELECT c.info_hash, c.name, c.total_bytes, f.file_path, f.file_size,
                    f.file_index, f.file_offset, c.files_json, c.estimated_peers,
                    c.source, c.first_seen_ms, c.last_seen_ms
             FROM dht_torrent_files f
             JOIN dht_torrent_catalog c ON c.info_hash = f.info_hash
             WHERE 1=1",
        );
        let mut bind_values: Vec<rusqlite::types::Value> = Vec::with_capacity(tokens.len() + 1);
        for token in &tokens {
            sql.push_str(" AND f.search_text LIKE ?");
            bind_values.push(rusqlite::types::Value::Text(format!("%{token}%")));
        }
        sql.push_str(" ORDER BY c.estimated_peers DESC, c.last_seen_ms DESC LIMIT ?");
        bind_values.push(rusqlite::types::Value::Integer(lim));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(bind_values.iter()),
            map_catalog_row,
        )?;
        for row in rows {
            let entry = row?;
            if seen_keys.insert((entry.info_hash.clone(), entry.video_file_index)) {
                out.push(entry);
            }
        }
    }

    // 2. Complementar con el catálogo principal (`dht_torrent_catalog`) para entradas sin filas hijas.
    if out.len() < lim as usize {
        let remaining = lim - out.len() as i64;
        let mut sql = String::from(
            "SELECT info_hash, name, total_bytes, video_file_name, video_file_size,
                    video_file_index, video_file_offset, files_json, estimated_peers,
                    source, first_seen_ms, last_seen_ms
             FROM dht_torrent_catalog
             WHERE 1=1",
        );
        let mut bind_values: Vec<rusqlite::types::Value> = Vec::with_capacity(tokens.len() + 1);
        for token in &tokens {
            sql.push_str(" AND search_text LIKE ?");
            bind_values.push(rusqlite::types::Value::Text(format!("%{token}%")));
        }
        sql.push_str(" ORDER BY estimated_peers DESC, last_seen_ms DESC LIMIT ?");
        bind_values.push(rusqlite::types::Value::Integer(remaining));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(bind_values.iter()),
            map_catalog_row,
        )?;
        for row in rows {
            let entry = row?;
            if seen_keys.insert((entry.info_hash.clone(), entry.video_file_index)) {
                out.push(entry);
            }
        }
    }
    Ok(out)
}

/// Registra o actualiza un puente híbrido eD2K ⇄ BitTorrent respetando el invariante
/// de no degradar nunca un puente ya `verified` ni resucitar uno `rejected` a `candidate`.
pub fn upsert_hybrid_bridge(
    conn: &mut Connection,
    record: &HybridBridgeRecord,
) -> Result<(), CoreError> {
    let ed2k_hash = record.ed2k_hash.trim().to_ascii_lowercase();
    let bt_info_hash = record.bt_info_hash.trim().to_ascii_lowercase();
    let incoming_status = record.status.as_str();
    conn.execute(
        "INSERT INTO hybrid_bridges (
            ed2k_hash, bt_info_hash, file_size, bt_file_index, bt_file_offset,
            status, origin, verified_chunks, published_kad_at_ms, created_at_ms, updated_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(ed2k_hash, bt_info_hash) DO UPDATE SET
            file_size = CASE WHEN excluded.file_size > 0 THEN excluded.file_size ELSE hybrid_bridges.file_size END,
            bt_file_index = excluded.bt_file_index,
            bt_file_offset = excluded.bt_file_offset,
            status = CASE
                WHEN hybrid_bridges.status = 'rejected' AND excluded.status = 'candidate' THEN 'rejected'
                WHEN hybrid_bridges.status = 'verified' AND excluded.status = 'candidate' THEN 'verified'
                ELSE excluded.status
            END,
            verified_chunks = MAX(hybrid_bridges.verified_chunks, excluded.verified_chunks),
            published_kad_at_ms = COALESCE(excluded.published_kad_at_ms, hybrid_bridges.published_kad_at_ms),
            updated_at_ms = MAX(hybrid_bridges.updated_at_ms, excluded.updated_at_ms)",
        params![
            ed2k_hash,
            bt_info_hash,
            record.file_size as i64,
            record.bt_file_index as i64,
            record.bt_file_offset as i64,
            incoming_status,
            record.origin,
            i64::from(record.verified_chunks),
            record.published_kad_at_ms,
            record.created_at_ms,
            record.updated_at_ms,
        ],
    )?;
    Ok(())
}

/// Actualiza explícitamente el estado criptográfico y el contador de partes verificadas de un puente.
pub fn update_hybrid_bridge_status(
    conn: &mut Connection,
    ed2k_hash: &str,
    bt_info_hash: &str,
    status: HybridBridgeStatus,
    verified_chunks: u32,
    updated_at_ms: i64,
) -> Result<bool, CoreError> {
    let changed = conn.execute(
        "UPDATE hybrid_bridges
         SET status = ?3,
             verified_chunks = MAX(verified_chunks, ?4),
             updated_at_ms = ?5
         WHERE ed2k_hash = ?1 AND bt_info_hash = ?2",
        params![
            ed2k_hash.trim().to_ascii_lowercase(),
            bt_info_hash.trim().to_ascii_lowercase(),
            status.as_str(),
            i64::from(verified_chunks),
            updated_at_ms,
        ],
    )?;
    Ok(changed > 0)
}

/// Registra la marca temporal de la última publicación de un puente verificado en notas Kad.
pub fn mark_hybrid_bridge_kad_published(
    conn: &mut Connection,
    ed2k_hash: &str,
    bt_info_hash: &str,
    published_at_ms: i64,
) -> Result<bool, CoreError> {
    let changed = conn.execute(
        "UPDATE hybrid_bridges
         SET published_kad_at_ms = ?3,
             updated_at_ms = MAX(updated_at_ms, ?3)
         WHERE ed2k_hash = ?1 AND bt_info_hash = ?2",
        params![
            ed2k_hash.trim().to_ascii_lowercase(),
            bt_info_hash.trim().to_ascii_lowercase(),
            published_at_ms,
        ],
    )?;
    Ok(changed > 0)
}

fn map_bridge_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HybridBridgeRecord> {
    let status_str: String = row.get(5)?;
    Ok(HybridBridgeRecord {
        ed2k_hash: row.get(0)?,
        bt_info_hash: row.get(1)?,
        file_size: row.get::<_, i64>(2)?.max(0) as u64,
        bt_file_index: row.get::<_, i64>(3)?.max(0) as usize,
        bt_file_offset: row.get::<_, i64>(4)?.max(0) as u64,
        status: HybridBridgeStatus::parse_str(&status_str),
        origin: row.get(6)?,
        verified_chunks: row.get::<_, i64>(7)?.max(0) as u32,
        published_kad_at_ms: row.get(8)?,
        created_at_ms: row.get(9)?,
        updated_at_ms: row.get(10)?,
    })
}

/// Lista los puentes registrados para un `ed2k_hash`, ordenando los verificados primero.
pub fn list_hybrid_bridges_for_ed2k(
    conn: &Connection,
    ed2k_hash: &str,
) -> Result<Vec<HybridBridgeRecord>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT ed2k_hash, bt_info_hash, file_size, bt_file_index, bt_file_offset,
                status, origin, verified_chunks, published_kad_at_ms, created_at_ms, updated_at_ms
         FROM hybrid_bridges
         WHERE ed2k_hash = ?1
         ORDER BY
             CASE status WHEN 'verified' THEN 0 WHEN 'candidate' THEN 1 ELSE 2 END,
             verified_chunks DESC,
             updated_at_ms DESC",
    )?;
    let rows = stmt.query_map(params![ed2k_hash.trim().to_ascii_lowercase()], map_bridge_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Lista los puentes híbridos más recientes almacenados en la base de datos.
pub fn list_all_hybrid_bridges(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<HybridBridgeRecord>, CoreError> {
    let lim = limit.clamp(1, MAX_DHT_SEARCH_LIMIT) as i64;
    let mut stmt = conn.prepare(
        "SELECT ed2k_hash, bt_info_hash, file_size, bt_file_index, bt_file_offset,
                status, origin, verified_chunks, published_kad_at_ms, created_at_ms, updated_at_ms
         FROM hybrid_bridges
         ORDER BY
             CASE status WHEN 'verified' THEN 0 WHEN 'candidate' THEN 1 ELSE 2 END,
             updated_at_ms DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![lim], map_bridge_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Calcula los contadores globales del catálogo DHT local y de los puentes híbridos.
pub fn get_hybrid_catalog_stats(conn: &Connection) -> Result<HybridCatalogStats, CoreError> {
    let catalog_entries: i64 =
        conn.query_row("SELECT COUNT(*) FROM dht_torrent_catalog", [], |r| r.get(0))?;
    let bridges_total: i64 =
        conn.query_row("SELECT COUNT(*) FROM hybrid_bridges", [], |r| r.get(0))?;
    let bridges_verified: i64 = conn.query_row(
        "SELECT COUNT(*) FROM hybrid_bridges WHERE status = 'verified'",
        [],
        |r| r.get(0),
    )?;
    let bridges_candidate: i64 = conn.query_row(
        "SELECT COUNT(*) FROM hybrid_bridges WHERE status = 'candidate'",
        [],
        |r| r.get(0),
    )?;
    let bridges_rejected: i64 = conn.query_row(
        "SELECT COUNT(*) FROM hybrid_bridges WHERE status = 'rejected'",
        [],
        |r| r.get(0),
    )?;
    Ok(HybridCatalogStats {
        catalog_entries: catalog_entries.max(0) as usize,
        bridges_total: bridges_total.max(0) as usize,
        bridges_verified: bridges_verified.max(0) as usize,
        bridges_candidate: bridges_candidate.max(0) as usize,
        bridges_rejected: bridges_rejected.max(0) as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::Database;

    #[test]
    fn dht_catalog_exact_size_token_search_and_lru_prune() {
        let mut db = Database::open_in_memory().unwrap();
        let e1 = DhtTorrentCatalogEntry {
            info_hash: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            name: "Matrix.1999.1080p.BluRay.x264-GRP".to_string(),
            total_bytes: 4_721_883_912,
            video_file_name: "Matrix.1999.1080p.mkv".to_string(),
            video_file_size: 4_721_883_900,
            video_file_index: 0,
            video_file_offset: 0,
            files_json: "[]".to_string(),
            estimated_peers: 42,
            source: "dht_bep51".to_string(),
            first_seen_ms: 1_000,
            last_seen_ms: 2_000,
        };
        let e2 = DhtTorrentCatalogEntry {
            info_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            name: "Inception.2010.2160p.WEB-DL.mkv".to_string(),
            total_bytes: 8_000_000_000,
            video_file_name: "Inception.2010.2160p.WEB-DL.mkv".to_string(),
            video_file_size: 8_000_000_000,
            video_file_index: 0,
            video_file_offset: 0,
            files_json: "[]".to_string(),
            estimated_peers: 15,
            source: "dht_passive".to_string(),
            first_seen_ms: 3_000,
            last_seen_ms: 4_000,
        };

        db.upsert_dht_catalog_entry(&e1).unwrap();
        db.upsert_dht_catalog_entry(&e2).unwrap();

        let exact = db
            .find_dht_catalog_by_exact_video_size(4_721_883_900, 10)
            .unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].info_hash, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let found = db
            .search_dht_catalog_by_tokens("matrix 1080p bluray", 10)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].video_file_name, "Matrix.1999.1080p.mkv");

        let pruned = db.prune_dht_catalog(1).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(db.get_hybrid_catalog_stats().unwrap().catalog_entries, 1);
    }

    #[test]
    fn multifile_pack_indexes_every_episode_by_exact_size_and_tokens() {
        let mut db = Database::open_in_memory().unwrap();
        let pack_hash = "dddddddddddddddddddddddddddddddddddddddd";
        let pack_name = "Naruto.Complete.001-220.1080p.BluRay.Dual";
        db.upsert_dht_catalog_entry(&DhtTorrentCatalogEntry {
            info_hash: pack_hash.to_string(),
            name: pack_name.to_string(),
            total_bytes: 900_000_000,
            video_file_name: "Naruto - 001.mkv".to_string(),
            video_file_size: 300_000_001,
            video_file_index: 0,
            video_file_offset: 0,
            files_json: "[]".to_string(),
            estimated_peers: 25,
            source: "dht_bep51".to_string(),
            first_seen_ms: 100,
            last_seen_ms: 200,
        })
        .unwrap();

        let episodes = vec![
            DhtTorrentFileEntry {
                info_hash: pack_hash.to_string(),
                file_index: 0,
                file_path: "Season 01/Naruto - S01E01.mkv".to_string(),
                file_size: 300_000_001,
                file_offset: 0,
            },
            DhtTorrentFileEntry {
                info_hash: pack_hash.to_string(),
                file_index: 1,
                file_path: "Season 01/Naruto - S01E02.mkv".to_string(),
                file_size: 284_555_123,
                file_offset: 300_000_001,
            },
            DhtTorrentFileEntry {
                info_hash: pack_hash.to_string(),
                file_index: 2,
                file_path: "Season 01/Naruto - S01E03.mkv".to_string(),
                file_size: 315_444_876,
                file_offset: 584_555_124,
            },
        ];
        db.upsert_dht_catalog_files(pack_hash, pack_name, &episodes)
            .unwrap();

        // El episodio 2 (no principal) debe encontrarse tanto por tamaño exacto como por tokens.
        let by_size = db
            .find_dht_catalog_by_exact_video_size(284_555_123, 10)
            .unwrap();
        assert_eq!(by_size.len(), 1);
        assert_eq!(by_size[0].video_file_index, 1);
        assert_eq!(by_size[0].video_file_offset, 300_000_001);
        assert_eq!(by_size[0].video_file_name, "Season 01/Naruto - S01E02.mkv");

        let by_tokens = db.search_dht_catalog_by_tokens("naruto s01e02", 10).unwrap();
        assert_eq!(by_tokens.len(), 1);
        assert_eq!(by_tokens[0].video_file_index, 1);
    }

    #[test]
    fn hybrid_bridge_status_invariants_preserved() {
        let mut db = Database::open_in_memory().unwrap();
        let bridge = HybridBridgeRecord {
            ed2k_hash: "0123456789abcdef0123456789abcdef".to_string(),
            bt_info_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            file_size: 1_500_000,
            bt_file_index: 1,
            bt_file_offset: 512,
            status: HybridBridgeStatus::Candidate,
            origin: "kad_note".to_string(),
            verified_chunks: 0,
            published_kad_at_ms: None,
            created_at_ms: 100,
            updated_at_ms: 100,
        };
        db.upsert_hybrid_bridge(&bridge).unwrap();

        // Promover a Verified y confirmar que un nuevo candidato no lo degrada.
        assert!(db
            .update_hybrid_bridge_status(
                &bridge.ed2k_hash,
                &bridge.bt_info_hash,
                HybridBridgeStatus::Verified,
                2,
                200,
            )
            .unwrap());
        db.upsert_hybrid_bridge(&bridge).unwrap();

        let list = db.list_hybrid_bridges_for_ed2k(&bridge.ed2k_hash).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].status, HybridBridgeStatus::Verified);
        assert_eq!(list[0].verified_chunks, 2);

        // Un puente rechazado tampoco debe resucitar a candidato.
        let bad = HybridBridgeRecord {
            bt_info_hash: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            status: HybridBridgeStatus::Rejected,
            ..bridge.clone()
        };
        db.upsert_hybrid_bridge(&bad).unwrap();
        let retry_bad = HybridBridgeRecord {
            status: HybridBridgeStatus::Candidate,
            ..bad.clone()
        };
        db.upsert_hybrid_bridge(&retry_bad).unwrap();

        let all = db.list_hybrid_bridges_for_ed2k(&bridge.ed2k_hash).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, HybridBridgeStatus::Verified);
        assert_eq!(all[1].status, HybridBridgeStatus::Rejected);

        let stats = db.get_hybrid_catalog_stats().unwrap();
        assert_eq!(stats.bridges_total, 2);
        assert_eq!(stats.bridges_verified, 1);
        assert_eq!(stats.bridges_rejected, 1);
    }
}
