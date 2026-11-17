// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use rusqlite::{params, Connection};

use crate::error::CoreError;
use crate::model::SharedFile;

fn shared_file_from_row(row: &rusqlite::Row) -> rusqlite::Result<SharedFile> {
    let size_i64: i64 = row.get(2)?;
    let enabled_int: i32 = row.get(5)?;
    let aich_blob: Option<Vec<u8>> = row.get(6)?;
    let aich_root = aich_blob.and_then(|b| {
        if b.len() == 20 {
            let mut bytes = [0u8; 20];
            bytes.copy_from_slice(&b);
            Some(bytes)
        } else {
            None
        }
    });
    Ok(SharedFile {
        file_hash: row.get(0)?,
        file_name: row.get(1)?,
        file_size: size_i64.max(0) as u64,
        file_path: row.get(3)?,
        aich_root,
        shared_at_unix_ms: row.get(4)?,
        enabled: enabled_int != 0,
        up_priority: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "normal".into()),
        is_partial: false,
        completed_parts: Vec::new(),
        uploaded_bytes: 0,
        upload_rate_bps: 0,
        category_id: row.get::<_, Option<String>>(8)?,
    })
}

pub fn upsert_shared_file(conn: &mut Connection, file: &SharedFile) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO shared_files (file_hash, file_name, file_size, file_path, shared_at_unix_ms, enabled, aich_root, up_priority, category_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(file_hash) DO UPDATE SET
            file_name = excluded.file_name,
            file_size = excluded.file_size,
            file_path = excluded.file_path,
            enabled = excluded.enabled,
            up_priority = excluded.up_priority,
            aich_root = COALESCE(excluded.aich_root, shared_files.aich_root),
            category_id = COALESCE(excluded.category_id, shared_files.category_id)",
        params![
            file.file_hash,
            file.file_name,
            file.file_size as i64,
            file.file_path,
            file.shared_at_unix_ms,
            if file.enabled { 1 } else { 0 },
            file.aich_root.map(|a| a.to_vec()),
            file.up_priority,
            file.category_id,
        ],
    )?;
    Ok(())
}

/// Inserción o actualización por lotes (batch) de archivos compartidos en una sola transacción.
pub fn upsert_shared_files_batch(
    conn: &mut Connection,
    files: &[SharedFile],
) -> Result<usize, CoreError> {
    if files.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut count = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO shared_files (file_hash, file_name, file_size, file_path, shared_at_unix_ms, enabled, aich_root, up_priority, category_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(file_hash) DO UPDATE SET
                file_name = excluded.file_name,
                file_size = excluded.file_size,
                file_path = excluded.file_path,
                enabled = excluded.enabled,
                up_priority = excluded.up_priority,
                aich_root = COALESCE(excluded.aich_root, shared_files.aich_root),
                category_id = COALESCE(excluded.category_id, shared_files.category_id)",
        )?;

        for file in files {
            stmt.execute(params![
                file.file_hash,
                file.file_name,
                file.file_size as i64,
                file.file_path,
                file.shared_at_unix_ms,
                if file.enabled { 1 } else { 0 },
                file.aich_root.map(|a| a.to_vec()),
                file.up_priority,
                file.category_id,
            ])?;
            count += 1;
        }
    }
    tx.commit()?;
    Ok(count)
}

/// Almacena en caché el hash raíz AICH calculado de un archivo compartido.
pub fn set_shared_aich_root(
    conn: &mut Connection,
    file_hash: &str,
    aich_root: &[u8; 20],
) -> Result<bool, CoreError> {
    let rows = conn.execute(
        "UPDATE shared_files SET aich_root = ?1 WHERE file_hash = ?2",
        params![aich_root.as_slice(), file_hash],
    )?;
    Ok(rows > 0)
}

pub fn list_shared_files(conn: &Connection) -> Result<Vec<SharedFile>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT file_hash, file_name, file_size, file_path, shared_at_unix_ms, enabled, aich_root, up_priority, category_id
         FROM shared_files
         ORDER BY shared_at_unix_ms DESC",
    )?;
    let rows = stmt.query_map([], shared_file_from_row)?;

    let mut list = Vec::new();
    for r in rows {
        list.push(r?);
    }
    Ok(list)
}

pub fn get_shared_file(
    conn: &Connection,
    file_hash: &str,
) -> Result<Option<SharedFile>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT file_hash, file_name, file_size, file_path, shared_at_unix_ms, enabled, aich_root, up_priority, category_id
         FROM shared_files
         WHERE file_hash = ?1",
    )?;
    let mut rows = stmt.query(params![file_hash])?;
    if let Some(row) = rows.next()? {
        Ok(Some(shared_file_from_row(row)?))
    } else {
        Ok(None)
    }
}

pub fn delete_shared_file(conn: &mut Connection, file_hash: &str) -> Result<bool, CoreError> {
    let rows = conn.execute(
        "DELETE FROM shared_files WHERE file_hash = ?1",
        params![file_hash],
    )?;
    Ok(rows > 0)
}

/// Actualiza la prioridad de subida de eMule (`PR_*`) de un archivo compartido:
/// "verylow" | "low" | "normal" | "high" | "veryhigh" (= powershare).
pub fn set_shared_up_priority(
    conn: &mut Connection,
    file_hash: &str,
    up_priority: &str,
) -> Result<bool, CoreError> {
    let rows = conn.execute(
        "UPDATE shared_files SET up_priority = ?1 WHERE file_hash = ?2",
        params![up_priority, file_hash],
    )?;
    Ok(rows > 0)
}

pub fn set_shared_file_enabled(
    conn: &mut Connection,
    file_hash: &str,
    enabled: bool,
) -> Result<bool, CoreError> {
    let rows = conn.execute(
        "UPDATE shared_files SET enabled = ?1 WHERE file_hash = ?2",
        params![if enabled { 1 } else { 0 }, file_hash],
    )?;
    Ok(rows > 0)
}

/// Asocia (o desvincula, `None`) una categoría a un recurso compartido persistido.
/// UPDATE directo — a diferencia del upsert COALESCE, permite
/// desasignar explícitamente (`category_id` → NULL).
pub fn set_shared_file_category(
    conn: &mut Connection,
    file_hash: &str,
    category_id: Option<&str>,
) -> Result<bool, CoreError> {
    let rows = conn.execute(
        "UPDATE shared_files SET category_id = ?1 WHERE file_hash = ?2",
        params![category_id, file_hash],
    )?;
    Ok(rows > 0)
}

/// Añade `delta` bytes al total subido persistido de un archivo compartido
/// (`share_upload_stats` — tabla separada ya que los recursos compartidos parciales
/// `CPartFile` no tienen fila en `shared_files`).
pub fn add_share_uploaded_bytes(
    conn: &mut Connection,
    file_hash: &str,
    delta: u64,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO share_upload_stats (file_hash, uploaded_bytes) VALUES (?1, ?2)
         ON CONFLICT(file_hash) DO UPDATE SET
            uploaded_bytes = uploaded_bytes + excluded.uploaded_bytes",
        params![file_hash.to_lowercase(), delta as i64],
    )?;
    Ok(())
}

/// Totales subidos persistidos por hash de archivo (claves en minúsculas).
pub fn list_share_uploaded_bytes(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, u64>, CoreError> {
    let mut stmt =
        conn.prepare("SELECT file_hash, uploaded_bytes FROM share_upload_stats")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut map = std::collections::HashMap::new();
    for r in rows {
        let (hash, bytes) = r?;
        map.insert(hash.to_lowercase(), bytes.max(0) as u64);
    }
    Ok(map)
}
