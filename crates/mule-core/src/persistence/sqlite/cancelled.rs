// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Persistencia de `cancelled.met` de eMule — `CKnownFileList::AddCancelledFileID`/
//! `IsCancelledFileByID` (`KnownFileList.cpp:384-411`).
//!
//! eMule nunca almacena el propio hash del fichero, sino únicamente
//! `MD5(seed || hash)` — la lista de cancelados no puede filtrar lo que se
//! descargó y luego se canceló. Como cada máscara está vinculada a su
//! semilla, cada fila conserva la semilla con la que se generó: nuestros
//! propios registros usan el ajuste local `cancelled_seed`, y los ficheros
//! `cancelled.met` importados conservan la semilla escrita en su cabecera.

use rusqlite::{params, Connection};

use crate::error::CoreError;

/// Clave de configuración que almacena nuestro `m_dwCancelledFilesSeed` (u32 decimal,
/// `1..=0xFFFFFFFE` — generado de forma diferida igual que en eMule).
pub const CANCELLED_SEED_KEY: &str = "cancelled_seed";

/// Registra un hash enmascarado bajo `seed` (`INSERT OR IGNORE` — el conjunto es
/// idempotente como `m_mapCancelledFiles[key] = 1`).
pub fn add_cancelled_hash(
    conn: &mut Connection,
    seed: u32,
    masked_hash_hex: &str,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO cancelled_files (masked_hash, seed) VALUES (?1, ?2)",
        params![masked_hash_hex.to_lowercase(), seed as i64],
    )?;
    Ok(())
}

/// Devuelve `true` cuando `masked_hash_hex` existe bajo `seed`.
pub fn is_cancelled_masked(
    conn: &Connection,
    seed: u32,
    masked_hash_hex: &str,
) -> Result<bool, CoreError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM cancelled_files WHERE masked_hash = ?1 AND seed = ?2)",
        params![masked_hash_hex.to_lowercase(), seed as i64],
        |r| r.get(0),
    )?;
    Ok(exists)
}

/// Todas las semillas distintas presentes — `is_cancelled_file` debe probar cada semilla
/// (los archivos importados conservan su propio `m_dwCancelledFilesSeed`).
pub fn distinct_seeds(conn: &Connection) -> Result<Vec<u32>, CoreError> {
    let mut stmt = conn.prepare("SELECT DISTINCT seed FROM cancelled_files")?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row? as u32);
    }
    Ok(out)
}

/// Número de hashes enmascarados almacenados (inspección / informes de importación).
pub fn cancelled_count(conn: &Connection) -> Result<usize, CoreError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM cancelled_files", [], |r| r.get(0))?;
    Ok(n as usize)
}
