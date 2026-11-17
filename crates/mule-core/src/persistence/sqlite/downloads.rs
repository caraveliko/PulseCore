// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use rusqlite::{params, Connection};
use std::path::Path;
use uuid::Uuid;

use crate::error::CoreError;
use crate::model::{Download, DownloadStatus};
use crate::ranges::RangeSet;

pub fn insert_download(conn: &mut Connection, dl: &Download) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO downloads (id, display_name, target_path, temp_path, total_size, ed2k_hash, aich_root, status, created_at, updated_at, category_id, priority, max_sources)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            dl.id.to_string(),
            dl.display_name,
            dl.target_path.to_str().unwrap_or_default(),
            dl.temp_path.to_str().unwrap_or_default(),
            dl.total_size as i64,
            dl.ed2k_hash.0.as_slice(),
            dl.aich_root.as_ref().map(|a| a.0.as_slice()),
            dl.status.as_str(),
            dl.created_at,
            dl.updated_at,
            dl.category_id,
            dl.priority,
            dl.max_sources.map(|v| v as i64),
        ],
    )?;
    Ok(())
}

/// Persiste la prioridad por descarga ("low" | "normal" | "high").
pub fn update_priority(
    conn: &mut Connection,
    download_id: &Uuid,
    priority: &str,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE downloads SET priority = ?1 WHERE id = ?2",
        params![priority, download_id.to_string()],
    )?;
    Ok(())
}

/// Persiste el límite de fuentes por archivo (`FT_MAXSOURCES` de eMule). `None`
/// lo restablece al valor predeterminado global.
pub fn update_max_sources(
    conn: &mut Connection,
    download_id: &Uuid,
    max_sources: Option<u32>,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE downloads SET max_sources = ?1 WHERE id = ?2",
        params![max_sources.map(|v| v as i64), download_id.to_string()],
    )?;
    Ok(())
}

/// Persiste el hash raíz AICH de confianza para una descarga.
pub fn update_aich_root(
    conn: &mut Connection,
    download_id: &Uuid,
    aich_root: &[u8; 20],
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE downloads SET aich_root = ?1 WHERE id = ?2",
        params![aich_root.as_slice(), download_id.to_string()],
    )?;
    Ok(())
}

/// Reescritura completa de los rangos persistidos (DELETE + INSERT de todo el
/// RangeSet). Reservada para los puntos de compactación/desalojo: la ruta crítica de
/// recepción utiliza [`append_received_range`] (delta O(1) por bloque).
pub fn sync_received_ranges(
    conn: &mut Connection,
    download_id: &Uuid,
    ranges: &RangeSet,
) -> Result<(), CoreError> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM received_ranges WHERE download_id = ?1",
        params![download_id.to_string()],
    )?;

    {
        let mut stmt = tx.prepare(
            "INSERT INTO received_ranges (download_id, start_offset, end_offset) VALUES (?1, ?2, ?3)",
        )?;
        for r in ranges.ranges() {
            stmt.execute(params![
                download_id.to_string(),
                r.start as i64,
                r.end as i64
            ])?;
        }
    }

    tx.commit()?;
    Ok(())
}

/// Persiste un solo rango recién recibido (ruta crítica por bloque) sin
/// reescribir la tabla entera: upsert que amplía `end_offset` al máximo
/// cuando una fila ya comparte `start_offset`. Las filas resultantes
/// pueden solaparse — `load_received_ranges` las refusiona mediante
/// `RangeSet::insert`, y la reescritura completa ([`sync_received_ranges`],
/// cadencia `PART_PERSIST_INTERVAL` + desalojos) recompacta la tabla.
pub fn append_received_range(
    conn: &mut Connection,
    download_id: &Uuid,
    start: u64,
    end: u64,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO received_ranges (download_id, start_offset, end_offset)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(download_id, start_offset)
         DO UPDATE SET end_offset = MAX(received_ranges.end_offset, excluded.end_offset)",
        params![download_id.to_string(), start as i64, end as i64],
    )?;
    Ok(())
}

pub fn load_received_ranges(conn: &Connection, download_id: &Uuid) -> Result<RangeSet, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT start_offset, end_offset FROM received_ranges WHERE download_id = ?1 ORDER BY start_offset ASC",
    )?;

    let mut rows = stmt.query(params![download_id.to_string()])?;
    let mut set = RangeSet::new();

    while let Some(row) = rows.next()? {
        let s: i64 = row.get(0)?;
        let e: i64 = row.get(1)?;
        set.insert(s as u64..e as u64)?;
    }

    Ok(set)
}

pub fn mark_part_verified(
    conn: &mut Connection,
    download_id: &Uuid,
    part_index: u32,
) -> Result<(), CoreError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT OR REPLACE INTO verified_parts (download_id, part_index, verified_at) VALUES (?1, ?2, ?3)",
        params![download_id.to_string(), part_index as i64, now],
    )?;
    Ok(())
}

pub fn unmark_part_verified(
    conn: &mut Connection,
    download_id: &Uuid,
    part_index: u32,
) -> Result<(), CoreError> {
    conn.execute(
        "DELETE FROM verified_parts WHERE download_id = ?1 AND part_index = ?2",
        params![download_id.to_string(), part_index as i64],
    )?;
    Ok(())
}

pub fn get_verified_parts(conn: &Connection, download_id: &Uuid) -> Result<Vec<u32>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT part_index FROM verified_parts WHERE download_id = ?1 ORDER BY part_index ASC",
    )?;
    let mut rows = stmt.query(params![download_id.to_string()])?;
    let mut parts = Vec::new();
    while let Some(row) = rows.next()? {
        let p: i64 = row.get(0)?;
        parts.push(p as u32);
    }
    Ok(parts)
}

/// Almacena el conjunto completo de hashes MD4 por parte para un archivo (indexado por el hash eD2K hexadecimal en minúsculas).
pub fn store_part_hashes(
    conn: &mut Connection,
    file_hash: &str,
    part_hashes: &[[u8; 16]],
) -> Result<(), CoreError> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM file_part_hashes WHERE file_hash = ?1",
        params![file_hash],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO file_part_hashes (file_hash, part_index, hash) VALUES (?1, ?2, ?3)",
        )?;
        for (i, h) in part_hashes.iter().enumerate() {
            stmt.execute(params![file_hash, i as i64, h.as_slice()])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Carga el conjunto de hashes MD4 por parte almacenado para un archivo, ordenado por índice de parte.
pub fn get_part_hashes(conn: &Connection, file_hash: &str) -> Result<Vec<[u8; 16]>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT part_index, hash FROM file_part_hashes WHERE file_hash = ?1 ORDER BY part_index ASC",
    )?;
    let mut rows = stmt.query(params![file_hash])?;
    let mut hashes = Vec::new();
    while let Some(row) = rows.next()? {
        let blob: Vec<u8> = row.get(1)?;
        if blob.len() != 16 {
            return Err(CoreError::Database("Invalid part hash length in DB".into()));
        }
        let mut h = [0u8; 16];
        h.copy_from_slice(&blob);
        hashes.push(h);
    }
    Ok(hashes)
}

/// Elimina el conjunto de hashes por parte almacenado para un archivo.
pub fn delete_part_hashes(conn: &mut Connection, file_hash: &str) -> Result<(), CoreError> {
    conn.execute(
        "DELETE FROM file_part_hashes WHERE file_hash = ?1",
        params![file_hash],
    )?;
    Ok(())
}

pub fn update_status(
    conn: &mut Connection,
    download_id: &Uuid,
    status: DownloadStatus,
) -> Result<(), CoreError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "UPDATE downloads SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, download_id.to_string()],
    )?;
    Ok(())
}

pub fn get_all_downloads(conn: &Connection) -> Result<Vec<Download>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, target_path, temp_path, total_size, ed2k_hash, aich_root, status, created_at, updated_at, category_id, priority, max_sources FROM downloads ORDER BY created_at ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut downloads = Vec::new();
    while let Some(row) = rows.next()? {
        let id_str: String = row.get(0)?;
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| CoreError::Database(format!("Invalid UUID in database: {e}")))?;
        let display_name: String = row.get(1)?;
        let target_path: String = row.get(2)?;
        let temp_path: String = row.get(3)?;
        let total_size: i64 = row.get(4)?;
        let ed2k_blob: Vec<u8> = row.get(5)?;
        let aich_blob: Option<Vec<u8>> = row.get(6)?;
        let status_str: String = row.get(7)?;
        let created_at: i64 = row.get(8)?;
        let updated_at: i64 = row.get(9)?;
        let category_id: Option<String> = row.get(10)?;
        let priority: String = row.get(11).unwrap_or_else(|_| "normal".to_string());
        let max_sources: Option<i64> = row.get(12).unwrap_or(None);

        if ed2k_blob.len() != 16 {
            return Err(CoreError::Database("Invalid ed2k_hash length in DB".into()));
        }
        let mut ed2k_hash_bytes = [0u8; 16];
        ed2k_hash_bytes.copy_from_slice(&ed2k_blob);

        let aich_root = if let Some(b) = aich_blob {
            if b.len() == 20 {
                let mut aich_bytes = [0u8; 20];
                aich_bytes.copy_from_slice(&b);
                Some(mule_format::AichHash(aich_bytes))
            } else {
                None
            }
        } else {
            None
        };

        let status = status_str.parse().unwrap_or(DownloadStatus::Queued);

        downloads.push(Download {
            id,
            display_name,
            target_path: std::path::PathBuf::from(target_path),
            temp_path: std::path::PathBuf::from(temp_path),
            total_size: total_size as u64,
            ed2k_hash: mule_format::Ed2kHash(ed2k_hash_bytes),
            aich_root,
            status,
            created_at,
            updated_at,
            category_id,
            priority,
            max_sources: max_sources.and_then(|v| u32::try_from(v).ok()),
        });
    }
    Ok(downloads)
}

pub fn get_download(conn: &Connection, id: &Uuid) -> Result<Option<Download>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, target_path, temp_path, total_size, ed2k_hash, aich_root, status, created_at, updated_at, category_id, priority, max_sources FROM downloads WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id.to_string()])?;
    if let Some(row) = rows.next()? {
        let id_str: String = row.get(0)?;
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| CoreError::Database(format!("Invalid UUID in database: {e}")))?;
        let display_name: String = row.get(1)?;
        let target_path: String = row.get(2)?;
        let temp_path: String = row.get(3)?;
        let total_size: i64 = row.get(4)?;
        let ed2k_blob: Vec<u8> = row.get(5)?;
        let aich_blob: Option<Vec<u8>> = row.get(6)?;
        let status_str: String = row.get(7)?;
        let created_at: i64 = row.get(8)?;
        let updated_at: i64 = row.get(9)?;
        let category_id: Option<String> = row.get(10)?;
        let priority: String = row.get(11).unwrap_or_else(|_| "normal".to_string());
        let max_sources: Option<i64> = row.get(12).unwrap_or(None);

        if ed2k_blob.len() != 16 {
            return Err(CoreError::Database("Invalid ed2k_hash length in DB".into()));
        }
        let mut ed2k_hash_bytes = [0u8; 16];
        ed2k_hash_bytes.copy_from_slice(&ed2k_blob);

        let aich_root = if let Some(b) = aich_blob {
            if b.len() == 20 {
                let mut aich_bytes = [0u8; 20];
                aich_bytes.copy_from_slice(&b);
                Some(mule_format::AichHash(aich_bytes))
            } else {
                None
            }
        } else {
            None
        };

        let status = status_str.parse().unwrap_or(DownloadStatus::Queued);

        Ok(Some(Download {
            id,
            display_name,
            target_path: std::path::PathBuf::from(target_path),
            temp_path: std::path::PathBuf::from(temp_path),
            total_size: total_size as u64,
            ed2k_hash: mule_format::Ed2kHash(ed2k_hash_bytes),
            aich_root,
            status,
            created_at,
            updated_at,
            category_id,
            priority,
            max_sources: max_sources.and_then(|v| u32::try_from(v).ok()),
        }))
    } else {
        Ok(None)
    }
}

pub fn delete_download(conn: &mut Connection, download_id: &Uuid) -> Result<bool, CoreError> {
    let tx = conn.transaction()?;
    let id_str = download_id.to_string();
    tx.execute(
        "DELETE FROM received_ranges WHERE download_id = ?1",
        params![&id_str],
    )?;
    tx.execute(
        "DELETE FROM verified_parts WHERE download_id = ?1",
        params![&id_str],
    )?;
    let rows_deleted = tx.execute("DELETE FROM downloads WHERE id = ?1", params![&id_str])?;
    tx.commit()?;
    Ok(rows_deleted > 0)
}

pub fn update_target_path(
    conn: &mut Connection,
    download_id: &Uuid,
    target_path: &Path,
) -> Result<(), CoreError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "UPDATE downloads SET target_path = ?1, updated_at = ?2 WHERE id = ?3",
        params![
            target_path.to_str().unwrap_or_default(),
            now,
            download_id.to_string()
        ],
    )?;
    Ok(())
}

/// Cambia el nombre de una descarga: nombre para mostrar + ruta de destino de finalización derivada
/// de este (el archivo temporal `.part` mantiene su nombre basado en UUID).
pub fn update_download_name_and_target(
    conn: &mut Connection,
    download_id: &Uuid,
    display_name: &str,
    target_path: &Path,
) -> Result<(), CoreError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "UPDATE downloads SET display_name = ?1, target_path = ?2, updated_at = ?3 WHERE id = ?4",
        params![
            display_name,
            target_path.to_str().unwrap_or_default(),
            now,
            download_id.to_string()
        ],
    )?;
    Ok(())
}
