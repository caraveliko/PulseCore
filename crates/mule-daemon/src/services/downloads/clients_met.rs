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

/// eMule credit expiry (`ClientCredits.cpp:207` — `DAY2S(150)`).
const CREDIT_EXPIRY_SECS: u64 = 150 * 86_400;

/// Outcome of `import_clients_met`.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientsMetImportReport {
    /// `clients.met` records read.
    pub records: usize,
    /// Credits imported into `peer_credits`.
    pub imported: usize,
    /// Entries older than 150 days — dropped like eMule.
    pub expired: usize,
    /// Entries without a stored pubkey — unclaimable under our
    /// pubkey-keyed credit model.
    pub skipped_no_pubkey: usize,
    /// `true` when the file used the legacy `0x11` layout.
    pub legacy_format: bool,
}

/// `CClientCreditsList::LoadList` — imports an eMule `clients.met` into
/// `peer_credits` (upsert by `pubkey_hash`, like eMule's map overwrite).
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
            // eMule stores only `tLastSeen` — first_seen unknown.
            first_seen_unix_ms: seen_ms,
            last_seen_unix_ms: seen_ms,
        })
        .map_err(db_err("importing client credit"))?;
        report.imported += 1;
    }
    Ok(report)
}
