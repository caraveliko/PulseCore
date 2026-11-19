// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Persistencia SQLite de hashes de partes AICH recibidos mediante `OP_HASHSETANSWER2`.

use rusqlite::{params, Connection};

use crate::error::CoreError;

/// Longitud exacta en bytes de un hash de parte AICH (SHA-1).
pub const AICH_PART_HASH_BYTES: usize = 20;

/// Almacena atómicamente el conjunto ordenado de hashes de partes AICH para `file_hash`.
pub fn store_aich_part_hashes(
    conn: &mut Connection,
    file_hash: &str,
    part_hashes: &[[u8; AICH_PART_HASH_BYTES]],
) -> Result<(), CoreError> {
    let clean = file_hash.trim().to_ascii_lowercase();
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM file_aich_part_hashes WHERE file_hash = ?1",
        params![clean],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO file_aich_part_hashes (file_hash, part_index, aich_hash)
             VALUES (?1, ?2, ?3)",
        )?;
        for (idx, h) in part_hashes.iter().enumerate() {
            stmt.execute(params![clean, idx as i64, h.as_slice()])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Carga los hashes de partes AICH ordenados por índice para `file_hash`.
pub fn load_aich_part_hashes(
    conn: &Connection,
    file_hash: &str,
) -> Result<Vec<[u8; AICH_PART_HASH_BYTES]>, CoreError> {
    let clean = file_hash.trim().to_ascii_lowercase();
    let mut stmt = conn.prepare(
        "SELECT aich_hash FROM file_aich_part_hashes WHERE file_hash = ?1 ORDER BY part_index ASC",
    )?;
    let rows = stmt.query_map(params![clean], |row| {
        let blob: Vec<u8> = row.get(0)?;
        Ok(blob)
    })?;
    let mut out = Vec::new();
    for r in rows {
        let blob = r?;
        if let Ok(arr) = <[u8; AICH_PART_HASH_BYTES]>::try_from(blob.as_slice()) {
            out.push(arr);
        }
    }
    Ok(out)
}

/// Elimina los hashes de partes AICH persistidos para `file_hash`.
pub fn delete_aich_part_hashes(conn: &mut Connection, file_hash: &str) -> Result<(), CoreError> {
    let clean = file_hash.trim().to_ascii_lowercase();
    conn.execute(
        "DELETE FROM file_aich_part_hashes WHERE file_hash = ?1",
        params![clean],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::persistence::sqlite::Database;

    #[test]
    fn aich_part_hashes_roundtrip_and_delete() {
        let mut db = Database::open_in_memory().unwrap();
        let fh = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hashes = vec![[0x11u8; 20], [0x22u8; 20], [0x33u8; 20]];
        db.store_aich_part_hashes(fh, &hashes).unwrap();
        assert_eq!(db.load_aich_part_hashes(fh).unwrap(), hashes);

        db.delete_aich_part_hashes(fh).unwrap();
        assert!(db.load_aich_part_hashes(fh).unwrap().is_empty());
    }
}
