// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Importación de `clients.met` — `CClientCreditsList::LoadList` de eMule
//! (`ClientCredits.cpp:156-233`).
//!
//! eMule indexa los créditos por el hash de usuario del cliente (`abyKey`);
//! nuestra tabla `peer_credits` indexa por `MD4(pubkey)` porque los créditos solo
//! son reclamables bajo identificación segura — la clave pública RSA `abySecureIdent`
//! almacenada por registro es precisamente lo que nos permite reclamarlos.
//! Los registros sin clave pública almacenada se notifican como omitidos: eMule
//! puede concederles créditos débiles (no verificados), nuestro modelo no.
//!
//! Fiel a eMule, las entradas con `tLastSeen < now - 150 days` se descartan en la
//! importación (tampoco llegarían nunca al enrutamiento en eMule).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use md4::{Digest, Md4};
use mule_core::model::PeerCredit;
use mule_core::persistence::sqlite::Database;
use mule_format::met::ClientsMetFile;
use serde::Serialize;

use crate::error::DaemonError;

use super::db_err;

/// Caducidad de créditos de eMule (`ClientCredits.cpp:207` — `DAY2S(150)`).
const CREDIT_EXPIRY_SECS: u64 = 150 * 86_400;

/// Resultado de `import_clients_met`.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientsMetImportReport {
    /// Registros de `clients.met` leídos.
    pub records: usize,
    /// Créditos importados en `peer_credits`.
    pub imported: usize,
    /// Entradas con más de 150 días de antigüedad — descartadas igual que en eMule.
    pub expired: usize,
    /// Entradas sin clave pública almacenada — no reclamables bajo nuestro
    /// modelo de créditos indexado por clave pública.
    pub skipped_no_pubkey: usize,
    /// `true` cuando el archivo utilizaba el formato heredado `0x11`.
    pub legacy_format: bool,
}

/// `CClientCreditsList::LoadList` — importa un `clients.met` de eMule en
/// `peer_credits` (inserción o actualización por `pubkey_hash`, igual que la sobreescritura de mapa en eMule).
pub fn import_clients_met(
    db: &mut Database,
    path: &Path,
) -> Result<ClientsMetImportReport, DaemonError> {
    let file = ClientsMetFile::open(path)
        .map_err(|e| DaemonError::Business(format!("clients.met: {e}")))?;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expired_before = now_secs.saturating_sub(CREDIT_EXPIRY_SECS) as u32;

    let mut report = ClientsMetImportReport {
        records: file.records.len(),
        legacy_format: file.legacy_format,
        ..Default::default()
    };
    for record in &file.records {
        if record.last_seen < expired_before {
            report.expired += 1;
            continue;
        }
        let Some(pubkey) = &record.pubkey else {
            report.skipped_no_pubkey += 1;
            continue;
        };
        let pubkey_hash = hex::encode(Md4::digest(pubkey));
        let seen_ms = i64::from(record.last_seen) * 1000;
        db.upsert_peer_credit(&PeerCredit {
            pubkey_hash,
            pubkey_hex: hex::encode(pubkey),
            uploaded_bytes: record.uploaded,
            downloaded_bytes: record.downloaded,
            // eMule solo almacena `tLastSeen` — first_seen desconocido.
            first_seen_unix_ms: seen_ms,
            last_seen_unix_ms: seen_ms,
        })
        .map_err(db_err("importing client credit"))?;
        report.imported += 1;
    }
    Ok(report)
}
