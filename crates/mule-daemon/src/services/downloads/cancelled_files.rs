// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Seguimiento de archivos cancelados de eMule — `CKnownFileList::AddCancelledFileID`,
//! `IsCancelledFileByID` e importación de `cancelled.met`
//! (`KnownFileList.cpp:384-411, 167-194`).
//!
//! El conjunto almacena únicamente hashes enmascarados `MD5(seed || file_hash)` —
//! al igual que eMule, nunca persistimos el hash real del archivo. Los archivos
//! `0x0F` importados conservan su propia semilla; las entradas heredadas `0x0E`
//! (hashes reales) se enmascaran bajo nuestra semilla local al importar,
//! exactamente lo que hace eMule al cargar.
//!
//! Tanto el registro como la búsqueda están controlados por `IsRememberingCancelledFiles`
//! → nuestro ajuste `remember_cancelled` (la misma puerta que usan las categorías
//! de historial). El conjunto marca los resultados de búsqueda como «cancelados»;
//! no bloquea de forma estricta las descargas — igual que eMule (coloreado de
//! `m_MyListCtrlItem`).

use std::path::Path;

use mule_core::persistence::sqlite::Database;
use mule_format::met::{cancelled_mask, CancelledEntry, CancelledMetFile};
use serde::Serialize;

use crate::error::DaemonError;

use super::db_err;
use super::history_categories::setting_enabled;

/// Ajuste `IsRememberingCancelledFiles` de eMule → `remember_cancelled`.
const REMEMBER_CANCELLED_KEY: &str = "remember_cancelled";

/// Resultado de `import_cancelled_met`.
#[derive(Debug, Default, Serialize)]
pub struct CancelledImportReport {
    /// Registros de `cancelled.met` leídos (tamaño de `m_mapCancelledFiles` tras la carga).
    pub records: usize,
    /// Entradas enmascaradas almacenadas bajo la propia semilla del archivo (entradas `0x0F`).
    pub masked_imported: usize,
    /// Hashes reales heredados enmascarados bajo nuestra semilla (entradas `0x0E`).
    pub legacy_masked: usize,
    /// `true` cuando el archivo utilizaba el formato obsoleto `0x0E`.
    pub legacy_format: bool,
    /// Total de hashes enmascarados almacenados (tamaño del conjunto tras la importación).
    pub total: usize,
}

/// `AddCancelledFileID` — registra `hash` como cancelado bajo nuestra semilla.
/// Operación no operativa cuando `remember_cancelled` está desactivado (puerta de eMule).
pub fn record_cancelled_file(db: &mut Database, hash: &[u8; 16]) -> Result<(), DaemonError> {
    if !setting_enabled(db, REMEMBER_CANCELLED_KEY) {
        return Ok(());
    }
    let seed = db
        .get_or_create_cancelled_seed()
        .map_err(db_err("getting cancelled seed"))?;
    let masked = hex::encode(cancelled_mask(seed, hash));
    db.add_cancelled_hash(seed, &masked)
        .map_err(db_err("recording cancelled hash"))
}

/// `IsCancelledFileByID` — `true` cuando `hash` está en el conjunto de cancelados.
/// Comprueba cada semilla conocida (los archivos importados conservan la suya). Siempre `false`
/// cuando `remember_cancelled` está desactivado, igual que en eMule.
pub fn is_cancelled_file(db: &Database, hash: &[u8; 16]) -> Result<bool, DaemonError> {
    if !setting_enabled(db, REMEMBER_CANCELLED_KEY) {
        return Ok(false);
    }
    for seed in db.cancelled_seeds().map_err(db_err("listing seeds"))? {
        let masked = hex::encode(cancelled_mask(seed, hash));
        if db
            .is_cancelled_masked(seed, &masked)
            .map_err(db_err("checking cancelled hash"))?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `CKnownFileList::LoadCancelled` — importa un archivo `cancelled.met`.
/// Las entradas `0x0F` se almacenan bajo la semilla del archivo; los hashes reales `0x0E` se
/// enmascaran bajo nuestra semilla (conversión en tiempo de carga de eMule).
pub fn import_cancelled_met(
    db: &mut Database,
    path: &Path,
) -> Result<CancelledImportReport, DaemonError> {
    let file = CancelledMetFile::open(path)
        .map_err(|e| DaemonError::Business(format!("cancelled.met: {e}")))?;
    let mut report = CancelledImportReport {
        records: file.entries.len(),
        legacy_format: file.legacy_format,
        ..Default::default()
    };
    // `0` es imposible en un archivo válido — solo se alcanza en el diseño
    // heredado donde no puede existir ninguna entrada `Masked` de todos modos.
    let file_seed = file.seed.unwrap_or(0);
    for entry in &file.entries {
        match entry {
            CancelledEntry::Masked(masked) => {
                db.add_cancelled_hash(file_seed, &hex::encode(masked))
                    .map_err(db_err("importing cancelled mask"))?;
                report.masked_imported += 1;
            }
            CancelledEntry::Real(hash) => {
                let seed = db
                    .get_or_create_cancelled_seed()
                    .map_err(db_err("getting cancelled seed"))?;
                let masked = hex::encode(cancelled_mask(seed, hash));
                db.add_cancelled_hash(seed, &masked)
                    .map_err(db_err("importing cancelled hash"))?;
                report.legacy_masked += 1;
            }
        }
    }
    report.total = db.cancelled_count().map_err(db_err("counting cancelled"))?;
    Ok(report)
}

/// Analiza un hash eD2K en hexadecimal en `[u8; 16]` (`None` ante entrada malformada).
pub fn parse_hash16(ed2k_hash: &str) -> Option<[u8; 16]> {
    let bytes = hex::decode(ed2k_hash).ok()?;
    <[u8; 16]>::try_from(bytes.as_slice()).ok()
}
