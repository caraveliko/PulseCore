// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Caché persistente de claves públicas SUI verificadas por `user_hash` de par.
//!
//! Permite emitir `OP_SECIDENTSTATE` con `state = 1` (`IS_SIGNATURENEEDED`) en
//! reconexiones posteriores sin solicitar de nuevo `OP_PUBLICKEY`.

use rusqlite::{params, Connection};

use crate::error::CoreError;

/// Entrada almacenada en la caché SUI de claves públicas verificadas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedSuiPubkey {
    /// Hash de usuario eD2K en hexadecimal minúsculas (32 caracteres).
    pub user_hash: String,
    /// Clave pública RSA DER PKCS#1 recibida y verificada.
    pub pubkey_der: Vec<u8>,
    /// Hash MD4 de `pubkey_der` en hexadecimal minúsculas.
    pub pubkey_hash: String,
    /// Marca temporal de la última verificación satisfactoria (unix ms).
    pub verified_at_unix_ms: i64,
    /// Última dirección IP observada para el par (opcional).
    pub last_ip: Option<String>,
}

/// Inserta o actualiza la clave pública SUI verificada para `user_hash`.
pub fn upsert_verified_peer_pubkey(
    conn: &mut Connection,
    user_hash: &str,
    pubkey_der: &[u8],
    pubkey_hash: &str,
    verified_at_unix_ms: i64,
    last_ip: Option<&str>,
) -> Result<(), CoreError> {
    let clean_uh = user_hash.trim().to_ascii_lowercase();
    let clean_pk_hash = pubkey_hash.trim().to_ascii_lowercase();
    conn.execute(
        "INSERT INTO sui_peer_pubkeys (user_hash, pubkey_der, pubkey_hash, verified_at_unix_ms, last_ip)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(user_hash) DO UPDATE SET
            pubkey_der = excluded.pubkey_der,
            pubkey_hash = excluded.pubkey_hash,
            verified_at_unix_ms = excluded.verified_at_unix_ms,
            last_ip = COALESCE(excluded.last_ip, sui_peer_pubkeys.last_ip)",
        params![
            clean_uh,
            pubkey_der,
            clean_pk_hash,
            verified_at_unix_ms,
            last_ip,
        ],
    )?;
    Ok(())
}

/// Recupera la clave pública DER cacheada para `user_hash`, si existe.
pub fn get_verified_peer_pubkey(
    conn: &Connection,
    user_hash: &str,
) -> Result<Option<Vec<u8>>, CoreError> {
    let clean_uh = user_hash.trim().to_ascii_lowercase();
    let mut stmt = conn.prepare(
        "SELECT pubkey_der FROM sui_peer_pubkeys WHERE user_hash = ?1",
    )?;
    let mut rows = stmt.query(params![clean_uh])?;
    if let Some(row) = rows.next()? {
        let der: Vec<u8> = row.get(0)?;
        Ok(Some(der))
    } else {
        Ok(None)
    }
}

/// Purga entradas de caché SUI cuya última verificación sea anterior a `cutoff_unix_ms`.
pub fn prune_stale_pubkeys(conn: &mut Connection, cutoff_unix_ms: i64) -> Result<usize, CoreError> {
    let deleted = conn.execute(
        "DELETE FROM sui_peer_pubkeys WHERE verified_at_unix_ms < ?1",
        params![cutoff_unix_ms],
    )?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use crate::persistence::sqlite::Database;

    #[test]
    fn sui_pubkey_cache_roundtrip_and_prune() {
        let mut db = Database::open_in_memory().unwrap();
        let uh = "0123456789abcdef0123456789abcdef";
        let der = vec![0x30u8, 0x3A, 0x02, 0x01];
        db.upsert_verified_peer_pubkey(uh, &der, "deadbeef", 1_000, Some("198.51.100.1"))
            .unwrap();

        assert_eq!(
            db.get_verified_peer_pubkey(&uh.to_uppercase()).unwrap(),
            Some(der.clone())
        );

        let pruned_none = db.prune_stale_pubkeys(500).unwrap();
        assert_eq!(pruned_none, 0);
        let pruned_one = db.prune_stale_pubkeys(2_000).unwrap();
        assert_eq!(pruned_one, 1);
        assert_eq!(db.get_verified_peer_pubkey(uh).unwrap(), None);
    }
}
